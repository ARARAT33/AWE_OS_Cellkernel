#![no_std]
#![allow(bad_asm_style)]

//! QEMU exception-injection self-test (cargo feature "kernel-selftest").
//!
//! This module exists only in validation builds. It deliberately injects
//! #DE, #UD and #PF, escalates a broken-stack fault into #DF and waits for
//! an externally injected NMI, requiring the real IDT handlers, the
//! corrected long-mode InterruptFrame model and the dedicated IST stacks
//! to recover execution. Default/production builds contain no self-test
//! code path: the entire module is feature-gated.
//!
//! # Injection contract
//!
//! 1. The driver arms one vector and calls an assembly injector.
//! 2. The injector stores its own resume RIP into RESUME_RIP (and, for the
//!    #DF case, the good RSP into RESUME_RSP) and then faults on purpose.
//! 3. try_recover, called from the common ISR, validates the hardware
//!    frame and the IST stack bounds, fixes up RIP/RSP and returns true so
//!    the ISR performs iretq back into the injector's resume label.
//! 4. #MC is deliberately not injected: QEMU provides no machine-check
//!    injection mechanism, so IST3 delivery remains hardware-only evidence
//!    and is not claimed by this module.

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use super::isr::InterruptFrame;
use super::{ist, serial_write_str};
use crate::process::{
    ProcessDescriptor, ProcessId, ProcessManager, ProcessState, ResourceBudget,
    context::{CpuContext, ProcessContext},
    context_switch::SwitchFrame,
    switch_to_dispatch,
    x86_64_backend::switch_checked,
};

/// Sentinel for "no vector armed".
const EXPECT_NONE: u32 = u32::MAX;

/// First unmapped address above the 1 GiB supervisor-only bootstrap map.
const EXPECTED_PF_ADDRESS: u64 = 0x4000_0000;

/// How long to wait for the monitor-injected NMI, in PIT ticks (1000 Hz).
const NMI_WAIT_TICKS: u64 = 5_000;

/// Vector the self-test currently arms; EXPECT_NONE when disarmed.
static ARMED_VECTOR: AtomicU32 = AtomicU32::new(EXPECT_NONE);
/// Resume RIP installed by the fault recovery path.
#[unsafe(no_mangle)]
pub static RESUME_RIP: AtomicU64 = AtomicU64::new(0);
/// Known-good RSP restored after the #DF recovery.
#[unsafe(no_mangle)]
pub static RESUME_RSP: AtomicU64 = AtomicU64::new(0);
/// Whether an externally injected NMI is expected on IST1.
static NMI_EXPECTED: AtomicBool = AtomicBool::new(false);
/// Set by the NMI recovery path.
static NMI_HANDLED: AtomicBool = AtomicBool::new(false);

/// Round-robin phase of test process A: 0 = never entered, 1 = entered and
/// yielded, 2 = resumed from the persisted context and exited.
static SWITCH_A_PHASE: AtomicU32 = AtomicU32::new(0);
/// Same phase machine for test process B.
static SWITCH_B_PHASE: AtomicU32 = AtomicU32::new(0);
static mut SWITCH_MANAGER: ProcessManager<2> = ProcessManager::new();
static mut SWITCH_BOOT_FRAME: SwitchFrame = SwitchFrame::empty();
static mut SWITCH_A_FRAME: SwitchFrame = SwitchFrame::empty();
static mut SWITCH_B_FRAME: SwitchFrame = SwitchFrame::empty();
static mut SWITCH_A_STACK: [u8; 4096] = [0; 4096];
static mut SWITCH_B_STACK: [u8; 4096] = [0; 4096];

core::arch::global_asm!(
    r#".intel_syntax noprefix
.section .text
.global awe_selftest_inject_de
.type awe_selftest_inject_de, @function
awe_selftest_inject_de:
    lea rax, [rip + .Lde_resume]
    lea rcx, [rip + RESUME_RIP]
    mov qword ptr [rcx], rax
    xor ecx, ecx
    xor edx, edx
    xor eax, eax
    div ecx
.Lde_resume:
    ret

.global awe_selftest_inject_ud
.type awe_selftest_inject_ud, @function
awe_selftest_inject_ud:
    lea rax, [rip + .Lud_resume]
    lea rcx, [rip + RESUME_RIP]
    mov qword ptr [rcx], rax
    ud2
.Lud_resume:
    ret

.global awe_selftest_inject_pf
.type awe_selftest_inject_pf, @function
awe_selftest_inject_pf:
    lea rax, [rip + .Lpf_resume]
    lea rcx, [rip + RESUME_RIP]
    mov qword ptr [rcx], rax
    mov rax, qword ptr [0x40000000]
.Lpf_resume:
    ret

.global awe_selftest_inject_df
.type awe_selftest_inject_df, @function
awe_selftest_inject_df:
    lea rax, [rip + .Ldf_resume]
    lea rcx, [rip + RESUME_RIP]
    mov qword ptr [rcx], rax
    lea rcx, [rip + RESUME_RSP]
    mov qword ptr [rcx], rsp
    mov rsp, 0x8000000000000000
    push rax
.Ldf_resume:
    ret

.att_syntax prefix
"#
);

unsafe extern "C" {
    fn awe_selftest_inject_de();
    fn awe_selftest_inject_ud();
    fn awe_selftest_inject_pf();
    fn awe_selftest_inject_df();
}

/// Run the exception/IST injection sequence. Called only from the kernel
/// boot path in validation builds (see kernel/src/entry.rs).
pub fn run() {
    serial_write_str("AWEOS SELFTEST: begin exception injection\r\n");

    ARMED_VECTOR.store(0, Ordering::SeqCst); // #DE
    unsafe {
        awe_selftest_inject_de();
    }
    serial_write_str("AWEOS SELFTEST: driver resumed after #DE\r\n");

    ARMED_VECTOR.store(6, Ordering::SeqCst); // #UD
    unsafe {
        awe_selftest_inject_ud();
    }
    serial_write_str("AWEOS SELFTEST: driver resumed after #UD\r\n");

    ARMED_VECTOR.store(14, Ordering::SeqCst); // #PF above the bootstrap map
    unsafe {
        awe_selftest_inject_pf();
    }
    serial_write_str("AWEOS SELFTEST: driver resumed after #PF\r\n");

    ARMED_VECTOR.store(8, Ordering::SeqCst); // broken-stack #DF escalation
    unsafe {
        awe_selftest_inject_df();
    }
    serial_write_str("AWEOS SELFTEST: driver resumed after #DF\r\n");

    serial_write_str("AWEOS SELFTEST: testing x86_64 task-stack dispatch\r\n");
    if !run_context_switch_selftest() {
        fail("two-stack context-switch/dispatch acknowledgement failed");
        return;
    }
    serial_write_str("AWEOS SELFTEST: two-stack dispatch completion passed\r\n");

    serial_write_str("AWEOS SELFTEST: waiting for external NMI\r\n");
    NMI_EXPECTED.store(true, Ordering::SeqCst);
    // Unmask interrupts only now: the PIT tick bounds the NMI wait.
    unsafe {
        core::arch::asm!("sti", options(nomem, nostack, preserves_flags));
    }
    let deadline = super::isr_stubs::timer_irq_count() + NMI_WAIT_TICKS;
    while !NMI_HANDLED.load(Ordering::SeqCst) {
        if super::isr_stubs::timer_irq_count() >= deadline {
            serial_write_str("AWEOS SELFTEST: NMI not received before timeout\r\n");
            return;
        }
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
    serial_write_str("AWEOS SELFTEST: ALL PHASES PASSED\r\n");
}

/// Exercise the real assembly switch path with two fresh process stacks in a
/// round-robin. Each process yields once and its outgoing context must be
/// persisted by the driver; each process must then resume from that persisted
/// context (not from its original entry point) before it exits and is reaped.
fn run_context_switch_selftest() -> bool {
    const PROCESS_A: ProcessId = ProcessId(1);
    const PROCESS_B: ProcessId = ProcessId(2);

    SWITCH_A_PHASE.store(0, Ordering::SeqCst);
    SWITCH_B_PHASE.store(0, Ordering::SeqCst);

    unsafe {
        let manager_ptr = core::ptr::addr_of_mut!(SWITCH_MANAGER);
        let boot_frame_ptr = core::ptr::addr_of_mut!(SWITCH_BOOT_FRAME);
        let a_frame_ptr = core::ptr::addr_of_mut!(SWITCH_A_FRAME);
        let b_frame_ptr = core::ptr::addr_of_mut!(SWITCH_B_FRAME);

        *manager_ptr = ProcessManager::new();
        let manager = &mut *manager_ptr;
        if !register_process(
            manager,
            PROCESS_A,
            process_a_entry as *const () as usize as u64,
            SWITCH_A_STACK.as_mut_ptr(),
        ) || !register_process(
            manager,
            PROCESS_B,
            process_b_entry as *const () as usize as u64,
            SWITCH_B_STACK.as_mut_ptr(),
        ) {
            return false;
        }

        *boot_frame_ptr = SwitchFrame::from_context(&CpuContext::kernel_entry(
            run_context_switch_selftest as *const () as usize as u64,
            current_rsp(),
            0,
        ));

        // Dispatch A: fresh entry on its own stack, must yield back.
        if !dispatch_process(manager_ptr, boot_frame_ptr, PROCESS_A) {
            return false;
        }
        if SWITCH_A_PHASE.load(Ordering::SeqCst) != 1 {
            return false;
        }
        if !persist_yield(
            manager_ptr,
            PROCESS_A,
            a_frame_ptr,
            process_a_entry as *const () as usize as u64,
            SWITCH_A_STACK.as_mut_ptr(),
        ) {
            return false;
        }
        serial_write_str("AWEOS SELFTEST: process A yielded; outgoing context persisted\r\n");

        // Dispatch B: fresh entry, yields; A is requeued by commit_running.
        if !dispatch_process(manager_ptr, boot_frame_ptr, PROCESS_B) {
            return false;
        }
        if SWITCH_B_PHASE.load(Ordering::SeqCst) != 1 {
            return false;
        }
        if !persist_yield(
            manager_ptr,
            PROCESS_B,
            b_frame_ptr,
            process_b_entry as *const () as usize as u64,
            SWITCH_B_STACK.as_mut_ptr(),
        ) {
            return false;
        }
        serial_write_str("AWEOS SELFTEST: process B yielded; outgoing context persisted\r\n");

        // Re-dispatch A: it must resume from the persisted context and exit.
        if !dispatch_process(manager_ptr, boot_frame_ptr, PROCESS_A) {
            return false;
        }
        if SWITCH_A_PHASE.load(Ordering::SeqCst) != 2 {
            return false;
        }
        let manager = &mut *manager_ptr;
        if manager.confirm_switched_to_idle(PROCESS_A).is_err() || manager.reap(PROCESS_A).is_err()
        {
            return false;
        }

        // Re-dispatch B: resumes, exits, is reaped; registry must be empty.
        if !dispatch_process(manager_ptr, boot_frame_ptr, PROCESS_B) {
            return false;
        }
        if SWITCH_B_PHASE.load(Ordering::SeqCst) != 2 {
            return false;
        }
        let manager = &mut *manager_ptr;
        if manager.confirm_switched_to_idle(PROCESS_B).is_err() || manager.reap(PROCESS_B).is_err()
        {
            return false;
        }

        serial_write_str("AWEOS SELFTEST: round-robin resume from persisted contexts passed\r\n");
        manager.len() == 0
    }
}

/// Register a process with a fresh 4 KiB stack and make it runnable.
/// `stack` must point to 4096 writable, pinned, kernel-mapped bytes.
fn register_process(
    manager: &mut ProcessManager<2>,
    process: ProcessId,
    entry: u64,
    stack: *mut u8,
) -> bool {
    let stack_start = stack as u64;
    let stack_top = (stack_start + 4096) & !0xF;
    let Some(task_rsp) = stack_top.checked_sub(8) else {
        return false;
    };
    let descriptor = ProcessDescriptor {
        id: process,
        state: ProcessState::Created,
        budget: ResourceBudget {
            cpu_ticks: 100,
            memory_bytes: 4096,
            ipc_messages: 8,
        },
    };
    let context = ProcessContext::new(process, CpuContext::kernel_entry(entry, task_rsp, 0));
    manager.register(descriptor, context).is_ok() && manager.make_runnable(process).is_ok()
}

/// Dequeue the exact expected process and switch to it on its own stack.
unsafe fn dispatch_process(
    manager_ptr: *mut ProcessManager<2>,
    boot_frame_ptr: *mut SwitchFrame,
    expected: ProcessId,
) -> bool {
    let target = match (*manager_ptr).next_dispatch() {
        Ok(target) if target.process == expected => target,
        _ => return false,
    };
    switch_to_dispatch(manager_ptr, boot_frame_ptr, target).is_ok()
}

/// Persist a just-yielded process's outgoing frame and verify it describes a
/// mid-execution resume point inside that process's own stack.
unsafe fn persist_yield(
    manager_ptr: *mut ProcessManager<2>,
    process: ProcessId,
    frame_ptr: *mut SwitchFrame,
    entry_rip: u64,
    stack: *mut u8,
) -> bool {
    let stack_start = stack as u64;
    let stack_end = stack_start + 4096;
    let manager = &mut *manager_ptr;
    if manager
        .persist_outgoing_frame(process, &*frame_ptr)
        .is_err()
    {
        return false;
    }
    let Some(persisted) = manager.process_context(process) else {
        return false;
    };
    // The saved RIP must have advanced past the process entry (it is the
    // resume label inside the switch backend), and RSP must still point
    // into the process's own stack.
    persisted.cpu.rip != entry_rip
        && (stack_start..stack_end).contains(&persisted.cpu.rsp)
        && persisted.is_valid()
}

/// Verify the manager currently publishes this exact process as Running.
fn entered_as_running(process: ProcessId) -> bool {
    let manager = unsafe { &mut *core::ptr::addr_of_mut!(SWITCH_MANAGER) };
    manager.current_process_id() == Some(process)
        && manager
            .descriptor(process)
            .is_some_and(|descriptor| descriptor.state == ProcessState::Running)
}

/// Process A entry: runs on its own stack, yields once, and must be resumed
/// from the persisted context before it exits. Any unexpected state fails the
/// self-test and halts; this is a validation-only code path.
extern "C" fn process_a_entry() -> ! {
    const PROCESS: ProcessId = ProcessId(1);
    let frame_ptr = core::ptr::addr_of_mut!(SWITCH_A_FRAME);
    let boot_frame_ptr = core::ptr::addr_of!(SWITCH_BOOT_FRAME);

    if !entered_as_running(PROCESS) {
        fail("process A entry did not observe the acknowledged Running state");
        unsafe { super::boot::halt() };
    }
    SWITCH_A_PHASE.store(1, Ordering::SeqCst);
    if unsafe { switch_checked(&mut *frame_ptr, &*boot_frame_ptr) }.is_err() {
        fail("process A failed to yield back to the boot stack");
        unsafe { super::boot::halt() };
    }

    // Control returns here only after a re-dispatch through the persisted
    // context; the callback must have re-published Running before resuming.
    if !entered_as_running(PROCESS) || SWITCH_A_PHASE.swap(2, Ordering::SeqCst) != 1 {
        fail("process A was not resumed from its persisted context");
        unsafe { super::boot::halt() };
    }
    let exited = unsafe {
        (&mut *core::ptr::addr_of_mut!(SWITCH_MANAGER))
            .exit(PROCESS)
            .is_ok()
    };
    if !exited {
        fail("process A could not exit");
        unsafe { super::boot::halt() };
    }
    if unsafe { switch_checked(&mut *frame_ptr, &*boot_frame_ptr) }.is_err() {
        fail("process A failed to switch back after exit");
    }
    // The scheduler must reap an exited process and never resume it.
    fail("exited process A was resumed");
    unsafe { super::boot::halt() };
}

/// Process B entry: same phase machine as process A on a separate stack.
extern "C" fn process_b_entry() -> ! {
    const PROCESS: ProcessId = ProcessId(2);
    let frame_ptr = core::ptr::addr_of_mut!(SWITCH_B_FRAME);
    let boot_frame_ptr = core::ptr::addr_of!(SWITCH_BOOT_FRAME);

    if !entered_as_running(PROCESS) {
        fail("process B entry did not observe the acknowledged Running state");
        unsafe { super::boot::halt() };
    }
    SWITCH_B_PHASE.store(1, Ordering::SeqCst);
    if unsafe { switch_checked(&mut *frame_ptr, &*boot_frame_ptr) }.is_err() {
        fail("process B failed to yield back to the boot stack");
        unsafe { super::boot::halt() };
    }

    if !entered_as_running(PROCESS) || SWITCH_B_PHASE.swap(2, Ordering::SeqCst) != 1 {
        fail("process B was not resumed from its persisted context");
        unsafe { super::boot::halt() };
    }
    let exited = unsafe {
        (&mut *core::ptr::addr_of_mut!(SWITCH_MANAGER))
            .exit(PROCESS)
            .is_ok()
    };
    if !exited {
        fail("process B could not exit");
        unsafe { super::boot::halt() };
    }
    if unsafe { switch_checked(&mut *frame_ptr, &*boot_frame_ptr) }.is_err() {
        fail("process B failed to switch back after exit");
    }
    fail("exited process B was resumed");
    unsafe { super::boot::halt() };
}

/// Attempt to recover the armed self-test vector. Returns true when the
/// frame was fixed up and the ISR must return through iretq.
pub fn try_recover(frame: &mut InterruptFrame) -> bool {
    let vector = frame.vector as u8;
    let armed = ARMED_VECTOR.load(Ordering::SeqCst);

    if u32::from(vector) == armed {
        match vector {
            0 => recover_fault(frame, "#DE"),
            6 => recover_fault(frame, "#UD"),
            14 => recover_page_fault(frame),
            8 => recover_double_fault(frame),
            _ => false,
        }
    } else if vector == ist::NMI_VECTOR && NMI_EXPECTED.load(Ordering::SeqCst) {
        recover_nmi()
    } else {
        // Not an armed self-test event: fall through to the normal handler.
        false
    }
}

/// Recover a same-CPL CPU exception and resume at the injector label.
fn recover_fault(frame: &mut InterruptFrame, name: &str) -> bool {
    // Runtime proof of the corrected long-mode frame model: a same-CPL
    // hardware frame must carry the interrupted SS:RSP and a canonical RIP.
    if !frame.validate() || frame.interrupted_from_user() {
        fail("frame validation failed before recovery");
        return false;
    }
    frame.rip = RESUME_RIP.load(Ordering::SeqCst);
    ARMED_VECTOR.store(EXPECT_NONE, Ordering::SeqCst);
    serial_write_str("AWEOS SELFTEST: ");
    serial_write_str(name);
    serial_write_str(" handled and recovered (frame model OK)\r\n");
    true
}

/// Recover the injected #PF and verify CR2 points at the unmapped probe.
fn recover_page_fault(frame: &mut InterruptFrame) -> bool {
    let cr2: u64;
    unsafe {
        core::arch::asm!(
            "mov {}, cr2",
            out(reg) cr2,
            options(nomem, nostack, preserves_flags)
        );
    }
    if cr2 != EXPECTED_PF_ADDRESS {
        fail("unexpected page-fault address");
        return false;
    }
    if !frame.validate() || frame.interrupted_from_user() {
        fail("frame validation failed before recovery");
        return false;
    }
    frame.rip = RESUME_RIP.load(Ordering::SeqCst);
    ARMED_VECTOR.store(EXPECT_NONE, Ordering::SeqCst);
    serial_write_str("AWEOS SELFTEST: #PF handled and recovered (CR2 = ");
    print_u64_hex(cr2);
    serial_write_str(", frame model OK)\r\n");
    true
}

/// Recover the broken-stack double fault. The handler must already run on
/// the dedicated IST2 stack; the escalated frame carries the broken RSP,
/// so the saved good RSP is restored before returning.
fn recover_double_fault(frame: &mut InterruptFrame) -> bool {
    let rsp = current_rsp();
    let bounds = ist::critical_stack_bounds();
    if !(bounds[1].0..bounds[1].1).contains(&rsp) {
        fail("#DF handler not running on the dedicated IST2 stack");
        return false;
    }
    frame.rip = RESUME_RIP.load(Ordering::SeqCst);
    frame.rsp = RESUME_RSP.load(Ordering::SeqCst);
    ARMED_VECTOR.store(EXPECT_NONE, Ordering::SeqCst);
    serial_write_str("AWEOS SELFTEST: #DF handled on IST2 (rsp inside dedicated stack)\r\n");
    true
}

/// Recover the monitor-injected NMI on the dedicated IST1 stack.
fn recover_nmi() -> bool {
    let rsp = current_rsp();
    let bounds = ist::critical_stack_bounds();
    if !(bounds[0].0..bounds[0].1).contains(&rsp) {
        fail("NMI handler not running on the dedicated IST1 stack");
        return false;
    }
    NMI_EXPECTED.store(false, Ordering::SeqCst);
    NMI_HANDLED.store(true, Ordering::SeqCst);
    serial_write_str("AWEOS SELFTEST: NMI handled on IST1 (rsp inside dedicated stack)\r\n");
    true
}

fn fail(reason: &str) {
    ARMED_VECTOR.store(EXPECT_NONE, Ordering::SeqCst);
    serial_write_str("AWEOS SELFTEST: FAIL: ");
    serial_write_str(reason);
    serial_write_str("\r\n");
}

fn current_rsp() -> u64 {
    let rsp: u64;
    unsafe {
        core::arch::asm!("mov {}, rsp", out(reg) rsp, options(nomem, nostack, preserves_flags));
    }
    rsp
}

fn print_u64_hex(mut val: u64) {
    let hex = b"0123456789ABCDEF";
    let mut buf = [b'0'; 18];
    buf[0] = b'0';
    buf[1] = b'x';
    for i in (2..18).rev() {
        buf[i] = hex[(val & 0xF) as usize];
        val >>= 4;
    }
    for b in buf {
        super::serial_write_byte(b);
    }
}
