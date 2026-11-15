#![no_std]

//! Preemptive round-robin kernel-thread runtime driven by the PIT timer (IRQ0).
//!
//! Scope and honesty contract:
//!
//! - This schedules ring-0 kernel threads that share the kernel address
//!   space. It is **not** process scheduling: there are no per-thread address
//!   spaces and no ring-3 execution yet. Those remain separate readiness
//!   gates (see `PRODUCT_READINESS.md`, Gate B).
//! - The switch is performed inside the IRQ0 handler by rewriting the live
//!   [`InterruptFrame`]: the outgoing thread's frame is stored in its slot
//!   and the incoming thread's saved frame is written over the live one, so
//!   the common ISR epilogue (register pops + `iretq`) resumes the incoming
//!   thread. No separate assembly switch path is needed on this architecture.
//! - Single-CPU only. There is no per-CPU state and no locking beyond
//!   interrupt masking; SMP requires per-CPU schedulers and is future work.
//!
//! Concurrency model: the scheduler state is mutated only (a) inside the
//! IRQ0 handler, which always runs with IF=0, or (b) under
//! [`interrupt_free`], which masks interrupts around the mutation. A thread
//! holding the critical section therefore cannot be interrupted by the timer,
//! and the handler never blocks.

use core::cell::UnsafeCell;

#[cfg(all(target_arch = "x86_64", target_os = "none"))]
use crate::arch::x86_64::boot::{disable_interrupts, enable_interrupts};
#[cfg(target_arch = "x86_64")]
use crate::arch::x86_64::gdt::{KERNEL_CODE_SELECTOR, KERNEL_DATA_SELECTOR};
#[cfg(target_arch = "x86_64")]
use crate::arch::x86_64::isr::InterruptFrame;
use crate::memory::address::is_canonical_x86_64_48;

/// Maximum number of kernel threads, including the boot/idle context.
pub const MAX_THREADS: usize = 8;

/// Per-thread kernel stack size (bytes).
pub const THREAD_STACK_SIZE: usize = 16 * 1024;

/// Default time slice in PIT ticks. At the programmed 1000 Hz PIT rate this
/// is a 50 ms quantum: long enough to keep switch overhead negligible, short
/// enough to demonstrate preemption in QEMU smoke runs.
pub const DEFAULT_QUANTUM_TICKS: u32 = 50;

/// Slot 0 is reserved for the boot/idle context that ran `kernel_entry`.
pub const BOOT_THREAD_INDEX: usize = 0;

const RFLAGS_IF: u64 = 1 << 9;
/// IF=1, bit 1 always set, IOPL=0, DF=0.
const THREAD_INITIAL_RFLAGS: u64 = 0x202;

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ThreadState {
    Empty = 0,
    Runnable = 1,
    Exited = 2,
}

#[repr(align(16))]
#[derive(Clone, Copy)]
struct ThreadStack([u8; THREAD_STACK_SIZE]);

#[cfg(target_arch = "x86_64")]
#[derive(Clone, Copy)]
struct KernelThread {
    state: ThreadState,
    frame: InterruptFrame,
    dispatch_count: u64,
    stack: ThreadStack,
}

#[cfg(target_arch = "x86_64")]
impl KernelThread {
    const fn empty() -> Self {
        Self {
            state: ThreadState::Empty,
            frame: InterruptFrame::empty(),
            dispatch_count: 0,
            stack: ThreadStack([0; THREAD_STACK_SIZE]),
        }
    }
}

#[cfg(target_arch = "x86_64")]
struct Preemptive {
    threads: [KernelThread; MAX_THREADS],
    current: usize,
    quantum: u32,
    ticks_in_quantum: u32,
    ticks: u64,
    switches: u64,
    started: bool,
}

#[cfg(target_arch = "x86_64")]
impl Preemptive {
    const fn new() -> Self {
        Self {
            threads: [KernelThread::empty(); MAX_THREADS],
            current: BOOT_THREAD_INDEX,
            quantum: DEFAULT_QUANTUM_TICKS,
            ticks_in_quantum: 0,
            ticks: 0,
            switches: 0,
            started: false,
        }
    }

    fn next_runnable(&self) -> Option<usize> {
        let mut offset = 1;
        while offset <= MAX_THREADS {
            let index = (self.current + offset) % MAX_THREADS;
            if self.threads[index].state == ThreadState::Runnable {
                return Some(index);
            }
            offset += 1;
        }
        None
    }

    fn tick(&mut self, frame: &mut InterruptFrame) {
        self.ticks = self.ticks.saturating_add(1);
        if !self.started {
            return;
        }
        self.ticks_in_quantum += 1;
        if self.ticks_in_quantum < self.quantum {
            return;
        }
        self.ticks_in_quantum = 0;
        let Some(next) = self.next_runnable() else {
            return;
        };
        if next == self.current {
            return;
        }
        // Save the outgoing thread's full resume state, then overwrite the
        // live frame so the ISR epilogue resumes the incoming thread.
        self.threads[self.current].frame = *frame;
        *frame = self.threads[next].frame;
        self.threads[next].dispatch_count = self.threads[next].dispatch_count.saturating_add(1);
        self.current = next;
        self.switches = self.switches.saturating_add(1);
    }

    fn spawn(&mut self, entry: extern "C" fn() -> !) -> Option<usize> {
        let index = self
            .threads
            .iter()
            .position(|thread| thread.state == ThreadState::Empty)?;
        let stack_base = self.threads[index].stack.0.as_ptr() as u64;
        let stack_top = stack_base.checked_add(THREAD_STACK_SIZE as u64)?;
        let rip = entry as usize as u64;
        // Mimic the post-CALL alignment contract: a Rust function body
        // expects RSP % 16 == 8 at entry. `iretq` pushes no return address,
        // so the initial RSP is biased by 8 from the 16-aligned stack top.
        let rsp = stack_top - 8;
        if !is_canonical_x86_64_48(rip) || !is_canonical_x86_64_48(rsp) {
            return None;
        }
        let mut frame = InterruptFrame::empty();
        frame.rip = rip;
        frame.cs = KERNEL_CODE_SELECTOR as u64;
        frame.ss = KERNEL_DATA_SELECTOR as u64;
        frame.rflags = THREAD_INITIAL_RFLAGS;
        frame.rsp = rsp;
        self.threads[index].frame = frame;
        self.threads[index].state = ThreadState::Runnable;
        Some(index)
    }
}

#[cfg(target_arch = "x86_64")]
struct SchedulerCell(UnsafeCell<Preemptive>);

// SAFETY: single-CPU bring-up only. Every mutation path either runs in the
// IRQ0 handler (IF=0 for the whole handler) or under `interrupt_free`, which
// masks interrupts, so no two accessors can ever overlap.
#[cfg(target_arch = "x86_64")]
unsafe impl Sync for SchedulerCell {}

#[cfg(target_arch = "x86_64")]
static SCHEDULER: SchedulerCell = SchedulerCell(UnsafeCell::new(Preemptive::new()));

#[cfg(all(target_arch = "x86_64", target_os = "none"))]
#[inline(always)]
fn read_rflags() -> u64 {
    let flags: u64;
    unsafe {
        core::arch::asm!(
            "pushfq",
            "pop {}",
            out(reg) flags,
            options(nomem, preserves_flags)
        );
    }
    flags
}

/// Runs `f` with maskable interrupts disabled, restoring the previous IF
/// state afterwards. Safe to call from thread context (IF=1) and from the
/// IRQ handler (IF=0; nothing is restored in that case).
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub fn interrupt_free<R>(f: impl FnOnce() -> R) -> R {
    let flags = read_rflags();
    unsafe { disable_interrupts() };
    let result = f();
    if flags & RFLAGS_IF != 0 {
        unsafe { enable_interrupts() };
    }
    result
}

/// Host (unit-test) variant: `cli`/`sti` are privileged and would fault in
/// ring 3, so the critical section degrades to a plain call. Host tests
/// serialize scheduler access through a test mutex instead.
#[cfg(all(target_arch = "x86_64", not(target_os = "none")))]
pub fn interrupt_free<R>(f: impl FnOnce() -> R) -> R {
    f()
}

#[cfg(target_arch = "x86_64")]
fn with_scheduler<R>(f: impl FnOnce(&mut Preemptive) -> R) -> R {
    interrupt_free(|| f(unsafe { &mut *SCHEDULER.0.get() }))
}

/// Registers the context that executed `kernel_entry` as the runnable
/// boot/idle thread. Must be called once, before [`start`], with interrupts
/// still disabled. Its saved frame stays uninitialized until the first
/// preemption fills it; it is the current thread, so it is never dispatched
/// *to* before it has been dispatched *from*.
#[cfg(target_arch = "x86_64")]
pub fn register_boot_context() {
    with_scheduler(|scheduler| {
        scheduler.threads[BOOT_THREAD_INDEX].state = ThreadState::Runnable;
        scheduler.threads[BOOT_THREAD_INDEX].dispatch_count = 1;
        scheduler.current = BOOT_THREAD_INDEX;
    });
}

/// Allocates a kernel thread that starts executing `entry` on its own
/// dedicated stack. Returns the thread index, or `None` when the fixed-size
/// table is full or the synthesized entry frame would be non-canonical.
#[cfg(target_arch = "x86_64")]
pub fn spawn(entry: extern "C" fn() -> !) -> Option<usize> {
    with_scheduler(|scheduler| scheduler.spawn(entry))
}

/// Arms preemption. From this point every `quantum` PIT ticks the IRQ0
/// handler rotates the CPU to the next runnable thread.
#[cfg(target_arch = "x86_64")]
pub fn start(quantum_ticks: u32) {
    with_scheduler(|scheduler| {
        scheduler.quantum = quantum_ticks.max(1);
        scheduler.ticks_in_quantum = 0;
        scheduler.started = true;
    });
}

/// Returns `true` once [`start`] has armed preemption.
#[cfg(target_arch = "x86_64")]
pub fn is_started() -> bool {
    with_scheduler(|scheduler| scheduler.started)
}

/// IRQ0 entry point. Called from the common ISR with interrupts disabled;
/// `frame` is the live interrupt frame on the interrupted stack and may be
/// rewritten to resume a different thread.
#[cfg(target_arch = "x86_64")]
pub fn on_timer_tick(frame: &mut InterruptFrame) {
    with_scheduler(|scheduler| scheduler.tick(frame));
}

/// Marks the calling thread as exited and parks it. The next timer tick
/// schedules a runnable thread; an exited slot is never dispatched again.
#[cfg(target_arch = "x86_64")]
pub fn exit_current() -> ! {
    with_scheduler(|scheduler| {
        scheduler.threads[scheduler.current].state = ThreadState::Exited;
    });
    // The slot is already marked Exited, so even though this context remains
    // `current` until the next tick, it can never be dispatched again. Park
    // in a hlt loop; the timer IRQ wakes the CPU and schedules away.
    loop {
        unsafe { core::arch::asm!("hlt", options(nomem, nostack, preserves_flags)) };
    }
}

/// Total PIT ticks observed by the scheduler since boot.
#[cfg(target_arch = "x86_64")]
pub fn ticks() -> u64 {
    with_scheduler(|scheduler| scheduler.ticks)
}

/// Total number of completed preemptions since [`start`].
#[cfg(target_arch = "x86_64")]
pub fn switch_count() -> u64 {
    with_scheduler(|scheduler| scheduler.switches)
}

/// Number of times the given thread index has been dispatched, or `None`
/// for an out-of-range or never-registered slot.
#[cfg(target_arch = "x86_64")]
pub fn dispatch_count(index: usize) -> Option<u64> {
    with_scheduler(|scheduler| {
        scheduler
            .threads
            .get(index)
            .filter(|thread| thread.state != ThreadState::Empty)
            .map(|thread| thread.dispatch_count)
    })
}

/// Index of the thread currently running on this CPU.
#[cfg(target_arch = "x86_64")]
pub fn current_thread() -> usize {
    with_scheduler(|scheduler| scheduler.current)
}

/// Busy-waits for at least `ticks_to_wait` scheduler ticks. The CPU stays
/// interruptible, so the caller can still be preempted while waiting.
#[cfg(target_arch = "x86_64")]
pub fn sleep_ticks(ticks_to_wait: u64) {
    let start = ticks();
    while ticks().wrapping_sub(start) < ticks_to_wait {
        core::hint::spin_loop();
    }
}

#[cfg(all(test, target_arch = "x86_64"))]
mod tests {
    extern crate std;

    use super::*;
    use std::sync::Mutex;

    // The scheduler is a global singleton; serialize tests that drive it.
    static TEST_MUTEX: Mutex<()> = Mutex::new(());

    fn reset() {
        with_scheduler(|scheduler| *scheduler = Preemptive::new());
    }

    extern "C" fn dummy_a() -> ! {
        loop {
            core::hint::spin_loop();
        }
    }
    extern "C" fn dummy_b() -> ! {
        loop {
            core::hint::spin_loop();
        }
    }

    fn tick_n(frame: &mut InterruptFrame, n: u64) {
        for _ in 0..n {
            on_timer_tick(frame);
        }
    }

    #[test]
    fn zero_quantum_is_normalized_to_one_tick() {
        let _guard = TEST_MUTEX.lock().unwrap();
        reset();
        register_boot_context();
        start(0);
        assert_eq!(with_scheduler(|scheduler| scheduler.quantum), 1);
    }

    #[test]
    fn no_switch_before_start() {
        let _guard = TEST_MUTEX.lock().unwrap();
        reset();
        register_boot_context();
        assert!(spawn(dummy_a).is_some());
        let mut frame = InterruptFrame::empty();
        frame.rip = 0x1000;
        tick_n(&mut frame, 1000);
        assert_eq!(switch_count(), 0);
        assert_eq!(current_thread(), BOOT_THREAD_INDEX);
        assert!(ticks() >= 1000);
    }

    #[test]
    fn round_robin_rotates_on_quantum_expiry() {
        let _guard = TEST_MUTEX.lock().unwrap();
        reset();
        register_boot_context();
        let a = spawn(dummy_a).expect("spawn A");
        let b = spawn(dummy_b).expect("spawn B");
        start(2);

        let mut frame = InterruptFrame::empty();
        frame.rip = 0x2000;

        tick_n(&mut frame, 2);
        assert_eq!(current_thread(), a);
        tick_n(&mut frame, 2);
        assert_eq!(current_thread(), b);
        tick_n(&mut frame, 2);
        assert_eq!(current_thread(), BOOT_THREAD_INDEX);
        assert_eq!(switch_count(), 3);
    }

    #[test]
    fn quantum_boundary_is_respected() {
        let _guard = TEST_MUTEX.lock().unwrap();
        reset();
        register_boot_context();
        spawn(dummy_a).expect("spawn A");
        start(50);
        let mut frame = InterruptFrame::empty();
        tick_n(&mut frame, 49);
        assert_eq!(current_thread(), BOOT_THREAD_INDEX);
        tick_n(&mut frame, 1);
        assert_ne!(current_thread(), BOOT_THREAD_INDEX);
    }

    #[test]
    fn outgoing_frame_is_saved_and_incoming_restored() {
        let _guard = TEST_MUTEX.lock().unwrap();
        reset();
        register_boot_context();
        let a = spawn(dummy_a).expect("spawn A");
        start(1);

        let mut frame = InterruptFrame::empty();
        frame.rip = 0xdead_beef;
        frame.rsp = 0x1122_3000;
        tick_n(&mut frame, 1);
        // We now run thread A: the live frame carries its synthesized entry.
        assert_eq!(current_thread(), a);
        assert_eq!(frame.rip, dummy_a as *const () as usize as u64);
        assert_eq!(frame.cs, KERNEL_CODE_SELECTOR as u64);
        assert_eq!(frame.rflags, THREAD_INITIAL_RFLAGS);
        assert_eq!(frame.rsp % 16, 8);

        // Switch back to the boot context: it must resume exactly where it
        // was preempted.
        tick_n(&mut frame, 1);
        assert_eq!(current_thread(), BOOT_THREAD_INDEX);
        assert_eq!(frame.rip, 0xdead_beef);
        assert_eq!(frame.rsp, 0x1122_3000);
    }

    #[test]
    fn exited_threads_are_skipped() {
        let _guard = TEST_MUTEX.lock().unwrap();
        reset();
        register_boot_context();
        let a = spawn(dummy_a).expect("spawn A");
        let b = spawn(dummy_b).expect("spawn B");
        start(1);

        let mut frame = InterruptFrame::empty();
        tick_n(&mut frame, 1); // -> A
        assert_eq!(current_thread(), a);
        // Mark A exited from "its" context.
        with_scheduler(|scheduler| scheduler.threads[a].state = ThreadState::Exited);
        tick_n(&mut frame, 1); // -> B, skipping nothing yet
        assert_eq!(current_thread(), b);
        tick_n(&mut frame, 1); // -> boot, A must be skipped
        assert_eq!(current_thread(), BOOT_THREAD_INDEX);
        tick_n(&mut frame, 1); // -> B again
        assert_eq!(current_thread(), b);
    }

    #[test]
    fn single_runnable_thread_never_switches() {
        let _guard = TEST_MUTEX.lock().unwrap();
        reset();
        register_boot_context();
        start(1);
        let mut frame = InterruptFrame::empty();
        frame.rip = 0x4000;
        tick_n(&mut frame, 10);
        assert_eq!(current_thread(), BOOT_THREAD_INDEX);
        assert_eq!(switch_count(), 0);
        assert_eq!(frame.rip, 0x4000);
    }

    #[test]
    fn spawn_table_exhaustion_is_reported() {
        let _guard = TEST_MUTEX.lock().unwrap();
        reset();
        register_boot_context();
        for expected in 1..MAX_THREADS {
            assert_eq!(spawn(dummy_a), Some(expected));
        }
        assert_eq!(spawn(dummy_a), None);
    }

    #[test]
    fn dispatch_counts_track_preemptions() {
        let _guard = TEST_MUTEX.lock().unwrap();
        reset();
        register_boot_context();
        let a = spawn(dummy_a).expect("spawn A");
        start(1);
        let mut frame = InterruptFrame::empty();
        tick_n(&mut frame, 4);
        assert_eq!(dispatch_count(BOOT_THREAD_INDEX), Some(1 + 2));
        assert_eq!(dispatch_count(a), Some(2));
        assert_eq!(dispatch_count(MAX_THREADS + 1), None);
    }
}
