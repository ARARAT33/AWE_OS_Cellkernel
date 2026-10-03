#![no_std]

use super::context::ProcessContext;
use super::context_switch::SwitchFrame;
use super::dispatch::{DispatchError, DispatchTarget, prepare_dispatch};
use super::x86_64_backend::{BackendError, validate_backend_frames};
use super::{ProcessDescriptor, ProcessId, ProcessState};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SchedulerError {
    QueueFull,
    Empty,
    NotRunnable,
    UnknownProcess,
    InvalidContext,
    Dispatch(DispatchError),
    Backend(BackendError),
}

pub struct Scheduler<const N: usize> {
    queue: [Option<ProcessId>; N],
    head: usize,
    len: usize,
    ticks: u64,
}

impl<const N: usize> Default for Scheduler<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Scheduler<N> {
    pub const fn new() -> Self {
        Self {
            queue: [None; N],
            head: 0,
            len: 0,
            ticks: 0,
        }
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn ticks(&self) -> u64 {
        self.ticks
    }

    pub fn enqueue(&mut self, process: &ProcessDescriptor) -> Result<(), SchedulerError> {
        if !matches!(
            process.state,
            ProcessState::Runnable | ProcessState::Running
        ) {
            return Err(SchedulerError::NotRunnable);
        }
        if self.len == N {
            return Err(SchedulerError::QueueFull);
        }
        if self
            .queue
            .iter()
            .flatten()
            .any(|queued| *queued == process.id)
        {
            return Ok(());
        }
        if N == 0 {
            return Err(SchedulerError::QueueFull);
        }
        let index = (self.head + self.len) % N;
        self.queue[index] = Some(process.id);
        self.len += 1;
        Ok(())
    }

    pub fn dequeue(&mut self) -> Result<ProcessId, SchedulerError> {
        if self.len == 0 {
            return Err(SchedulerError::Empty);
        }
        let id = self.queue[self.head].take().ok_or(SchedulerError::Empty)?;
        self.head = (self.head + 1) % N;
        self.len -= 1;
        self.ticks = self.ticks.wrapping_add(1);
        Ok(id)
    }

    pub fn yield_process(&mut self, process: &ProcessDescriptor) -> Result<(), SchedulerError> {
        self.enqueue(process)
    }

    pub fn prepare_next(
        &mut self,
        contexts: &[ProcessContext; N],
    ) -> Result<DispatchTarget, SchedulerError> {
        let id = self.dequeue()?;
        let context = contexts
            .iter()
            .find(|ctx| ctx.process_id == id)
            .ok_or(SchedulerError::UnknownProcess)?;
        if !context.is_valid() {
            return Err(SchedulerError::InvalidContext);
        }
        let descriptor = ProcessDescriptor {
            id,
            state: ProcessState::Runnable,
            budget: super::ResourceBudget::unlimited(),
        };
        DispatchTarget::from_descriptor(&descriptor, &context.cpu).map_err(SchedulerError::Dispatch)
    }

    pub fn prepare_switch(
        &mut self,
        current: &SwitchFrame,
        contexts: &[ProcessContext; N],
    ) -> Result<DispatchTarget, SchedulerError> {
        let target = self.prepare_next(contexts)?;
        prepare_dispatch(current, &target).map_err(SchedulerError::Dispatch)?;
        Ok(target)
    }

    /// Execute the selected x86_64 register save/restore after validating both
    /// frames. This is the scheduler's real architecture boundary.
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn switch_next(
        &mut self,
        current: &mut SwitchFrame,
        contexts: &[ProcessContext; N],
    ) -> Result<DispatchTarget, SchedulerError> {
        let target = self.prepare_switch(current, contexts)?;
        validate_backend_frames(current, &target.frame).map_err(SchedulerError::Backend)?;
        // SAFETY: both frames were validated; the backend only saves/restores
        // registers and transfers execution to the target frame.
        unsafe {
            super::x86_64_backend::awe_x86_64_switch(current as *mut _, &target.frame as *const _);
        }
        Ok(target)
    }

    #[cfg(not(target_arch = "x86_64"))]
    pub unsafe fn switch_next(
        &mut self,
        current: &mut SwitchFrame,
        contexts: &[ProcessContext; N],
    ) -> Result<DispatchTarget, SchedulerError> {
        let _ = current;
        let _ = contexts;
        Err(SchedulerError::Backend(BackendError::InvalidCurrent))
    }
}
