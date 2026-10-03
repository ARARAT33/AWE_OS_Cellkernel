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
        if !context.is_valid() {
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
        let descriptor = self.processes[index]
            .as_mut()
            .ok_or(ProcessManagerError::InvalidProcess)?;
        descriptor
            .transition(ProcessState::Runnable)
            .map_err(|_| ProcessManagerError::InvalidProcess)?;
        self.scheduler
            .enqueue(descriptor)
            .map_err(ProcessManagerError::Scheduler)
    }

    pub fn mark_running(&mut self, id: ProcessId) -> Result<(), ProcessManagerError> {
        let index = self
            .find_index(id)
            .ok_or(ProcessManagerError::InvalidProcess)?;
        let descriptor = self.processes[index]
            .as_mut()
            .ok_or(ProcessManagerError::InvalidProcess)?;
        descriptor
            .transition(ProcessState::Running)
            .map_err(|_| ProcessManagerError::InvalidProcess)
    }

    pub fn exit(&mut self, id: ProcessId) -> Result<(), ProcessManagerError> {
        let index = self
            .find_index(id)
            .ok_or(ProcessManagerError::InvalidProcess)?;
        let descriptor = self.processes[index]
            .as_mut()
            .ok_or(ProcessManagerError::InvalidProcess)?;
        descriptor
            .transition(ProcessState::Exited)
            .map_err(|_| ProcessManagerError::InvalidProcess)
    }

    pub fn next_dispatch(&mut self) -> Result<DispatchTarget, ProcessManagerError> {
        let contexts = self.context_snapshot();
        self.scheduler
            .prepare_next(&contexts)
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

    fn context_snapshot(&self) -> [ProcessContext; N] {
        core::array::from_fn(|i| {
            self.contexts[i].unwrap_or_else(|| {
                ProcessContext::new(
                    ProcessId(u64::MAX),
                    context::CpuContext::kernel_entry(1, 1, 0),
                )
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use context::CpuContext;

    fn descriptor() -> ProcessDescriptor {
        ProcessDescriptor {
            id: ProcessId(1),
            state: ProcessState::Created,
            budget: ResourceBudget {
                cpu_ticks: 10,
                memory_bytes: 4096,
                ipc_messages: 2,
            },
        }
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
        let context =
            ProcessContext::new(descriptor.id, CpuContext::kernel_entry(0x1000, 0x2000, 0));
        manager.register(descriptor, context).unwrap();
        manager.make_runnable(ProcessId(1)).unwrap();
        assert_eq!(manager.len(), 1);
        assert_eq!(manager.scheduler_ticks(), 0);
    }
}
