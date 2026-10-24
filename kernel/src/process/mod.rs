#![no_std]

pub mod context;
pub mod context_switch;
pub mod context_switch_abi;
pub mod dispatch;
pub mod dispatch_runtime;
pub mod scheduler;
pub mod x86_64_backend;

use context::ProcessContext;
use core::sync::atomic::{AtomicU64, Ordering};
use dispatch::DispatchTarget;
use scheduler::{Scheduler, SchedulerError};

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
    pub fn allocate_id(&self) -> ProcessId {
        ProcessId(self.next_id.fetch_add(1, Ordering::Relaxed))
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
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProcessManagerError {
    Full,
    DuplicateProcess,
    InvalidProcess,
    InvalidState,
    InvalidContext,
    Scheduler(SchedulerError),
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

    pub fn mark_running(&mut self, id: ProcessId) -> Result<(), ProcessManagerError> {
        let index = self
            .find_index(id)
            .ok_or(ProcessManagerError::InvalidProcess)?;
        let mut candidate = self.processes[index].ok_or(ProcessManagerError::InvalidProcess)?;
        candidate
            .transition(ProcessState::Running)
            .map_err(|_| ProcessManagerError::InvalidProcess)?;
        // This method can be called either before or after dequeue/dispatch.
        // Removing an absent entry is harmless and avoids stale queued copies.
        self.scheduler.remove(id);
        self.processes[index] = Some(candidate);
        Ok(())
    }

    pub fn exit(&mut self, id: ProcessId) -> Result<(), ProcessManagerError> {
        let index = self
            .find_index(id)
            .ok_or(ProcessManagerError::InvalidProcess)?;
        let mut candidate = self.processes[index].ok_or(ProcessManagerError::InvalidProcess)?;
        candidate
            .transition(ProcessState::Exited)
            .map_err(|_| ProcessManagerError::InvalidProcess)?;
        self.scheduler.remove(id);
        self.processes[index] = Some(candidate);
        Ok(())
    }

    pub fn next_dispatch(&mut self) -> Result<DispatchTarget, ProcessManagerError> {
        self.scheduler
            .prepare_next(&self.contexts)
            .map_err(ProcessManagerError::Scheduler)
    }

    pub fn descriptor(&self, id: ProcessId) -> Option<ProcessDescriptor> {
        self.find_index(id).and_then(|i| self.processes[i])
    }

    fn find_index(&self, id: ProcessId) -> Option<usize> {
        self.processes
            .iter()
            .position(|p| p.map(|d| d.id) == Some(id))
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

    #[test]
    fn process_ids_are_monotonic() {
        let table = ProcessTable::new();
        assert_eq!(table.allocate_id(), ProcessId(1));
        assert_eq!(table.allocate_id(), ProcessId(2));
        assert_eq!(table.allocate_id(), ProcessId(3));
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
    fn marking_running_removes_process_from_run_queue() {
        let mut manager: ProcessManager<3> = ProcessManager::new();
        manager.register(descriptor_with_id(1), context(1)).unwrap();
        manager.register(descriptor_with_id(2), context(2)).unwrap();
        manager.make_runnable(ProcessId(1)).unwrap();
        manager.make_runnable(ProcessId(2)).unwrap();

        manager.mark_running(ProcessId(1)).unwrap();

        assert_eq!(
            manager.descriptor(ProcessId(1)).map(|p| p.state),
            Some(ProcessState::Running)
        );
        assert_eq!(manager.next_dispatch().unwrap().process, ProcessId(2));
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
