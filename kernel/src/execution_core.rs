#![no_std]
#![allow(clippy::collapsible_if)]

use crate::process::{ProcessDescriptor, ProcessId, ProcessState, ResourceBudget};
use crate::scheduler::{DispatchAction, Scheduler};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CoreError {
    ProcessTableFull,
    ProcessIdExhausted,
    RunQueueFull,
    InvalidTransition,
    BudgetExceeded,
}

/// Trace record for execution core event diagnostics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CoreTraceRecord {
    pub process: ProcessId,
    pub event_kind: u16,
    pub value: u64,
}

/// A bounded integration boundary for the A-C execution core.
/// It keeps scheduling, lifecycle and resource accounting in one deterministic
/// state machine without allocating or depending on hardware drivers.
pub struct ExecutionCore<const N: usize> {
    scheduler: Scheduler<N>,
    processes: [Option<ProcessDescriptor>; N],
    next_id: u64,
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
            scheduler: Scheduler::new(),
            processes: [None; N],
            next_id: 1,
            trace_records: [None; 32],
            trace_count: 0,
        }
    }

    pub fn create_process(&mut self, budget: ResourceBudget) -> Result<ProcessId, CoreError> {
        let mut slot = 0;
        while slot < N {
            if self.processes[slot].is_none() {
                // Zero is the exhausted sentinel. Unlike saturating_add,
                // this prevents repeatedly issuing u64::MAX after exhaustion.
                if self.next_id == 0 {
                    return Err(CoreError::ProcessIdExhausted);
                }
                let id = ProcessId(self.next_id);
                self.next_id = self.next_id.checked_add(1).unwrap_or(0);
                let mut descriptor = ProcessDescriptor {
                    id,
                    state: ProcessState::Created,
                    budget,
                };
                descriptor
                    .transition(ProcessState::Runnable)
                    .map_err(|_| CoreError::InvalidTransition)?;
                self.processes[slot] = Some(descriptor);
                if !self.scheduler.enqueue(id) {
                    self.processes[slot] = None;
                    return Err(CoreError::ProcessTableFull);
                }
                self.record_trace(id, 1, budget.cpu_ticks);
                return Ok(id);
            }
            slot += 1;
        }
        Err(CoreError::ProcessTableFull)
    }

    pub fn block_process(&mut self, id: ProcessId) -> Result<(), CoreError> {
        let index = self.find_index(id).ok_or(CoreError::InvalidTransition)?;
        let mut candidate = self.processes[index].ok_or(CoreError::InvalidTransition)?;
        if candidate.state != ProcessState::Running && candidate.state != ProcessState::Runnable {
            return Err(CoreError::InvalidTransition);
        }
        if candidate.state == ProcessState::Runnable {
            candidate
                .transition(ProcessState::Running)
                .map_err(|_| CoreError::InvalidTransition)?;
        }
        candidate
            .transition(ProcessState::Blocked)
            .map_err(|_| CoreError::InvalidTransition)?;

        // Blocking must release both a queued slot and the current slot.
        self.scheduler.remove(id);
        self.processes[index] = Some(candidate);
        self.record_trace(id, 2, 0);
        Ok(())
    }

    pub fn unblock_process(&mut self, id: ProcessId) -> Result<(), CoreError> {
        let index = self.find_index(id).ok_or(CoreError::InvalidTransition)?;
        let mut candidate = self.processes[index].ok_or(CoreError::InvalidTransition)?;
        if candidate.state != ProcessState::Blocked {
            return Err(CoreError::InvalidTransition);
        }
        candidate
            .transition(ProcessState::Runnable)
            .map_err(|_| CoreError::InvalidTransition)?;

        // Do not publish Runnable state unless queue admission succeeds.
        if !self.scheduler.enqueue(id) {
            return Err(CoreError::RunQueueFull);
        }
        self.processes[index] = Some(candidate);
        self.record_trace(id, 3, 0);
        Ok(())
    }

    pub fn yield_process(&mut self, id: ProcessId) -> Result<(), CoreError> {
        let index = self.find_index(id).ok_or(CoreError::InvalidTransition)?;
        let mut candidate = self.processes[index].ok_or(CoreError::InvalidTransition)?;
        if candidate.state != ProcessState::Running {
            return Err(CoreError::InvalidTransition);
        }
        candidate
            .transition(ProcessState::Runnable)
            .map_err(|_| CoreError::InvalidTransition)?;

        // Keep the old Running state if the queue cannot accept this process.
        if !self.scheduler.enqueue(id) {
            return Err(CoreError::RunQueueFull);
        }
        self.processes[index] = Some(candidate);
        self.record_trace(id, 4, 0);
        Ok(())
    }

    pub fn schedule(&mut self) -> DispatchAction {
        self.scheduler.request_reschedule();
        loop {
            let action = self.scheduler.schedule_boundary();
            match action {
                DispatchAction::SwitchTo(id) => {
                    let is_active = self
                        .find(id)
                        .map(|p| {
                            p.state == ProcessState::Runnable || p.state == ProcessState::Running
                        })
                        .unwrap_or(false);
                    if is_active {
                        self.set_running(id);
                        return action;
                    }
                    // Skip blocked / exited processes that remain in queue
                }
                DispatchAction::KeepCurrent => {
                    let is_active = self
                        .current()
                        .and_then(|curr| self.find(curr))
                        .map(|p| {
                            p.state == ProcessState::Runnable || p.state == ProcessState::Running
                        })
                        .unwrap_or(false);
                    if is_active {
                        return action;
                    }
                    return DispatchAction::KeepCurrent;
                }
            }
        }
    }

    pub fn consume_cpu(&mut self, id: ProcessId, ticks: u64) -> Result<(), CoreError> {
        if let Some(process) = self.find_mut(id) {
            if process.budget.consume_cpu(ticks) {
                self.record_trace(id, 5, ticks);
                Ok(())
            } else {
                Err(CoreError::BudgetExceeded)
            }
        } else {
            Err(CoreError::InvalidTransition)
        }
    }

    pub fn exit(&mut self, id: ProcessId) -> Result<(), CoreError> {
        let index = self.find_index(id).ok_or(CoreError::InvalidTransition)?;
        let mut candidate = self.processes[index].ok_or(CoreError::InvalidTransition)?;
        if candidate.state != ProcessState::Running
            && candidate.state != ProcessState::Runnable
            && candidate.state != ProcessState::Blocked
        {
            return Err(CoreError::InvalidTransition);
        }
        if candidate.state == ProcessState::Runnable {
            candidate
                .transition(ProcessState::Running)
                .map_err(|_| CoreError::InvalidTransition)?;
        }
        candidate
            .transition(ProcessState::Exited)
            .map_err(|_| CoreError::InvalidTransition)?;

        // An exited process must not remain runnable or remain the current task.
        self.scheduler.remove(id);
        self.processes[index] = Some(candidate);
        self.record_trace(id, 6, 0);
        Ok(())
    }

    pub const fn current(&self) -> Option<ProcessId> {
        self.scheduler.current()
    }

    pub fn trace_records(&self) -> &[Option<CoreTraceRecord>] {
        &self.trace_records[..self.trace_count]
    }

    fn record_trace(&mut self, process: ProcessId, event_kind: u16, value: u64) {
        if self.trace_count < 32 {
            self.trace_records[self.trace_count] = Some(CoreTraceRecord {
                process,
                event_kind,
                value,
            });
            self.trace_count += 1;
        }
    }

    fn set_running(&mut self, id: ProcessId) {
        let mut i = 0;
        while i < N {
            if let Some(process) = self.processes[i].as_mut() {
                if process.id == id {
                    let _ = process.transition(ProcessState::Running);
                } else if process.state == ProcessState::Running {
                    let _ = process.transition(ProcessState::Runnable);
                }
            }
            i += 1;
        }
    }

    fn find(&self, id: ProcessId) -> Option<&ProcessDescriptor> {
        let mut i = 0;
        while i < N {
            if self.processes[i].as_ref().map(|p| p.id) == Some(id) {
                return self.processes[i].as_ref();
            }
            i += 1;
        }
        None
    }

    fn find_mut(&mut self, id: ProcessId) -> Option<&mut ProcessDescriptor> {
        let mut i = 0;
        while i < N {
            if self.processes[i].as_ref().map(|p| p.id) == Some(id) {
                return self.processes[i].as_mut();
            }
            i += 1;
        }
        None
    }

    fn find_index(&self, id: ProcessId) -> Option<usize> {
        let mut i = 0;
        while i < N {
            if self.processes[i].as_ref().map(|p| p.id) == Some(id) {
                return Some(i);
            }
            i += 1;
        }
        None
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

    #[test]
    fn lifecycle_and_preemption_are_bounded() {
        let mut core: ExecutionCore<2> = ExecutionCore::new();
        let first = core.create_process(budget()).unwrap();
        let second = core.create_process(budget()).unwrap();
        assert_eq!(core.schedule(), DispatchAction::SwitchTo(first));
        assert_eq!(core.current(), Some(first));
        assert_eq!(core.schedule(), DispatchAction::SwitchTo(second));
        assert_eq!(core.current(), Some(second));
    }

    #[test]
    fn process_id_exhaustion_does_not_reissue_max_id() {
        let mut core: ExecutionCore<3> = ExecutionCore::new();
        core.next_id = u64::MAX - 1;

        let penultimate = core.create_process(budget()).unwrap();
        let final_id = core.create_process(budget()).unwrap();

        assert_eq!(penultimate, ProcessId(u64::MAX - 1));
        assert_eq!(final_id, ProcessId(u64::MAX));
        assert_eq!(
            core.create_process(budget()),
            Err(CoreError::ProcessIdExhausted)
        );
        assert_eq!(
            core.create_process(budget()),
            Err(CoreError::ProcessIdExhausted)
        );
        assert_eq!(core.processes.iter().flatten().count(), 2);
    }

    #[test]
    fn capacity_and_budget_fail_closed() {
        let mut core: ExecutionCore<1> = ExecutionCore::new();
        let id = core.create_process(budget()).unwrap();
        assert_eq!(
            core.create_process(budget()),
            Err(CoreError::ProcessTableFull)
        );
        assert!(core.consume_cpu(id, 4).is_ok());
        assert_eq!(core.consume_cpu(id, 1), Err(CoreError::BudgetExceeded));
    }

    #[test]
    fn invalid_exit_is_rejected() {
        let mut core: ExecutionCore<1> = ExecutionCore::new();
        assert_eq!(core.exit(ProcessId(99)), Err(CoreError::InvalidTransition));
    }

    #[test]
    fn block_and_exit_remove_processes_from_scheduler_state() {
        let mut core: ExecutionCore<2> = ExecutionCore::new();
        let first = core.create_process(budget()).unwrap();
        let second = core.create_process(budget()).unwrap();

        assert_eq!(core.schedule(), DispatchAction::SwitchTo(first));
        assert!(core.block_process(first).is_ok());
        assert_eq!(core.current(), None);
        assert_eq!(core.schedule(), DispatchAction::SwitchTo(second));

        assert!(core.exit(second).is_ok());
        assert_eq!(core.current(), None);
        assert_eq!(core.schedule(), DispatchAction::KeepCurrent);
        assert_eq!(core.current(), None);

        assert!(core.unblock_process(first).is_ok());
        assert_eq!(core.schedule(), DispatchAction::SwitchTo(first));
    }

    #[test]
    fn queue_full_does_not_publish_unblock_or_yield_state() {
        let mut unblock_core: ExecutionCore<2> = ExecutionCore::new();
        let blocked = unblock_core.create_process(budget()).unwrap();
        let other = unblock_core.create_process(budget()).unwrap();
        assert_eq!(unblock_core.schedule(), DispatchAction::SwitchTo(blocked));
        assert!(unblock_core.block_process(blocked).is_ok());
        assert!(unblock_core.scheduler.enqueue(ProcessId(99)));
        assert_eq!(
            unblock_core.unblock_process(blocked),
            Err(CoreError::RunQueueFull)
        );
        assert_eq!(
            unblock_core.find(blocked).map(|p| p.state),
            Some(ProcessState::Blocked)
        );
        assert_eq!(
            unblock_core.find(other).map(|p| p.state),
            Some(ProcessState::Runnable)
        );

        let mut yield_core: ExecutionCore<2> = ExecutionCore::new();
        let first = yield_core.create_process(budget()).unwrap();
        let running = yield_core.create_process(budget()).unwrap();
        assert_eq!(yield_core.schedule(), DispatchAction::SwitchTo(first));
        assert_eq!(yield_core.schedule(), DispatchAction::SwitchTo(running));
        assert!(yield_core.scheduler.enqueue(ProcessId(99)));
        assert_eq!(
            yield_core.yield_process(running),
            Err(CoreError::RunQueueFull)
        );
        assert_eq!(
            yield_core.find(running).map(|p| p.state),
            Some(ProcessState::Running)
        );
        assert_eq!(yield_core.current(), Some(running));
    }

    #[test]
    fn block_unblock_yield_and_tracing_work_as_expected() {
        let mut core: ExecutionCore<2> = ExecutionCore::new();
        let p1 = core.create_process(budget()).unwrap();
        assert_eq!(core.schedule(), DispatchAction::SwitchTo(p1));
        assert!(core.yield_process(p1).is_ok());
        assert!(core.block_process(p1).is_ok());
        assert_eq!(core.yield_process(p1), Err(CoreError::InvalidTransition));
        assert!(core.unblock_process(p1).is_ok());
        assert_eq!(core.schedule(), DispatchAction::SwitchTo(p1));
        assert!(core.exit(p1).is_ok());
        assert!(!core.trace_records().is_empty());
    }
}
