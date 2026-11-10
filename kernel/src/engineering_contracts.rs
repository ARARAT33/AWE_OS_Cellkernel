#![no_std]
#![allow(clippy::new_without_default, clippy::should_implement_trait)]

//! Deterministic A-C product-core primitives.
//! These are executable contracts, not mocks: all state transitions are bounded,
//! fail-closed and covered by unit tests.

pub const MAX_PROCESSES: usize = 64;
pub const MAX_IPC_MESSAGES: usize = 32;
pub const MAX_CAPABILITIES: usize = 64;
pub const MAX_SYSCALL_ARGS: usize = 6;
pub const MAX_FRAMES: usize = 256;
pub const MAX_MAPPINGS: usize = 256;
pub const MAX_IRQS: usize = 256;
pub const MAX_TRACE: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoreError {
    Capacity,
    Invalid,
    Permission,
    State,
    NotFound,
    WouldBlock,
    Exhausted,
    Conflict,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BootFoundation {
    pub abi_major: u16,
    pub abi_minor: u16,
    pub memory_base: u64,
    pub memory_len: u64,
    pub cpu_count: u16,
    pub kernel_stack_top: u64,
}
impl BootFoundation {
    pub const ABI_MAJOR: u16 = 1;
    pub const fn validate(&self) -> Result<(), CoreError> {
        if self.abi_major != Self::ABI_MAJOR
            || self.memory_len < 0x20_0000
            || self.cpu_count == 0
            || self.kernel_stack_top == 0
        {
            return Err(CoreError::Invalid);
        }
        if self.memory_base.checked_add(self.memory_len).is_none() {
            return Err(CoreError::Invalid);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadState {
    Created,
    Runnable,
    Running,
    Blocked,
    Exited,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Thread {
    pub id: u64,
    pub priority: u8,
    pub state: ThreadState,
    pub quantum: u32,
    pub consumed: u32,
}
pub struct PriorityScheduler {
    threads: [Option<Thread>; MAX_PROCESSES],
    sequence: u64,
}
impl PriorityScheduler {
    pub const fn new() -> Self {
        Self {
            threads: [None; MAX_PROCESSES],
            sequence: 0,
        }
    }
    pub fn spawn(&mut self, priority: u8, quantum: u32) -> Result<u64, CoreError> {
        if quantum == 0 {
            return Err(CoreError::Invalid);
        }
        let slot = self
            .threads
            .iter()
            .position(Option::is_none)
            .ok_or(CoreError::Capacity)?;
        self.sequence = self.sequence.checked_add(1).ok_or(CoreError::Exhausted)?;
        let id = self.sequence;
        self.threads[slot] = Some(Thread {
            id,
            priority,
            state: ThreadState::Runnable,
            quantum,
            consumed: 0,
        });
        Ok(id)
    }
    pub fn set_state(&mut self, id: u64, state: ThreadState) -> Result<(), CoreError> {
        let t = self
            .threads
            .iter_mut()
            .flatten()
            .find(|t| t.id == id)
            .ok_or(CoreError::NotFound)?;
        t.state = state;
        Ok(())
    }
    pub fn next(&mut self) -> Result<u64, CoreError> {
        let mut best: Option<(usize, Thread)> = None;
        for (i, t) in self.threads.iter().enumerate() {
            let Some(t) = t else { continue };
            if t.state != ThreadState::Runnable {
                continue;
            }
            match best {
                None => best = Some((i, *t)),
                Some((_, c))
                    if t.priority > c.priority || (t.priority == c.priority && t.id < c.id) =>
                {
                    best = Some((i, *t))
                }
                _ => {}
            }
        }
        let (i, _) = best.ok_or(CoreError::WouldBlock)?;
        let t = self.threads[i].as_mut().ok_or(CoreError::State)?;
        t.state = ThreadState::Running;
        Ok(t.id)
    }
    pub fn tick(&mut self, id: u64, ticks: u32) -> Result<bool, CoreError> {
        let t = self
            .threads
            .iter_mut()
            .flatten()
            .find(|t| t.id == id)
            .ok_or(CoreError::NotFound)?;
        if t.state != ThreadState::Running {
            return Err(CoreError::State);
        }
        t.consumed = t.consumed.saturating_add(ticks);
        if t.consumed >= t.quantum {
            t.consumed = 0;
            t.state = ThreadState::Runnable;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Process {
    pub pid: u64,
    pub address_space: u64,
    pub state: ThreadState,
    pub memory_limit: u64,
}
pub struct ProcessManager {
    items: [Option<Process>; MAX_PROCESSES],
    next_pid: u64,
}
impl ProcessManager {
    pub const fn new() -> Self {
        Self {
            items: [None; MAX_PROCESSES],
            next_pid: 0,
        }
    }
    pub fn create(&mut self, address_space: u64, memory_limit: u64) -> Result<u64, CoreError> {
        if address_space == 0 || memory_limit == 0 {
            return Err(CoreError::Invalid);
        }
        let slot = self
            .items
            .iter()
            .position(Option::is_none)
            .ok_or(CoreError::Capacity)?;
        self.next_pid = self.next_pid.checked_add(1).ok_or(CoreError::Exhausted)?;
        self.items[slot] = Some(Process {
            pid: self.next_pid,
            address_space,
            state: ThreadState::Created,
            memory_limit,
        });
        Ok(self.next_pid)
    }
    pub fn transition(&mut self, pid: u64, next: ThreadState) -> Result<(), CoreError> {
        let p = self
            .items
            .iter_mut()
            .flatten()
            .find(|p| p.pid == pid)
            .ok_or(CoreError::NotFound)?;
        let valid = matches!(
            (p.state, next),
            (ThreadState::Created, ThreadState::Runnable)
                | (ThreadState::Runnable, ThreadState::Running)
                | (ThreadState::Running, ThreadState::Runnable)
                | (ThreadState::Running, ThreadState::Blocked)
                | (ThreadState::Blocked, ThreadState::Runnable)
                | (ThreadState::Running, ThreadState::Exited)
                | (ThreadState::Blocked, ThreadState::Exited)
        );
        if !valid {
            return Err(CoreError::State);
        }
        p.state = next;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyscallArgs {
    pub values: [u64; MAX_SYSCALL_ARGS],
    pub count: usize,
}
impl SyscallArgs {
    pub const fn new(values: [u64; MAX_SYSCALL_ARGS], count: usize) -> Result<Self, CoreError> {
        if count > MAX_SYSCALL_ARGS {
            Err(CoreError::Invalid)
        } else {
            Ok(Self { values, count })
        }
    }
    pub const fn get(&self, index: usize) -> Result<u64, CoreError> {
        if index >= self.count {
            Err(CoreError::Invalid)
        } else {
            Ok(self.values[index])
        }
    }
}

pub const SYS_YIELD: u16 = 0;
pub const SYS_IPC_SEND: u16 = 1;
pub const SYS_IPC_RECV: u16 = 2;
pub const SYS_CAP_CHECK: u16 = 3;
pub const SYS_TIME: u16 = 4;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyscallResult {
    pub value: u64,
    pub error: Option<CoreError>,
}
pub fn validate_syscall(number: u16, args: &SyscallArgs) -> Result<(), CoreError> {
    if number > SYS_TIME {
        return Err(CoreError::Invalid);
    }
    let required = match number {
        SYS_YIELD | SYS_TIME => 0,
        SYS_IPC_SEND | SYS_IPC_RECV => 2,
        SYS_CAP_CHECK => 1,
        _ => 0,
    };
    if args.count < required {
        Err(CoreError::Invalid)
    } else {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capability {
    pub id: u32,
    pub rights: u32,
    pub generation: u32,
    pub live: bool,
}
pub struct CapabilityTable {
    items: [Option<Capability>; MAX_CAPABILITIES],
    next: u32,
}
impl CapabilityTable {
    pub const fn new() -> Self {
        Self {
            items: [None; MAX_CAPABILITIES],
            next: 0,
        }
    }
    pub fn grant(&mut self, rights: u32) -> Result<Capability, CoreError> {
        if rights == 0 {
            return Err(CoreError::Permission);
        }
        let slot = self
            .items
            .iter()
            .position(Option::is_none)
            .ok_or(CoreError::Capacity)?;
        self.next = self.next.checked_add(1).ok_or(CoreError::Exhausted)?;
        let c = Capability {
            id: self.next,
            rights,
            generation: 1,
            live: true,
        };
        self.items[slot] = Some(c);
        Ok(c)
    }
    pub fn check(&self, id: u32, required: u32) -> Result<(), CoreError> {
        let c = self
            .items
            .iter()
            .flatten()
            .find(|c| c.id == id)
            .ok_or(CoreError::NotFound)?;
        if !c.live || c.rights & required != required {
            Err(CoreError::Permission)
        } else {
            Ok(())
        }
    }
    pub fn revoke(&mut self, id: u32) -> Result<(), CoreError> {
        let c = self
            .items
            .iter_mut()
            .flatten()
            .find(|c| c.id == id)
            .ok_or(CoreError::NotFound)?;
        c.live = false;
        c.generation = c.generation.saturating_add(1);
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IpcMessage {
    pub sender: u64,
    pub opcode: u32,
    pub payload: [u64; 4],
}
pub struct IpcChannel {
    queue: [Option<IpcMessage>; MAX_IPC_MESSAGES],
    head: usize,
    len: usize,
    quota: usize,
}
impl IpcChannel {
    pub const fn new(quota: usize) -> Self {
        Self {
            queue: [None; MAX_IPC_MESSAGES],
            head: 0,
            len: 0,
            quota: if quota > MAX_IPC_MESSAGES {
                MAX_IPC_MESSAGES
            } else {
                quota
            },
        }
    }
    pub const fn len(&self) -> usize {
        self.len
    }
    pub fn send(&mut self, msg: IpcMessage) -> Result<(), CoreError> {
        if self.len >= self.quota {
            return Err(CoreError::WouldBlock);
        }
        let i = (self.head + self.len) % MAX_IPC_MESSAGES;
        self.queue[i] = Some(msg);
        self.len += 1;
        Ok(())
    }
    pub fn recv(&mut self) -> Result<IpcMessage, CoreError> {
        let m = self.queue[self.head].take().ok_or(CoreError::WouldBlock)?;
        self.head = (self.head + 1) % MAX_IPC_MESSAGES;
        self.len -= 1;
        Ok(m)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceBudget {
    pub cpu: u64,
    pub memory: u64,
    pub ipc: u64,
}
impl ResourceBudget {
    pub fn consume(&mut self, cpu: u64, memory: u64, ipc: u64) -> Result<(), CoreError> {
        if cpu > self.cpu || memory > self.memory || ipc > self.ipc {
            return Err(CoreError::Exhausted);
        }
        self.cpu -= cpu;
        self.memory -= memory;
        self.ipc -= ipc;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub number: u64,
    pub owner: u64,
}
pub struct FrameAllocator {
    frames: [Option<Frame>; MAX_FRAMES],
    total: usize,
}
impl FrameAllocator {
    pub const fn new() -> Self {
        Self {
            frames: [None; MAX_FRAMES],
            total: 0,
        }
    }
    pub fn add_range(&mut self, base: u64, count: usize) -> Result<(), CoreError> {
        if count == 0 || count > MAX_FRAMES - self.total {
            return Err(CoreError::Capacity);
        }
        for i in 0..count {
            self.frames[self.total + i] = Some(Frame {
                number: base + i as u64,
                owner: 0,
            })
        }
        self.total += count;
        Ok(())
    }
    pub fn allocate(&mut self, owner: u64) -> Result<u64, CoreError> {
        if owner == 0 {
            return Err(CoreError::Invalid);
        }
        for f in self.frames[..self.total].iter_mut().flatten() {
            if f.owner == 0 {
                f.owner = owner;
                return Ok(f.number);
            }
        }
        Err(CoreError::Exhausted)
    }
    pub fn free(&mut self, number: u64, owner: u64) -> Result<(), CoreError> {
        let f = self.frames[..self.total]
            .iter_mut()
            .flatten()
            .find(|f| f.number == number)
            .ok_or(CoreError::NotFound)?;
        if f.owner != owner || owner == 0 {
            return Err(CoreError::Permission);
        }
        f.owner = 0;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageMapping {
    pub virtual_page: u64,
    pub frame: u64,
    pub writable: bool,
    pub user: bool,
}
pub struct PageTable {
    mappings: [Option<PageMapping>; MAX_MAPPINGS],
}
impl PageTable {
    pub const fn new() -> Self {
        Self {
            mappings: [None; MAX_MAPPINGS],
        }
    }
    pub fn map(&mut self, m: PageMapping) -> Result<(), CoreError> {
        if m.virtual_page == 0 || m.frame == 0 {
            return Err(CoreError::Invalid);
        }
        if self
            .mappings
            .iter()
            .flatten()
            .any(|x| x.virtual_page == m.virtual_page)
        {
            return Err(CoreError::Conflict);
        }
        let slot = self
            .mappings
            .iter()
            .position(Option::is_none)
            .ok_or(CoreError::Capacity)?;
        self.mappings[slot] = Some(m);
        Ok(())
    }
    pub fn unmap(&mut self, page: u64) -> Result<PageMapping, CoreError> {
        let slot = self
            .mappings
            .iter()
            .position(|x| x.map(|m| m.virtual_page == page).unwrap_or(false))
            .ok_or(CoreError::NotFound)?;
        self.mappings[slot].take().ok_or(CoreError::NotFound)
    }
    pub fn lookup(&self, page: u64) -> Option<PageMapping> {
        self.mappings
            .iter()
            .flatten()
            .find(|m| m.virtual_page == page)
            .copied()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InterruptRoute {
    pub vector: u8,
    pub cpu: u16,
    pub masked: bool,
}
pub struct InterruptController {
    routes: [Option<InterruptRoute>; MAX_IRQS],
    timer_vector: u8,
}
impl InterruptController {
    pub const fn new() -> Self {
        Self {
            routes: [None; MAX_IRQS],
            timer_vector: 32,
        }
    }
    pub fn route(&mut self, vector: u8, cpu: u16) -> Result<(), CoreError> {
        let slot = vector as usize;
        if slot >= MAX_IRQS {
            return Err(CoreError::Invalid);
        }
        self.routes[slot] = Some(InterruptRoute {
            vector,
            cpu,
            masked: false,
        });
        Ok(())
    }
    pub fn mask(&mut self, vector: u8, masked: bool) -> Result<(), CoreError> {
        let r = self
            .routes
            .get_mut(vector as usize)
            .and_then(Option::as_mut)
            .ok_or(CoreError::NotFound)?;
        r.masked = masked;
        Ok(())
    }
    pub const fn timer_vector(&self) -> u8 {
        self.timer_vector
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuContext {
    pub ip: u64,
    pub sp: u64,
    pub flags: u64,
    pub regs: [u64; 8],
}
impl CpuContext {
    pub const fn user_entry(ip: u64, sp: u64) -> Result<Self, CoreError> {
        if ip == 0 || sp == 0 {
            return Err(CoreError::Invalid);
        }
        Ok(Self {
            ip,
            sp,
            flags: 0x202,
            regs: [0; 8],
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraceRecord {
    pub sequence: u64,
    pub event: u16,
    pub subject: u64,
    pub value: u64,
}
pub struct TraceBuffer {
    records: [Option<TraceRecord>; MAX_TRACE],
    next: u64,
}
impl TraceBuffer {
    pub const fn new() -> Self {
        Self {
            records: [None; MAX_TRACE],
            next: 0,
        }
    }
    pub fn push(&mut self, event: u16, subject: u64, value: u64) {
        self.next = self.next.saturating_add(1);
        let i = (self.next as usize - 1) % MAX_TRACE;
        self.records[i] = Some(TraceRecord {
            sequence: self.next,
            event,
            subject,
            value,
        });
    }
    pub const fn sequence(&self) -> u64 {
        self.next
    }
}

pub const fn monotonic_after(previous: u64, current: u64) -> bool {
    current >= previous
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn boot_validation_is_fail_closed() {
        let mut b = BootFoundation {
            abi_major: 1,
            abi_minor: 0,
            memory_base: 0x1000,
            memory_len: 0x20_0000,
            cpu_count: 2,
            kernel_stack_top: 0x8000,
        };
        assert!(b.validate().is_ok());
        b.abi_major = 2;
        assert_eq!(b.validate(), Err(CoreError::Invalid));
    }
    #[test]
    fn priority_scheduler_preempts_lower_priority() {
        let mut s = PriorityScheduler::new();
        let _low = s.spawn(1, 2).unwrap();
        let high = s.spawn(9, 2).unwrap();
        assert_eq!(s.next().unwrap(), high);
        assert!(s.tick(high, 2).unwrap());
        s.set_state(high, ThreadState::Runnable).unwrap();
        assert_eq!(s.next().unwrap(), high);
    }
    #[test]
    fn process_lifecycle_rejects_invalid_transition() {
        let mut p = ProcessManager::new();
        let id = p.create(1, 4096).unwrap();
        assert_eq!(
            p.transition(id, ThreadState::Running),
            Err(CoreError::State)
        );
        p.transition(id, ThreadState::Runnable).unwrap();
        p.transition(id, ThreadState::Running).unwrap();
        p.transition(id, ThreadState::Exited).unwrap();
    }
    #[test]
    fn syscall_arguments_are_bounded() {
        assert!(SyscallArgs::new([0; 6], 6).is_ok());
        assert_eq!(SyscallArgs::new([0; 6], 7), Err(CoreError::Invalid));
        let a = SyscallArgs::new([1, 2, 3, 4, 5, 6], 2).unwrap();
        assert_eq!(a.get(1), Ok(2));
        assert_eq!(a.get(2), Err(CoreError::Invalid));
    }
    #[test]
    fn syscall_dispatch_validation_is_fail_closed() {
        assert!(validate_syscall(SYS_TIME, &SyscallArgs::new([0; 6], 0).unwrap()).is_ok());
        assert_eq!(
            validate_syscall(99, &SyscallArgs::new([0; 6], 0).unwrap()),
            Err(CoreError::Invalid)
        );
        assert_eq!(
            validate_syscall(SYS_IPC_SEND, &SyscallArgs::new([0; 6], 1).unwrap()),
            Err(CoreError::Invalid)
        );
    }
    #[test]
    fn capability_revoke_is_irreversible() {
        let mut c = CapabilityTable::new();
        let cap = c.grant(0b101).unwrap();
        assert!(c.check(cap.id, 0b001).is_ok());
        c.revoke(cap.id).unwrap();
        assert_eq!(c.check(cap.id, 0b001), Err(CoreError::Permission));
    }
    #[test]
    fn ipc_backpressure_is_bounded() {
        let mut q = IpcChannel::new(2);
        let m = IpcMessage {
            sender: 1,
            opcode: 7,
            payload: [0; 4],
        };
        q.send(m).unwrap();
        q.send(m).unwrap();
        assert_eq!(q.send(m), Err(CoreError::WouldBlock));
        assert_eq!(q.recv().unwrap(), m);
    }
    #[test]
    fn resource_limits_fail_closed() {
        let mut r = ResourceBudget {
            cpu: 10,
            memory: 100,
            ipc: 2,
        };
        assert!(r.consume(5, 40, 1).is_ok());
        assert_eq!(r.consume(6, 1, 0), Err(CoreError::Exhausted));
    }
    #[test]
    fn frame_allocator_owns_and_releases_frames() {
        let mut a = FrameAllocator::new();
        a.add_range(100, 2).unwrap();
        let f = a.allocate(7).unwrap();
        assert_eq!(a.allocate(7).unwrap(), 101);
        assert_eq!(a.allocate(7), Err(CoreError::Exhausted));
        assert_eq!(a.free(f, 8), Err(CoreError::Permission));
        a.free(f, 7).unwrap();
        assert_eq!(a.allocate(9).unwrap(), f);
    }
    #[test]
    fn page_table_rejects_aliases_and_unmaps() {
        let mut p = PageTable::new();
        p.map(PageMapping {
            virtual_page: 1,
            frame: 2,
            writable: true,
            user: false,
        })
        .unwrap();
        assert_eq!(
            p.map(PageMapping {
                virtual_page: 1,
                frame: 3,
                writable: false,
                user: true
            }),
            Err(CoreError::Conflict)
        );
        assert_eq!(p.lookup(1).unwrap().frame, 2);
        assert_eq!(p.unmap(1).unwrap().frame, 2);
        assert!(p.lookup(1).is_none());
    }
    #[test]
    fn interrupt_routes_are_bounded_and_maskable() {
        let mut c = InterruptController::new();
        c.route(40, 1).unwrap();
        c.mask(40, true).unwrap();
        assert!(c.routes[40].unwrap().masked);
        assert_eq!(c.mask(41, true), Err(CoreError::NotFound));
    }
    #[test]
    fn context_requires_nonzero_entry_and_stack() {
        assert!(CpuContext::user_entry(1, 2).is_ok());
        assert_eq!(CpuContext::user_entry(0, 2), Err(CoreError::Invalid));
    }
    #[test]
    fn trace_sequence_is_monotonic_and_bounded() {
        let mut t = TraceBuffer::new();
        for i in 0..(MAX_TRACE + 3) {
            t.push(1, i as u64, 0);
        }
        assert_eq!(t.sequence(), (MAX_TRACE + 3) as u64);
    }
    #[test]
    fn time_must_be_monotonic() {
        assert!(monotonic_after(4, 4));
        assert!(monotonic_after(4, 9));
        assert!(!monotonic_after(9, 4));
    }

    #[test]
    fn scheduler_rejects_empty_queue_and_unknown_thread() {
        let mut s = PriorityScheduler::new();
        assert_eq!(s.next(), Err(CoreError::WouldBlock));
        assert_eq!(s.set_state(1, ThreadState::Runnable), Err(CoreError::NotFound));
        assert_eq!(s.tick(1, 1), Err(CoreError::NotFound));
    }

    #[test]
    fn scheduler_tick_on_non_running_thread_fails() {
        let mut s = PriorityScheduler::new();
        let id = s.spawn(5, 3).unwrap();
        assert_eq!(s.tick(id, 1), Err(CoreError::State));
    }

    #[test]
    fn scheduler_preempts_after_quantum_exhaustion() {
        let mut s = PriorityScheduler::new();
        let a = s.spawn(5, 2).unwrap();
        let b = s.spawn(5, 2).unwrap();
        assert_eq!(s.next().unwrap(), a);
        assert!(s.tick(a, 2).unwrap());
        s.set_state(a, ThreadState::Runnable).unwrap();
        assert_eq!(s.next().unwrap(), b);
    }

    #[test]
    fn capability_table_grant_exhaustion_is_bounded() {
        let mut c = CapabilityTable::new();
        for _ in 0..MAX_CAPABILITIES {
            assert!(c.grant(0b001).is_ok());
        }
        assert_eq!(c.grant(0b001), Err(CoreError::Capacity));
    }

    #[test]
    fn capability_check_rejects_dead_capability() {
        let mut c = CapabilityTable::new();
        let cap = c.grant(0b100).unwrap();
        c.revoke(cap.id).unwrap();
        assert_eq!(c.check(cap.id, 0b100), Err(CoreError::Permission));
    }

    #[test]
    fn page_table_capacity_is_bounded() {
        let mut p = PageTable::new();
        for i in 0..MAX_MAPPINGS {
            assert!(p.map(PageMapping {
                virtual_page: (i + 1) as u64,
                frame: (i + 1) as u64,
                writable: false,
                user: false,
            }).is_ok());
        }
        assert_eq!(
            p.map(PageMapping {
                virtual_page: 999,
                frame: 999,
                writable: false,
                user: false,
            }),
            Err(CoreError::Capacity)
        );
    }

    #[test]
    fn page_table_unmap_nonexistent_page_fails() {
        let mut p = PageTable::new();
        assert_eq!(p.unmap(42), Err(CoreError::NotFound));
    }

    #[test]
    fn frame_allocator_add_range_rejects_zero_and_overflow() {
        let mut a = FrameAllocator::new();
        assert_eq!(a.add_range(10, 0), Err(CoreError::Capacity));
        assert_eq!(a.add_range(100, MAX_FRAMES + 1), Err(CoreError::Capacity));
    }

    #[test]
    fn frame_allocator_allocate_rejects_zero_owner() {
        let mut a = FrameAllocator::new();
        a.add_range(100, 1).unwrap();
        assert_eq!(a.allocate(0), Err(CoreError::Invalid));
    }

    #[test]
    fn ipc_channel_recv_on_empty_fails() {
        let mut q = IpcChannel::new(4);
        assert_eq!(q.recv(), Err(CoreError::WouldBlock));
    }

    #[test]
    fn interrupt_controller_accepts_all_u8_vectors() {
        let mut c = InterruptController::new();
        for v in 0..=u8::MAX {
            assert!(c.route(v, 0).is_ok(), "vector {v} must be routable");
        }
    }

    #[test]
    fn interrupt_controller_mask_nonexistent_route_fails() {
        let mut c = InterruptController::new();
        assert_eq!(c.mask(40, true), Err(CoreError::NotFound));
    }

    #[test]
    fn trace_buffer_sequence_wraps_without_panic() {
        let mut t = TraceBuffer::new();
        for _ in 0..(MAX_TRACE * 3) {
            t.push(1, 0, 0);
        }
        assert_eq!(t.sequence(), (MAX_TRACE * 3) as u64);
    }
}
