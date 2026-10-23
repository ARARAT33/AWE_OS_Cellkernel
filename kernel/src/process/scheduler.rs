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
        // Check for an existing entry before capacity: re-enqueuing an already
        // queued process is idempotent, even when the queue is full.
        if self
            .queue
            .iter()
            .flatten()
            .any(|queued| *queued == process.id)
        {
            return Ok(());
        }
        if self.len == N || N == 0 {
            return Err(SchedulerError::QueueFull);
        }
        let index = (self.head + self.len) % N;
        self.queue[index] = Some(process.id);
        self.len += 1;
        Ok(())
    }

    /// Remove a process from the circular run queue without changing the
    /// relative order of the other entries. Returns false when it was absent.
    pub fn remove(&mut self, id: ProcessId) -> bool {
        let Some(offset) =
            (0..self.len).find(|&offset| self.queue[(self.head + offset) % N] == Some(id))
        else {
            return false;
        };

        for current_offset in offset..self.len - 1 {
            let destination = (self.head + current_offset) % N;
            let source = (self.head + current_offset + 1) % N;
            self.queue[destination] = self.queue[source];
        }

        let tail = (self.head + self.len - 1) % N;
        self.queue[tail] = None;
        self.len -= 1;
        if self.len == 0 {
            self.head = 0;
        }
        true
    }

    pub fn dequeue(&mut self) -> Result<ProcessId, SchedulerError> {
        if self.len == 0 {
            return Err(SchedulerError::Empty);
        }
        let id = self.queue[self.head].take().ok_or(SchedulerError::Empty)?;
        self.head = (self.head + 1) % N;
        self.len -= 1;
        self.ticks = self.ticks.wrapping_add(1);
        if self.len == 0 {
            self.head = 0;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn process(id: u64) -> ProcessDescriptor {
        ProcessDescriptor {
            id: ProcessId(id),
            state: ProcessState::Runnable,
            budget: super::super::ResourceBudget::unlimited(),
        }
    }

    #[test]
    fn remove_preserves_fifo_order_across_wraparound() {
        let mut scheduler = Scheduler::<3>::new();
        scheduler.enqueue(&process(1)).unwrap();
        scheduler.enqueue(&process(2)).unwrap();
        scheduler.enqueue(&process(3)).unwrap();
        assert_eq!(scheduler.dequeue(), Ok(ProcessId(1)));
        scheduler.enqueue(&process(4)).unwrap();

        assert!(scheduler.remove(ProcessId(3)));
        assert_eq!(scheduler.len(), 2);
        assert_eq!(scheduler.dequeue(), Ok(ProcessId(2)));
        assert_eq!(scheduler.dequeue(), Ok(ProcessId(4)));
        assert_eq!(scheduler.dequeue(), Err(SchedulerError::Empty));
    }

    #[test]
    fn duplicate_enqueue_is_idempotent_even_when_full() {
        let mut scheduler = Scheduler::<2>::new();
        scheduler.enqueue(&process(1)).unwrap();
        scheduler.enqueue(&process(2)).unwrap();

        assert_eq!(scheduler.enqueue(&process(1)), Ok(()));
        assert_eq!(scheduler.len(), 2);
        assert_eq!(scheduler.dequeue(), Ok(ProcessId(1)));
        assert_eq!(scheduler.dequeue(), Ok(ProcessId(2)));
    }

    #[test]
    fn zero_capacity_queue_fails_safely() {
        let mut scheduler = Scheduler::<0>::new();
        assert_eq!(
            scheduler.enqueue(&process(1)),
            Err(SchedulerError::QueueFull)
        );
        assert!(!scheduler.remove(ProcessId(1)));
        assert_eq!(scheduler.dequeue(), Err(SchedulerError::Empty));
    }
}
