#![no_std]

pub mod context;
pub mod context_switch;
pub mod context_switch_abi;
pub mod dispatch;
pub mod dispatch_runtime;
pub mod scheduler;
pub mod x86_64_backend;

#[cfg(target_arch = "x86_64")]
use crate::memory::address::is_canonical_x86_64_48;
use context::ProcessContext;
#[cfg(target_arch = "x86_64")]
use context_switch::SwitchFrame;
use core::sync::atomic::{AtomicU64, Ordering};
use dispatch::DispatchTarget;
use scheduler::{Scheduler, SchedulerError};
#[cfg(target_arch = "x86_64")]
use x86_64_backend::{BackendError, switch_checked};

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ProcessId(pub u64);

pub struct ProcessTable {
    next_id: AtomicU64,
}
impl Default for ProcessTable {
    fn default() -> Self {
        Self::new()
    }
}
impl ProcessTable {
    pub const fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
        }
    }
    /// Allocate a unique non-zero process ID.
    ///
    /// Returns `None` after the ID space is exhausted. Zero is used as the
    /// exhausted sentinel so the atomic counter never wraps back to an ID that
    /// may already have been issued.
    pub fn allocate_id(&self) -> Option<ProcessId> {
        let mut current = self.next_id.load(Ordering::Relaxed);
        loop {
            if current == 0 {
                return None;
            }

            let next = current.wrapping_add(1);
            match self.next_id.compare_exchange_weak(
                current,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Some(ProcessId(current)),
                Err(observed) => current = observed,
            }
        }
    }
}

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProcessState {
    Created = 0,
    Runnable = 1,
    Running = 2,
    Blocked = 3,
    Exited = 4,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProcessTransitionError {
    InvalidTransition,
}

impl ProcessState {
    pub const fn can_transition(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Created, Self::Runnable)
                | (Self::Created, Self::Exited)
                | (Self::Runnable, Self::Running)
                | (Self::Runnable, Self::Exited)
                | (Self::Running, Self::Runnable | Self::Blocked | Self::Exited)
                | (Self::Blocked, Self::Runnable | Self::Exited)
        )
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceBudget {
    pub cpu_ticks: u64,
    pub memory_bytes: u64,
    pub ipc_messages: u64,
}
impl ResourceBudget {
    pub const fn unlimited() -> Self {
        Self {
            cpu_ticks: u64::MAX,
            memory_bytes: u64::MAX,
            ipc_messages: u64::MAX,
        }
    }
    pub const fn permits_cpu(&self, ticks: u64) -> bool {
        ticks <= self.cpu_ticks
    }
    pub const fn permits_memory(&self, bytes: u64) -> bool {
        bytes <= self.memory_bytes
    }
    pub const fn permits_ipc(&self, messages: u64) -> bool {
        messages <= self.ipc_messages
    }
    pub fn consume_cpu(&mut self, ticks: u64) -> bool {
        if ticks > self.cpu_ticks {
            return false;
        }
        self.cpu_ticks -= ticks;
        true
    }
    pub fn consume_memory(&mut self, bytes: u64) -> bool {
        if bytes > self.memory_bytes {
            return false;
        }
        self.memory_bytes -= bytes;
        true
    }
    pub fn consume_ipc(&mut self, messages: u64) -> bool {
        if messages > self.ipc_messages {
            return false;
        }
        self.ipc_messages -= messages;
        true
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessDescriptor {
    pub id: ProcessId,
    pub state: ProcessState,
    pub budget: ResourceBudget,
}

impl ProcessDescriptor {
    pub const fn transition(&mut self, next: ProcessState) -> Result<(), ProcessTransitionError> {
        if self.state.can_transition(next) {
            self.state = next;
            Ok(())
        } else {
            Err(ProcessTransitionError::InvalidTransition)
        }
    }
}

/// Fixed-capacity runtime process registry. The registry owns process metadata,
/// while CPU contexts remain in a parallel fixed-capacity array so the scheduler
/// can select real execution contexts without heap allocation.
pub struct ProcessManager<const N: usize> {
    processes: [Option<ProcessDescriptor>; N],
    contexts: [Option<ProcessContext>; N],
    count: usize,
    next_slot: usize,
    scheduler: Scheduler<N>,
    // A dequeued target remains pending until the architecture completion
    // path acknowledges that exact frame, or explicitly cancels the switch.
    pending_dispatch: Option<DispatchTarget>,
    // Exited contexts cannot be reclaimed until execution has switched away.
    current: Option<ProcessId>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProcessManagerError {
    Full,
    DuplicateProcess,
    InvalidProcess,
    InvalidState,
    InvalidContext,
    NotExited,
    CurrentlyRunning,
    StillQueued,
    DispatchPending,
    NoPendingDispatch,
    DispatchMismatch,
    Scheduler(SchedulerError),
}

#[cfg(target_arch = "x86_64")]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DispatchSwitchError {
    NullManager,
    NullCurrentFrame,
    InvalidManagerPointer,
    InvalidFramePointer,
    DispatchMismatch,
    InvalidCompletion,
    Backend(BackendError),
    CancelFailed(ProcessManagerError),
}

impl<const N: usize> Default for ProcessManager<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> ProcessManager<N> {
    pub const fn new() -> Self {
        Self {
            processes: [None; N],
            contexts: [None; N],
            count: 0,
            next_slot: 0,
            scheduler: Scheduler::new(),
            pending_dispatch: None,
            current: None,
        }
    }

    pub const fn len(&self) -> usize {
        self.count
    }

    pub const fn scheduler_ticks(&self) -> u64 {
        self.scheduler.ticks()
    }

    pub fn register(
        &mut self,
        descriptor: ProcessDescriptor,
        context: ProcessContext,
    ) -> Result<usize, ProcessManagerError> {
        if self
            .processes
            .iter()
            .flatten()
            .any(|p| p.id == descriptor.id)
        {
            return Err(ProcessManagerError::DuplicateProcess);
        }
        // The descriptor and execution context must identify the same process.
        // Otherwise it could be queued under one ID but never find its context
        // during dispatch, silently consuming a scheduler slot.
        // ID zero is reserved for "no process" in runtime contracts.
        if descriptor.id.0 == 0 {
            return Err(ProcessManagerError::InvalidProcess);
        }
        // A newly registered process must enter the lifecycle through Created.
        if descriptor.state != ProcessState::Created {
            return Err(ProcessManagerError::InvalidState);
        }
        if context.process_id != descriptor.id || !context.is_valid() {
            return Err(ProcessManagerError::InvalidContext);
        }
        let slot = (self.next_slot..N)
            .chain(0..self.next_slot)
            .find(|&i| self.processes[i].is_none())
            .ok_or(ProcessManagerError::Full)?;
        self.processes[slot] = Some(descriptor);
        self.contexts[slot] = Some(context);
        self.next_slot = if N == 0 { 0 } else { (slot + 1) % N };
        self.count += 1;
        Ok(slot)
    }

    pub fn make_runnable(&mut self, id: ProcessId) -> Result<(), ProcessManagerError> {
        let index = self
            .find_index(id)
            .ok_or(ProcessManagerError::InvalidProcess)?;
        // Work on a copy first: a scheduler error must not leave the registry
        // claiming the process is runnable when it was not queued.
        let mut candidate = self.processes[index].ok_or(ProcessManagerError::InvalidProcess)?;
        candidate
            .transition(ProcessState::Runnable)
            .map_err(|_| ProcessManagerError::InvalidProcess)?;
        self.scheduler
            .enqueue(&candidate)
            .map_err(ProcessManagerError::Scheduler)?;
        self.processes[index] = Some(candidate);
        Ok(())
    }

    /// Acknowledge the exact scheduler target after the architecture switch has
    /// completed and execution is on that target's stack.
    ///
    /// The token must come from the immediately preceding `next_dispatch`.
    /// This prevents arbitrary runnable ProcessIds or stale frames from being
    /// published as the current task. It is still the architecture layer's
    /// responsibility to call this only from a true switch-completion path.
    pub fn confirm_dispatch(&mut self, target: DispatchTarget) -> Result<(), ProcessManagerError> {
        let pending = self
            .pending_dispatch
            .ok_or(ProcessManagerError::NoPendingDispatch)?;
        if pending != target {
            return Err(ProcessManagerError::DispatchMismatch);
        }

        self.commit_running(target.process)?;
        self.pending_dispatch = None;
        Ok(())
    }

    fn confirm_pending_dispatch_for(
        &mut self,
        process: ProcessId,
    ) -> Result<(), ProcessManagerError> {
        let pending = self
            .pending_dispatch
            .ok_or(ProcessManagerError::NoPendingDispatch)?;
        if pending.process != process {
            return Err(ProcessManagerError::DispatchMismatch);
        }
        self.confirm_dispatch(pending)
    }

    /// Cancel a prepared switch when the architecture backend declines it.
    /// The selected runnable process is returned to the queue before the
    /// pending token is cleared.
    pub fn cancel_dispatch(&mut self, target: DispatchTarget) -> Result<(), ProcessManagerError> {
        let pending = self
            .pending_dispatch
            .ok_or(ProcessManagerError::NoPendingDispatch)?;
        if pending != target {
            return Err(ProcessManagerError::DispatchMismatch);
        }
        let index = self
            .find_index(target.process)
            .ok_or(ProcessManagerError::InvalidProcess)?;
        let descriptor = self.processes[index].ok_or(ProcessManagerError::InvalidProcess)?;
        if descriptor.state != ProcessState::Runnable {
            return Err(ProcessManagerError::InvalidState);
        }
        self.scheduler
            .enqueue(&descriptor)
            .map_err(ProcessManagerError::Scheduler)?;
        self.pending_dispatch = None;
        Ok(())
    }

    fn commit_running(&mut self, id: ProcessId) -> Result<(), ProcessManagerError> {
        let index = self
            .find_index(id)
            .ok_or(ProcessManagerError::InvalidProcess)?;
        let mut candidate = self.processes[index].ok_or(ProcessManagerError::InvalidProcess)?;
        candidate
            .transition(ProcessState::Running)
            .map_err(|_| ProcessManagerError::InvalidProcess)?;

        // A single-core manager may track only one running process. Once the
        // selected context has actually begun running, return the previous
        // running process to the runnable queue before publishing the switch.
        let previous = self.current;
        let mut previous_update = None;
        if let Some(previous_id) = previous.filter(|previous_id| *previous_id != id) {
            let previous_index = self
                .find_index(previous_id)
                .ok_or(ProcessManagerError::InvalidProcess)?;
            let mut previous_candidate =
                self.processes[previous_index].ok_or(ProcessManagerError::InvalidProcess)?;
            if previous_candidate.state == ProcessState::Running {
                previous_candidate
                    .transition(ProcessState::Runnable)
                    .map_err(|_| ProcessManagerError::InvalidProcess)?;
                self.scheduler
                    .enqueue(&previous_candidate)
                    .map_err(ProcessManagerError::Scheduler)?;
                previous_update = Some((previous_index, previous_candidate));
            }
        }

        self.scheduler.remove(id);
        if let Some((previous_index, previous_candidate)) = previous_update {
            self.processes[previous_index] = Some(previous_candidate);
        }
        self.processes[index] = Some(candidate);
        self.current = Some(id);
        Ok(())
    }

    pub fn exit(&mut self, id: ProcessId) -> Result<(), ProcessManagerError> {
        if self
            .pending_dispatch
            .is_some_and(|target| target.process == id)
        {
            return Err(ProcessManagerError::DispatchPending);
        }
        let index = self
            .find_index(id)
            .ok_or(ProcessManagerError::InvalidProcess)?;
        let mut candidate = self.processes[index].ok_or(ProcessManagerError::InvalidProcess)?;
        candidate
            .transition(ProcessState::Exited)
            .map_err(|_| ProcessManagerError::InvalidProcess)?;
        self.scheduler.remove(id);
        self.processes[index] = Some(candidate);
        // Keep current set to an exiting process until the architecture layer
        // confirms that execution has switched away from it.
        Ok(())
    }

    /// Reclaim an exited process only after it is no longer current or queued.
    /// This releases both the descriptor and its CPU context as one operation.
    pub fn reap(&mut self, id: ProcessId) -> Result<(), ProcessManagerError> {
        if self
            .pending_dispatch
            .is_some_and(|target| target.process == id)
        {
            return Err(ProcessManagerError::DispatchPending);
        }
        let index = self
            .find_index(id)
            .ok_or(ProcessManagerError::InvalidProcess)?;
        let descriptor = self.processes[index].ok_or(ProcessManagerError::InvalidProcess)?;

        if descriptor.state != ProcessState::Exited {
            return Err(ProcessManagerError::NotExited);
        }
        if self.current == Some(id) {
            return Err(ProcessManagerError::CurrentlyRunning);
        }
        if self.scheduler.contains(id) {
            return Err(ProcessManagerError::StillQueued);
        }

        self.processes[index] = None;
        self.contexts[index] = None;
        self.count -= 1;
        self.next_slot = index;
        Ok(())
    }

    /// Confirm a switch to idle for the exact current process, and only after
    /// that process has exited. A stale or premature acknowledgement cannot
    /// clear another task's current-process identity.
    pub fn confirm_switched_to_idle(
        &mut self,
        expected_current: ProcessId,
    ) -> Result<(), ProcessManagerError> {
        if self.pending_dispatch.is_some() {
            return Err(ProcessManagerError::DispatchPending);
        }
        if self.current != Some(expected_current) {
            return Err(ProcessManagerError::InvalidProcess);
        }
        let descriptor = self
            .descriptor(expected_current)
            .ok_or(ProcessManagerError::InvalidProcess)?;
        if descriptor.state != ProcessState::Exited {
            return Err(ProcessManagerError::InvalidState);
        }
        self.current = None;
        Ok(())
    }

    /// Dequeue and retain one exact dispatch target. A second dequeue is
    /// rejected until the pending switch is acknowledged or cancelled.
    pub fn next_dispatch(&mut self) -> Result<DispatchTarget, ProcessManagerError> {
        if self.pending_dispatch.is_some() {
            return Err(ProcessManagerError::DispatchPending);
        }
        let target = self
            .scheduler
            .prepare_next(&self.contexts)
            .map_err(ProcessManagerError::Scheduler)?;
        self.pending_dispatch = Some(target);
        Ok(target)
    }

    pub fn descriptor(&self, id: ProcessId) -> Option<ProcessDescriptor> {
        self.find_index(id).and_then(|i| self.processes[i])
    }

    /// Return the kernel-tracked current process only while its descriptor is
    /// still Running. The identity is established by the trusted context
    /// switch path, not supplied by a syscall argument.
    pub(crate) fn current_process_id(&self) -> Option<ProcessId> {
        let id = self.current?;
        self.descriptor(id)
            .filter(|descriptor| descriptor.state == ProcessState::Running)
            .map(|descriptor| descriptor.id)
    }

    fn find_index(&self, id: ProcessId) -> Option<usize> {
        self.processes
            .iter()
            .position(|p| p.map(|d| d.id) == Some(id))
    }
}

/// Confirm a dispatch from the target stack, after the architecture backend
/// has installed that stack and before it jumps to the process RIP.
#[cfg(target_arch = "x86_64")]
unsafe extern "C" fn confirm_pending_dispatch_on_target<const N: usize>(
    manager_address: u64,
    process_id: u64,
) {
    if manager_address == 0 {
        unsafe { crate::arch::x86_64::boot::halt() };
    }
    let manager = unsafe { &mut *(manager_address as *mut ProcessManager<N>) };
    if manager
        .confirm_pending_dispatch_for(ProcessId(process_id))
        .is_err()
    {
        // Current-task identity is security-sensitive. Never run the target
        // if its exact pending dispatch cannot be committed.
        unsafe { crate::arch::x86_64::boot::halt() };
    }
}

/// Perform a validated x86_64 switch and acknowledge the exact pending target
/// on that target's stack.
///
/// # Safety
/// - manager must point to a pinned ProcessManager in shared kernel memory.
/// - current must point to writable, pinned SwitchFrame storage.
/// - The caller must have exclusive manager access and keep interrupts disabled
///   for the whole switch; this implementation is single-core only.
/// - Both stacks and the callback context must remain mapped in the kernel
///   address space. This is not a Ring-3 isolation or SMP mechanism.
#[cfg(target_arch = "x86_64")]
pub unsafe fn switch_to_dispatch<const N: usize>(
    manager: *mut ProcessManager<N>,
    current: *mut SwitchFrame,
    target: DispatchTarget,
) -> Result<(), DispatchSwitchError> {
    if manager.is_null() {
        return Err(DispatchSwitchError::NullManager);
    }
    if current.is_null() {
        return Err(DispatchSwitchError::NullCurrentFrame);
    }

    let manager_address = manager as usize as u64;
    let frame_address = current as usize as u64;
    if !is_canonical_x86_64_48(manager_address)
        || manager_address & (core::mem::align_of::<ProcessManager<N>>() as u64 - 1) != 0
    {
        return Err(DispatchSwitchError::InvalidManagerPointer);
    }
    if !is_canonical_x86_64_48(frame_address)
        || frame_address & (core::mem::align_of::<SwitchFrame>() as u64 - 1) != 0
    {
        return Err(DispatchSwitchError::InvalidFramePointer);
    }

    let exact_pending_target = unsafe { (*manager).pending_dispatch == Some(target) };
    if !exact_pending_target {
        return Err(DispatchSwitchError::DispatchMismatch);
    }

    let next = match unsafe {
        target.frame.with_completion(
            confirm_pending_dispatch_on_target::<N>,
            manager_address,
            target.process.0,
        )
    } {
        Ok(frame) => frame,
        Err(_) => {
            return match unsafe { (&mut *manager).cancel_dispatch(target) } {
                Ok(()) => Err(DispatchSwitchError::InvalidCompletion),
                Err(error) => Err(DispatchSwitchError::CancelFailed(error)),
            };
        }
    };

    match unsafe { switch_checked(&mut *current, &next) } {
        Ok(()) => Ok(()),
        Err(error) => match unsafe { (&mut *manager).cancel_dispatch(target) } {
            Ok(()) => Err(DispatchSwitchError::Backend(error)),
            Err(cancel_error) => Err(DispatchSwitchError::CancelFailed(cancel_error)),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use context::CpuContext;

    fn descriptor() -> ProcessDescriptor {
        descriptor_with_id(1)
    }

    fn descriptor_with_id(id: u64) -> ProcessDescriptor {
        ProcessDescriptor {
            id: ProcessId(id),
            state: ProcessState::Created,
            budget: ResourceBudget {
                cpu_ticks: 10,
                memory_bytes: 4096,
                ipc_messages: 2,
            },
        }
    }

    fn context(id: u64) -> ProcessContext {
        ProcessContext::new(ProcessId(id), CpuContext::kernel_entry(0x1000, 0x2000, 0))
    }

    fn dispatch_and_confirm<const N: usize>(manager: &mut ProcessManager<N>, expected: ProcessId) {
        let target = manager.next_dispatch().unwrap();
        assert_eq!(target.process, expected);
        manager.confirm_dispatch(target).unwrap();
    }

    #[test]
    fn process_ids_are_monotonic() {
        let table = ProcessTable::new();
        assert_eq!(table.allocate_id(), Some(ProcessId(1)));
        assert_eq!(table.allocate_id(), Some(ProcessId(2)));
        assert_eq!(table.allocate_id(), Some(ProcessId(3)));
    }

    #[test]
    fn process_id_allocator_stops_at_u64_exhaustion_without_wrapping() {
        let table = ProcessTable {
            next_id: AtomicU64::new(u64::MAX - 1),
        };

        assert_eq!(table.allocate_id(), Some(ProcessId(u64::MAX - 1)));
        assert_eq!(table.allocate_id(), Some(ProcessId(u64::MAX)));
        assert_eq!(table.allocate_id(), None);
        assert_eq!(table.allocate_id(), None);
        assert_eq!(table.next_id.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn lifecycle_accepts_only_valid_transitions() {
        let mut p = descriptor();
        assert!(p.transition(ProcessState::Runnable).is_ok());
        assert!(p.transition(ProcessState::Running).is_ok());
        assert!(p.transition(ProcessState::Blocked).is_ok());
        assert!(p.transition(ProcessState::Runnable).is_ok());
        assert!(p.transition(ProcessState::Exited).is_ok());
        assert_eq!(
            p.transition(ProcessState::Running),
            Err(ProcessTransitionError::InvalidTransition)
        );
    }

    #[test]
    fn lifecycle_rejects_created_to_running() {
        let mut p = descriptor();
        assert_eq!(
            p.transition(ProcessState::Running),
            Err(ProcessTransitionError::InvalidTransition)
        );
    }

    #[test]
    fn budget_consumption_is_atomic_on_failure() {
        let mut b = ResourceBudget {
            cpu_ticks: 10,
            memory_bytes: 4096,
            ipc_messages: 2,
        };
        assert!(b.consume_cpu(4));
        assert_eq!(b.cpu_ticks, 6);
        assert!(!b.consume_cpu(7));
        assert_eq!(b.cpu_ticks, 6);
        assert!(b.consume_memory(1024));
        assert_eq!(b.memory_bytes, 3072);
        assert!(!b.consume_memory(4096));
        assert_eq!(b.memory_bytes, 3072);
        assert!(b.consume_ipc(2));
        assert_eq!(b.ipc_messages, 0);
        assert!(!b.consume_ipc(1));
        assert_eq!(b.ipc_messages, 0);
    }

    #[test]
    fn budget_permission_checks_are_non_mutating() {
        let b = ResourceBudget {
            cpu_ticks: 10,
            memory_bytes: 4096,
            ipc_messages: 2,
        };
        assert!(b.permits_cpu(10));
        assert!(!b.permits_cpu(11));
        assert!(b.permits_memory(4096));
        assert!(!b.permits_memory(4097));
        assert!(b.permits_ipc(2));
        assert!(!b.permits_ipc(3));
    }

    #[test]
    fn process_manager_registers_and_enqueues_real_contexts() {
        let mut manager: ProcessManager<4> = ProcessManager::new();
        let descriptor = descriptor();
        let process_context = context(descriptor.id.0);
        manager.register(descriptor, process_context).unwrap();
        manager.make_runnable(ProcessId(1)).unwrap();
        assert_eq!(manager.len(), 1);
        assert_eq!(manager.scheduler_ticks(), 0);
    }

    #[test]
    fn registration_rejects_mismatched_context_without_consuming_a_slot() {
        let mut manager: ProcessManager<1> = ProcessManager::new();

        assert_eq!(
            manager.register(descriptor_with_id(1), context(2)),
            Err(ProcessManagerError::InvalidContext)
        );
        assert_eq!(manager.len(), 0);
        assert_eq!(manager.descriptor(ProcessId(1)), None);

        // A rejected registration must leave the slot available.
        assert_eq!(manager.register(descriptor_with_id(1), context(1)), Ok(0));
        assert_eq!(manager.len(), 1);
    }

    #[test]
    fn registration_rejects_non_created_state_without_consuming_a_slot() {
        let mut manager: ProcessManager<1> = ProcessManager::new();
        let mut not_created = descriptor_with_id(1);
        not_created.state = ProcessState::Runnable;

        assert_eq!(
            manager.register(not_created, context(1)),
            Err(ProcessManagerError::InvalidState)
        );
        assert_eq!(manager.len(), 0);
        assert_eq!(manager.register(descriptor_with_id(1), context(1)), Ok(0));
    }

    #[test]
    fn registration_rejects_reserved_zero_process_id() {
        let mut manager: ProcessManager<1> = ProcessManager::new();

        assert_eq!(
            manager.register(descriptor_with_id(0), context(0)),
            Err(ProcessManagerError::InvalidProcess)
        );
        assert_eq!(manager.len(), 0);
    }

    #[test]
    fn current_process_identity_requires_acknowledged_dispatch_and_exit() {
        let mut manager: ProcessManager<1> = ProcessManager::new();
        manager.register(descriptor_with_id(1), context(1)).unwrap();
        assert_eq!(manager.current_process_id(), None);

        manager.make_runnable(ProcessId(1)).unwrap();
        assert_eq!(manager.current_process_id(), None);
        assert_eq!(
            manager.confirm_switched_to_idle(ProcessId(1)),
            Err(ProcessManagerError::InvalidProcess)
        );

        dispatch_and_confirm(&mut manager, ProcessId(1));
        assert_eq!(manager.current_process_id(), Some(ProcessId(1)));
        assert_eq!(
            manager.confirm_switched_to_idle(ProcessId(1)),
            Err(ProcessManagerError::InvalidState)
        );
        assert_eq!(manager.current_process_id(), Some(ProcessId(1)));

        manager.exit(ProcessId(1)).unwrap();
        assert_eq!(manager.current_process_id(), None);
        manager.confirm_switched_to_idle(ProcessId(1)).unwrap();
        assert_eq!(manager.current_process_id(), None);
    }

    #[test]
    fn acknowledged_dispatch_updates_running_state_and_round_robin_queue() {
        let mut manager: ProcessManager<3> = ProcessManager::new();
        manager.register(descriptor_with_id(1), context(1)).unwrap();
        manager.register(descriptor_with_id(2), context(2)).unwrap();
        manager.make_runnable(ProcessId(1)).unwrap();
        manager.make_runnable(ProcessId(2)).unwrap();

        dispatch_and_confirm(&mut manager, ProcessId(1));
        assert_eq!(
            manager.descriptor(ProcessId(1)).map(|p| p.state),
            Some(ProcessState::Running)
        );

        dispatch_and_confirm(&mut manager, ProcessId(2));
        assert_eq!(
            manager.descriptor(ProcessId(1)).map(|p| p.state),
            Some(ProcessState::Runnable)
        );
        let target = manager.next_dispatch().unwrap();
        assert_eq!(target.process, ProcessId(1));
        manager.cancel_dispatch(target).unwrap();
    }

    #[test]
    fn dispatch_acknowledgement_is_exact_single_use_and_cancellable() {
        let mut manager: ProcessManager<2> = ProcessManager::new();
        manager.register(descriptor_with_id(1), context(1)).unwrap();
        manager.register(descriptor_with_id(2), context(2)).unwrap();
        manager.make_runnable(ProcessId(1)).unwrap();
        manager.make_runnable(ProcessId(2)).unwrap();

        let target = manager.next_dispatch().unwrap();
        assert_eq!(target.process, ProcessId(1));
        assert_eq!(
            manager.next_dispatch(),
            Err(ProcessManagerError::DispatchPending)
        );

        let mismatched = DispatchTarget {
            process: ProcessId(2),
            frame: target.frame,
        };
        assert_eq!(
            manager.confirm_dispatch(mismatched),
            Err(ProcessManagerError::DispatchMismatch)
        );
        assert_eq!(manager.current_process_id(), None);

        manager.cancel_dispatch(target).unwrap();

        // Cancellation puts the target at the queue tail, preserving FIFO
        // fairness for processes that were already runnable.
        let other = manager.next_dispatch().unwrap();
        assert_eq!(other.process, ProcessId(2));
        manager.confirm_dispatch(other).unwrap();

        let retried = manager.next_dispatch().unwrap();
        assert_eq!(retried.process, ProcessId(1));
        manager.confirm_dispatch(retried).unwrap();
        assert_eq!(
            manager.confirm_dispatch(retried),
            Err(ProcessManagerError::NoPendingDispatch)
        );
        assert_eq!(manager.current_process_id(), Some(ProcessId(1)));
    }

    #[test]
    fn exit_cannot_reclaim_or_cancel_a_pending_dispatch_implicitly() {
        let mut manager: ProcessManager<1> = ProcessManager::new();
        manager.register(descriptor_with_id(1), context(1)).unwrap();
        manager.make_runnable(ProcessId(1)).unwrap();
        let target = manager.next_dispatch().unwrap();

        assert_eq!(
            manager.exit(ProcessId(1)),
            Err(ProcessManagerError::DispatchPending)
        );
        assert_eq!(
            manager.reap(ProcessId(1)),
            Err(ProcessManagerError::DispatchPending)
        );

        manager.cancel_dispatch(target).unwrap();
        manager.exit(ProcessId(1)).unwrap();
        assert_eq!(
            manager.confirm_switched_to_idle(ProcessId(1)),
            Err(ProcessManagerError::InvalidProcess)
        );
    }

    #[test]
    fn reap_requires_exit_and_switch_away_then_reuses_slot_without_stale_context() {
        let mut manager: ProcessManager<1> = ProcessManager::new();
        manager.register(descriptor_with_id(1), context(1)).unwrap();

        assert_eq!(
            manager.reap(ProcessId(1)),
            Err(ProcessManagerError::NotExited)
        );
        manager.make_runnable(ProcessId(1)).unwrap();
        dispatch_and_confirm(&mut manager, ProcessId(1));
        manager.exit(ProcessId(1)).unwrap();

        assert_eq!(
            manager.reap(ProcessId(1)),
            Err(ProcessManagerError::CurrentlyRunning)
        );
        assert_eq!(manager.len(), 1);

        manager.confirm_switched_to_idle(ProcessId(1)).unwrap();
        assert_eq!(manager.reap(ProcessId(1)), Ok(()));
        assert_eq!(manager.len(), 0);
        assert_eq!(manager.descriptor(ProcessId(1)), None);

        assert_eq!(manager.register(descriptor_with_id(2), context(2)), Ok(0));
        manager.make_runnable(ProcessId(2)).unwrap();
        let target = manager.next_dispatch().unwrap();
        assert_eq!(target.process, ProcessId(2));
        manager.cancel_dispatch(target).unwrap();
    }

    #[test]
    fn exiting_process_skips_it_in_future_dispatches() {
        let mut manager: ProcessManager<3> = ProcessManager::new();
        manager.register(descriptor_with_id(1), context(1)).unwrap();
        manager.register(descriptor_with_id(2), context(2)).unwrap();
        manager.make_runnable(ProcessId(1)).unwrap();
        manager.make_runnable(ProcessId(2)).unwrap();

        manager.exit(ProcessId(1)).unwrap();

        assert_eq!(
            manager.descriptor(ProcessId(1)).map(|p| p.state),
            Some(ProcessState::Exited)
        );
        assert_eq!(manager.next_dispatch().unwrap().process, ProcessId(2));
    }
}
