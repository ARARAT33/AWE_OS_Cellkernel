#![no_std]

use crate::process::{
    ProcessDescriptor, ProcessId, ProcessManager, ProcessManagerError, ProcessState, ProcessTable,
    ResourceBudget, context::CpuContext, context_switch::SwitchFrame, dispatch::DispatchTarget,
    scheduler::SchedulerError,
};

/// Errors exposed by the execution-core compatibility facade.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CoreError {
    ProcessTableFull,
    ProcessIdExhausted,
    RunQueueFull,
    InvalidTransition,
    InvalidContext,
    BudgetExceeded,
    NoRunnable,
    DispatchPending,
    DispatchMismatch,
    NotExited,
    CurrentlyRunning,
    StillScheduled,
}

/// Trace record for execution-core event diagnostics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CoreTraceRecord {
    pub process: ProcessId,
    pub event_kind: u16,
    pub value: u64,
}

/// Compatibility facade over the authoritative ProcessManager.
///
/// This type deliberately owns no process table, scheduler, or current-task
/// field of its own. Process descriptors, execution contexts, queue membership,
/// pending dispatch and current-process identity all live in ProcessManager.
pub struct ExecutionCore<const N: usize> {
    processes: ProcessManager<N>,
    ids: ProcessTable,
    trace_records: [Option<CoreTraceRecord>; 32],
    trace_count: usize,
}

impl<const N: usize> Default for ExecutionCore<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> ExecutionCore<N> {
    pub const fn new() -> Self {
        Self {
            processes: ProcessManager::new(),
            ids: ProcessTable::new(),
            trace_records: [None; 32],
            trace_count: 0,
        }
    }

    /// Register a process with an explicit, validated kernel CPU context.
    /// A metadata-only process with no executable context is no longer admitted.
    pub fn create_process(
        &mut self,
        budget: ResourceBudget,
        cpu: CpuContext,
    ) -> Result<ProcessId, CoreError> {
        if !cpu.validate() {
            return Err(CoreError::InvalidContext);
        }
        let id = self
            .ids
            .allocate_id()
            .ok_or(CoreError::ProcessIdExhausted)?;
        let descriptor = ProcessDescriptor {
            id,
            state: ProcessState::Created,
            budget,
        };
        let context = crate::process::context::ProcessContext::new(id, cpu);
        self.processes
            .register(descriptor, context)
            .map_err(map_process_error)?;
        if let Err(error) = self.processes.make_runnable(id) {
            // Registration is rolled back as a normal Created -> Exited ->
            // reaped lifecycle. The monotonic ID is intentionally not reused.
            let _ = self.processes.exit(id);
            let _ = self.processes.reap(id);
            return Err(map_process_error(error));
        }
        self.record_trace(id, 1, budget.cpu_ticks);
        Ok(id)
    }

    /// Select the next dispatch target. If a process is currently running,
    /// it is first yielded into the same ProcessManager-owned run queue.
    /// The returned token must be confirmed only by the architecture completion
    /// path after the target stack is active.
    pub fn schedule(&mut self) -> Result<DispatchTarget, CoreError> {
        if let Some(current) = self.processes.current_process_id() {
            self.processes
                .yield_process(current)
                .map_err(map_process_error)?;
        }
        let target = self.processes.next_dispatch().map_err(map_process_error)?;
        self.record_trace(target.process, 7, 0);
        Ok(target)
    }

    pub fn confirm_dispatch(&mut self, target: DispatchTarget) -> Result<(), CoreError> {
        self.processes
            .confirm_dispatch(target)
            .map_err(map_process_error)?;
        self.record_trace(target.process, 8, 0);
        Ok(())
    }

    pub fn cancel_dispatch(&mut self, target: DispatchTarget) -> Result<(), CoreError> {
        self.processes
            .cancel_dispatch(target)
            .map_err(map_process_error)
    }

    pub const fn pending_dispatch(&self) -> Option<DispatchTarget> {
        self.processes.pending_dispatch_target()
    }

    pub fn yield_process(&mut self, id: ProcessId) -> Result<(), CoreError> {
        self.processes
            .yield_process(id)
            .map_err(map_process_error)?;
        self.record_trace(id, 4, 0);
        Ok(())
    }

    pub fn block_process(&mut self, id: ProcessId) -> Result<(), CoreError> {
        self.processes
            .block_process(id)
            .map_err(map_process_error)?;
        self.record_trace(id, 2, 0);
        Ok(())
    }

    pub fn unblock_process(&mut self, id: ProcessId) -> Result<(), CoreError> {
        self.processes
            .unblock_process(id)
            .map_err(map_process_error)?;
        self.record_trace(id, 3, 0);
        Ok(())
    }

    pub fn consume_cpu(&mut self, id: ProcessId, ticks: u64) -> Result<(), CoreError> {
        self.processes
            .consume_cpu(id, ticks)
            .map_err(map_process_error)?;
        self.record_trace(id, 5, ticks);
        Ok(())
    }

    pub fn exit(&mut self, id: ProcessId) -> Result<(), CoreError> {
        self.processes.exit(id).map_err(map_process_error)?;
        self.record_trace(id, 6, 0);
        Ok(())
    }

    /// Acknowledge a completed switch to idle for the current blocked/exited task.
    pub fn confirm_idle(&mut self, id: ProcessId) -> Result<(), CoreError> {
        let descriptor = self
            .processes
            .descriptor(id)
            .ok_or(CoreError::InvalidTransition)?;
        match descriptor.state {
            ProcessState::Exited => self
                .processes
                .confirm_switched_to_idle(id)
                .map_err(map_process_error),
            ProcessState::Blocked => self
                .processes
                .confirm_blocked_to_idle(id)
                .map_err(map_process_error),
            _ => Err(CoreError::InvalidTransition),
        }
    }

    pub fn reap(&mut self, id: ProcessId) -> Result<(), CoreError> {
        self.processes.reap(id).map_err(map_process_error)
    }

    pub const fn current(&self) -> Option<ProcessId> {
        self.processes.current_process_id()
    }

    pub fn descriptor(&self, id: ProcessId) -> Option<ProcessDescriptor> {
        self.processes.descriptor(id)
    }

    pub fn process_context(
        &self,
        id: ProcessId,
    ) -> Option<crate::process::context::ProcessContext> {
        self.processes.process_context(id)
    }

    pub fn trace_records(&self) -> &[Option<CoreTraceRecord>] {
        &self.trace_records[..self.trace_count]
    }

    fn record_trace(&mut self, process: ProcessId, event_kind: u16, value: u64) {
        if self.trace_count < self.trace_records.len() {
            self.trace_records[self.trace_count] = Some(CoreTraceRecord {
                process,
                event_kind,
                value,
            });
            self.trace_count += 1;
        }
    }
}

fn map_process_error(error: ProcessManagerError) -> CoreError {
    match error {
        ProcessManagerError::Full => CoreError::ProcessTableFull,
        ProcessManagerError::InvalidContext => CoreError::InvalidContext,
        ProcessManagerError::NotExited => CoreError::NotExited,
        ProcessManagerError::CurrentlyRunning => CoreError::CurrentlyRunning,
        ProcessManagerError::StillQueued => CoreError::StillScheduled,
        ProcessManagerError::DispatchPending => CoreError::DispatchPending,
        ProcessManagerError::NoPendingDispatch | ProcessManagerError::DispatchMismatch => {
            CoreError::DispatchMismatch
        }
        ProcessManagerError::BudgetExceeded => CoreError::BudgetExceeded,
        ProcessManagerError::Scheduler(SchedulerError::QueueFull) => CoreError::RunQueueFull,
        ProcessManagerError::Scheduler(SchedulerError::Empty) => CoreError::NoRunnable,
        ProcessManagerError::Scheduler(
            SchedulerError::InvalidContext | SchedulerError::UnknownProcess,
        ) => CoreError::InvalidContext,
        ProcessManagerError::Scheduler(_) => CoreError::InvalidTransition,
        ProcessManagerError::DuplicateProcess
        | ProcessManagerError::InvalidProcess
        | ProcessManagerError::InvalidState => CoreError::InvalidTransition,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget() -> ResourceBudget {
        ResourceBudget {
            cpu_ticks: 4,
            memory_bytes: 4096,
            ipc_messages: 2,
        }
    }

    fn cpu(entry: u64, stack: u64) -> CpuContext {
        CpuContext::kernel_entry(entry, stack, 0)
    }

    fn create(core: &mut ExecutionCore<3>, entry: u64, stack: u64) -> ProcessId {
        core.create_process(budget(), cpu(entry, stack)).unwrap()
    }

    #[test]
    fn process_id_exhaustion_does_not_reissue_max_id() {
        let table = ProcessTable {
            next_id: core::sync::atomic::AtomicU64::new(u64::MAX - 1),
        };
        assert_eq!(table.allocate_id(), Some(ProcessId(u64::MAX - 1)));
        assert_eq!(table.allocate_id(), Some(ProcessId(u64::MAX)));
        assert_eq!(table.allocate_id(), None);
        assert_eq!(table.allocate_id(), None);
    }

    #[test]
    fn capacity_and_context_validation_fail_closed() {
        let mut core: ExecutionCore<1> = ExecutionCore::new();
        let id = core.create_process(budget(), cpu(0x1000, 0x2000)).unwrap();
        assert_eq!(
            core.create_process(budget(), cpu(0x1100, 0x3000)),
            Err(CoreError::ProcessTableFull)
        );
        assert_eq!(
            core.create_process(budget(), cpu(0x0000_8000_0000_0000, 0x3000)),
            Err(CoreError::InvalidContext)
        );
        let target = core.schedule().unwrap();
        assert_eq!(target.process, id);
        core.confirm_dispatch(target).unwrap();
        assert_eq!(core.consume_cpu(id, 4), Ok(()));
        assert_eq!(core.consume_cpu(id, 1), Err(CoreError::BudgetExceeded));
    }

    #[test]
    fn current_identity_is_published_only_after_exact_dispatch_acknowledgement() {
        let mut core: ExecutionCore<3> = ExecutionCore::new();
        let first = create(&mut core, 0x1000, 0x2000);
        let second = create(&mut core, 0x1100, 0x3000);

        let first_target = core.schedule().unwrap();
        assert_eq!(first_target.process, first);
        assert_eq!(core.current(), None);
        assert_eq!(core.pending_dispatch(), Some(first_target));
        assert_eq!(core.schedule(), Err(CoreError::DispatchPending));

        core.confirm_dispatch(first_target).unwrap();
        assert_eq!(core.current(), Some(first));

        let second_target = core.schedule().unwrap();
        assert_eq!(second_target.process, second);
        assert_eq!(core.current(), None);
        core.confirm_dispatch(second_target).unwrap();
        assert_eq!(core.current(), Some(second));
    }

    #[test]
    fn block_unblock_and_round_robin_share_one_queue() {
        let mut core: ExecutionCore<3> = ExecutionCore::new();
        let first = create(&mut core, 0x1000, 0x2000);
        let second = create(&mut core, 0x1100, 0x3000);

        let target = core.schedule().unwrap();
        assert_eq!(target.process, first);
        core.confirm_dispatch(target).unwrap();
        core.block_process(first).unwrap();

        let target = core.schedule().unwrap();
        assert_eq!(target.process, second);
        core.confirm_dispatch(target).unwrap();

        core.unblock_process(first).unwrap();
        let target = core.schedule().unwrap();
        assert_eq!(target.process, first);
        core.confirm_dispatch(target).unwrap();
        assert_eq!(core.current(), Some(first));
    }

    #[test]
    fn exited_process_cannot_be_reaped_until_idle_or_next_dispatch_is_confirmed() {
        let mut core: ExecutionCore<3> = ExecutionCore::new();
        let first = create(&mut core, 0x1000, 0x2000);
        let second = create(&mut core, 0x1100, 0x3000);
        let target = core.schedule().unwrap();
        core.confirm_dispatch(target).unwrap();
        assert_eq!(target.process, first);

        core.exit(first).unwrap();
        assert_eq!(core.reap(first), Err(CoreError::CurrentlyRunning));

        let next = core.schedule().unwrap();
        assert_eq!(next.process, second);
        core.confirm_dispatch(next).unwrap();
        core.reap(first).unwrap();

        let third = create(&mut core, 0x1200, 0x4000);
        assert_eq!(third, ProcessId(3));
    }

    #[test]
    fn blocked_current_can_be_released_to_idle_only_after_blocking() {
        let mut core: ExecutionCore<3> = ExecutionCore::new();
        let first = create(&mut core, 0x1000, 0x2000);
        let target = core.schedule().unwrap();
        core.confirm_dispatch(target).unwrap();
        core.block_process(first).unwrap();
        assert_eq!(core.confirm_idle(first), Ok(()));
        assert_eq!(core.current(), None);
        assert_eq!(core.unblock_process(first), Ok(()));
    }

    #[test]
    fn invalid_exit_and_no_runnable_are_reported() {
        let mut core: ExecutionCore<1> = ExecutionCore::new();
        assert_eq!(core.exit(ProcessId(99)), Err(CoreError::InvalidTransition));
        assert_eq!(core.schedule(), Err(CoreError::NoRunnable));
    }
}
