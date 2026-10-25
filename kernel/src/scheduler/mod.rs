#![no_std]

pub mod clock;
pub mod dispatch;
pub mod priority;

use crate::process::ProcessId;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RunQueue<const N: usize> {
    items: [Option<ProcessId>; N],
    head: usize,
    tail: usize,
    len: usize,
}

impl<const N: usize> Default for RunQueue<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> RunQueue<N> {
    pub const fn new() -> Self {
        Self {
            items: [None; N],
            head: 0,
            tail: 0,
            len: 0,
        }
    }
    pub const fn len(&self) -> usize {
        self.len
    }
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn push(&mut self, id: ProcessId) -> bool {
        if N == 0 || self.len == N || self.contains(id) {
            return false;
        }
        self.items[self.tail] = Some(id);
        self.tail = (self.tail + 1) % N;
        self.len += 1;
        true
    }
    pub fn pop(&mut self) -> Option<ProcessId> {
        if self.len == 0 {
            return None;
        }
        let id = self.items[self.head].take();
        self.head = (self.head + 1) % N;
        self.len -= 1;
        if self.len == 0 {
            self.head = 0;
            self.tail = 0;
        }
        id
    }

    /// Remove a queued process while preserving the order of the remaining
    /// entries. This is used when a process blocks or exits before dispatch.
    pub fn remove(&mut self, id: ProcessId) -> bool {
        let mut offset = 0;
        while offset < self.len && self.items[(self.head + offset) % N] != Some(id) {
            offset += 1;
        }
        if offset == self.len {
            return false;
        }

        for current_offset in offset..self.len - 1 {
            let destination = (self.head + current_offset) % N;
            let source = (self.head + current_offset + 1) % N;
            self.items[destination] = self.items[source];
        }

        let last = (self.head + self.len - 1) % N;
        self.items[last] = None;
        self.len -= 1;
        if self.len == 0 {
            self.head = 0;
            self.tail = 0;
        } else {
            self.tail = (self.head + self.len) % N;
        }
        true
    }

    fn contains(&self, id: ProcessId) -> bool {
        let mut i = 0;
        while i < self.len {
            if self.items[(self.head + i) % N] == Some(id) {
                return true;
            }
            i += 1;
        }
        false
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TickAction {
    Continue,
    Reschedule,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SchedulerClock {
    quantum: u32,
    remaining: u32,
}

impl SchedulerClock {
    pub const fn new(quantum: u32) -> Self {
        Self {
            quantum,
            remaining: quantum,
        }
    }
    pub const fn quantum(&self) -> u32 {
        self.quantum
    }
    pub const fn remaining(&self) -> u32 {
        self.remaining
    }
    pub fn tick(&mut self) -> TickAction {
        if self.quantum == 0 {
            return TickAction::Reschedule;
        }
        if self.remaining > 0 {
            self.remaining -= 1;
        }
        if self.remaining == 0 {
            self.remaining = self.quantum;
            TickAction::Reschedule
        } else {
            TickAction::Continue
        }
    }
    pub fn reset(&mut self) {
        self.remaining = self.quantum;
    }
}

/// Small deterministic bridge between an interrupt-side reschedule request and
/// the scheduler dispatcher. The interrupt path only sets a flag; the safe
/// scheduling boundary consumes it and selects the next runnable process.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DispatchAction {
    KeepCurrent,
    SwitchTo(ProcessId),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Scheduler<const N: usize> {
    queue: RunQueue<N>,
    current: Option<ProcessId>,
    reschedule_pending: bool,
}

impl<const N: usize> Default for Scheduler<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Scheduler<N> {
    pub const fn new() -> Self {
        Self {
            queue: RunQueue::new(),
            current: None,
            reschedule_pending: false,
        }
    }
    pub const fn current(&self) -> Option<ProcessId> {
        self.current
    }
    pub const fn reschedule_pending(&self) -> bool {
        self.reschedule_pending
    }
    pub fn enqueue(&mut self, id: ProcessId) -> bool {
        self.queue.push(id)
    }

    /// Remove a process from both the runnable queue and the current slot.
    /// Returns true if either location contained the process.
    pub fn remove(&mut self, id: ProcessId) -> bool {
        let mut removed = self.queue.remove(id);
        if self.current == Some(id) {
            self.current = None;
            removed = true;
        }
        removed
    }

    pub fn request_reschedule(&mut self) {
        self.reschedule_pending = true;
    }

    pub fn schedule_boundary(&mut self) -> DispatchAction {
        if !self.reschedule_pending {
            return DispatchAction::KeepCurrent;
        }
        self.reschedule_pending = false;
        if let Some(current) = self.current {
            let _ = self.queue.push(current);
        }
        match self.queue.pop() {
            Some(next) => {
                self.current = Some(next);
                DispatchAction::SwitchTo(next)
            }
            None => DispatchAction::KeepCurrent,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fifo_and_capacity_are_deterministic() {
        let mut q: RunQueue<2> = RunQueue::new();
        assert!(q.push(ProcessId(1)));
        assert!(q.push(ProcessId(2)));
        assert!(!q.push(ProcessId(3)));
        assert_eq!(q.pop(), Some(ProcessId(1)));
        assert!(q.push(ProcessId(3)));
        assert_eq!(q.pop(), Some(ProcessId(2)));
        assert_eq!(q.pop(), Some(ProcessId(3)));
        assert!(q.pop().is_none());
    }
    #[test]
    fn duplicate_is_rejected() {
        let mut q: RunQueue<2> = RunQueue::new();
        assert!(q.push(ProcessId(1)));
        assert!(!q.push(ProcessId(1)));
    }

    #[test]
    fn remove_preserves_fifo_order_across_wraparound() {
        let mut q: RunQueue<3> = RunQueue::new();
        assert!(q.push(ProcessId(1)));
        assert!(q.push(ProcessId(2)));
        assert!(q.push(ProcessId(3)));
        assert_eq!(q.pop(), Some(ProcessId(1)));
        assert!(q.push(ProcessId(4)));

        assert!(q.remove(ProcessId(3)));
        assert_eq!(q.len(), 2);
        assert_eq!(q.pop(), Some(ProcessId(2)));
        assert_eq!(q.pop(), Some(ProcessId(4)));
        assert!(q.pop().is_none());
        assert!(!q.remove(ProcessId(9)));
    }
    #[test]
    fn scheduler_clock_requests_reschedule_at_quantum_boundary() {
        let mut clock = SchedulerClock::new(3);
        assert_eq!(clock.tick(), TickAction::Continue);
        assert_eq!(clock.tick(), TickAction::Continue);
        assert_eq!(clock.tick(), TickAction::Reschedule);
        assert_eq!(clock.remaining(), 3);
    }
    #[test]
    fn boundary_switches_to_next_runnable_process() {
        let mut scheduler: Scheduler<4> = Scheduler::new();
        assert!(scheduler.enqueue(ProcessId(1)));
        assert!(scheduler.enqueue(ProcessId(2)));
        scheduler.request_reschedule();
        assert_eq!(
            scheduler.schedule_boundary(),
            DispatchAction::SwitchTo(ProcessId(1))
        );
        scheduler.request_reschedule();
        assert_eq!(
            scheduler.schedule_boundary(),
            DispatchAction::SwitchTo(ProcessId(2))
        );
        assert!(!scheduler.reschedule_pending());
    }
    #[test]
    fn no_request_keeps_current() {
        let mut scheduler: Scheduler<2> = Scheduler::new();
        assert_eq!(scheduler.schedule_boundary(), DispatchAction::KeepCurrent);
    }

    #[test]
    fn removing_current_clears_it_and_preserves_other_runnable_processes() {
        let mut scheduler: Scheduler<2> = Scheduler::new();
        assert!(scheduler.enqueue(ProcessId(1)));
        assert!(scheduler.enqueue(ProcessId(2)));
        scheduler.request_reschedule();
        assert_eq!(
            scheduler.schedule_boundary(),
            DispatchAction::SwitchTo(ProcessId(1))
        );

        assert!(scheduler.remove(ProcessId(1)));
        assert_eq!(scheduler.current(), None);
        scheduler.request_reschedule();
        assert_eq!(
            scheduler.schedule_boundary(),
            DispatchAction::SwitchTo(ProcessId(2))
        );
    }
    #[test]
    fn zero_quantum_fails_safe() {
        let mut clock = SchedulerClock::new(0);
        assert_eq!(clock.tick(), TickAction::Reschedule);
    }
}
