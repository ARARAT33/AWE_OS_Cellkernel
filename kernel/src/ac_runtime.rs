#![no_std]
#![allow(clippy::collapsible_if)]

use crate::execution_core::{CoreError, ExecutionCore};
use crate::process::{
    ProcessId, ResourceBudget,
    context::CpuContext,
    context_switch::SwitchFrame,
    dispatch::DispatchTarget,
};

const MAX_RANGES: usize = 16;
const MAX_IPC: usize = 16;
const MAX_CAPS: usize = 32;
const MAX_TIMERS: usize = 16;
const MAX_TRACE: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AcError {
    Core(CoreError),
    InvalidRange,
    RangeTableFull,
    QueueFull,
    QueueEmpty,
    CapabilityTableFull,
    CapabilityDenied,
    InvalidSyscall,
    InvalidArgument,
    TimerTableFull,
    TraceFull,
}

impl From<CoreError> for AcError {
    fn from(value: CoreError) -> Self {
        Self::Core(value)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct UserRange {
    pub start: u64,
    pub len: u64,
}

impl UserRange {
    pub const fn end(self) -> Option<u64> {
        self.start.checked_add(self.len)
    }

    pub const fn contains(self, ptr: u64, len: u64) -> bool {
        match (self.end(), ptr.checked_add(len)) {
            (Some(end), Some(req_end)) => ptr >= self.start && req_end <= end,
            _ => false,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct IpcMessage {
    pub sender: ProcessId,
    pub opcode: u32,
    pub arg: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Capability {
    pub process: ProcessId,
    pub token: u64,
    pub rights: u32,
    pub active: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Timer {
    pub process: ProcessId,
    pub deadline: u64,
    pub token: u32,
    pub active: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TraceEvent {
    pub process: ProcessId,
    pub kind: u16,
    pub value: u64,
}

/// A bounded A-C runtime boundary. It connects process execution with the
/// privileged invariants required by the master plan: user-range validation,
/// quota-limited IPC, capability revocation, monotonic timers and structured
/// diagnostics. All storage is fixed-capacity and fail-closed.
pub struct AcRuntime<const N: usize> {
    pub core: ExecutionCore<N>,
    ranges: [Option<(ProcessId, UserRange)>; MAX_RANGES],
    ipc: [Option<IpcMessage>; MAX_IPC],
    ipc_head: usize,
    ipc_len: usize,
    caps: [Option<Capability>; MAX_CAPS],
    timers: [Option<Timer>; MAX_TIMERS],
    trace: [Option<TraceEvent>; MAX_TRACE],
    trace_len: usize,
    now: u64,
}

impl<const N: usize> Default for AcRuntime<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> AcRuntime<N> {
    pub const fn new() -> Self {
        Self {
            core: ExecutionCore::new(),
            ranges: [None; MAX_RANGES],
            ipc: [None; MAX_IPC],
            ipc_head: 0,
            ipc_len: 0,
            caps: [None; MAX_CAPS],
            timers: [None; MAX_TIMERS],
            trace: [None; MAX_TRACE],
            trace_len: 0,
            now: 0,
        }
    }

    pub fn create_process(
        &mut self,
        budget: ResourceBudget,
        cpu: CpuContext,
    ) -> Result<ProcessId, AcError> {
        let id = self.core.create_process(budget, cpu)?;
        self.trace(id, 1, 0)?;
        Ok(id)
    }

    pub fn map_user_range(
        &mut self,
        process: ProcessId,
        start: u64,
        len: u64,
    ) -> Result<(), AcError> {
        if len == 0 || start & 0xfff != 0 || len & 0xfff != 0 || start.checked_add(len).is_none() {
            return Err(AcError::InvalidRange);
        }
        let mut i = 0;
        while i < MAX_RANGES {
            if self.ranges[i].is_none() {
                self.ranges[i] = Some((process, UserRange { start, len }));
                self.trace(process, 2, len)?;
                return Ok(());
            }
            i += 1;
        }
        Err(AcError::RangeTableFull)
    }

    pub fn validate_user_buffer(
        &self,
        process: ProcessId,
        ptr: u64,
        len: u64,
    ) -> Result<(), AcError> {
        if len == 0 {
            return Err(AcError::InvalidArgument);
        }
        let mut i = 0;
        while i < MAX_RANGES {
            if let Some((owner, range)) = self.ranges[i] {
                if owner == process && range.contains(ptr, len) {
                    return Ok(());
                }
            }
            i += 1;
        }
        Err(AcError::InvalidRange)
    }

    pub fn send_ipc(&mut self, sender: ProcessId, opcode: u32, arg: u64) -> Result<(), AcError> {
        if self.ipc_len == MAX_IPC {
            return Err(AcError::QueueFull);
        }
        let index = (self.ipc_head + self.ipc_len) % MAX_IPC;
        self.ipc[index] = Some(IpcMessage {
            sender,
            opcode,
            arg,
        });
        self.ipc_len += 1;
        self.trace(sender, 3, opcode as u64)?;
        Ok(())
    }

    pub fn recv_ipc(&mut self) -> Result<IpcMessage, AcError> {
        if self.ipc_len == 0 {
            return Err(AcError::QueueEmpty);
        }
        let msg = self.ipc[self.ipc_head].take().ok_or(AcError::QueueEmpty)?;
        self.ipc_head = (self.ipc_head + 1) % MAX_IPC;
        self.ipc_len -= 1;
        Ok(msg)
    }

    pub fn grant_capability(
        &mut self,
        process: ProcessId,
        token: u64,
        rights: u32,
    ) -> Result<(), AcError> {
        let mut i = 0;
        while i < MAX_CAPS {
            if self.caps[i].is_none() {
                self.caps[i] = Some(Capability {
                    process,
                    token,
                    rights,
                    active: true,
                });
                self.trace(process, 4, token)?;
                return Ok(());
            }
            i += 1;
        }
        Err(AcError::CapabilityTableFull)
    }

    pub fn check_capability(
        &self,
        process: ProcessId,
        token: u64,
        rights: u32,
    ) -> Result<(), AcError> {
        let mut i = 0;
        while i < MAX_CAPS {
            if let Some(cap) = self.caps[i] {
                if cap.process == process
                    && cap.token == token
                    && cap.active
                    && cap.rights & rights == rights
                {
                    return Ok(());
                }
            }
            i += 1;
        }
        Err(AcError::CapabilityDenied)
    }

    pub fn revoke_capability(&mut self, process: ProcessId, token: u64) -> Result<(), AcError> {
        let mut i = 0;
        while i < MAX_CAPS {
            if let Some(mut cap) = self.caps[i] {
                if cap.process == process && cap.token == token {
                    cap.active = false;
                    self.caps[i] = Some(cap);
                    self.trace(process, 5, token)?;
                    return Ok(());
                }
            }
            i += 1;
        }
        Err(AcError::CapabilityDenied)
    }

    pub fn validate_syscall(
        &self,
        number: u32,
        argc: usize,
        max_number: u32,
        max_args: usize,
    ) -> Result<(), AcError> {
        if number > max_number {
            return Err(AcError::InvalidSyscall);
        }
        if argc > max_args {
            return Err(AcError::InvalidArgument);
        }
        Ok(())
    }

    pub fn arm_timer(&mut self, process: ProcessId, delay: u64, token: u32) -> Result<(), AcError> {
        let deadline = self
            .now
            .checked_add(delay)
            .ok_or(AcError::InvalidArgument)?;
        let mut i = 0;
        while i < MAX_TIMERS {
            if self.timers[i].is_none() {
                self.timers[i] = Some(Timer {
                    process,
                    deadline,
                    token,
                    active: true,
                });
                return Ok(());
            }
            i += 1;
        }
        Err(AcError::TimerTableFull)
    }

    pub fn advance_time(&mut self, delta: u64) -> Result<u64, AcError> {
        self.now = self
            .now
            .checked_add(delta)
            .ok_or(AcError::InvalidArgument)?;
        let mut fired = 0;
        let mut i = 0;
        while i < MAX_TIMERS {
            if let Some(mut timer) = self.timers[i] {
                if timer.active && timer.deadline <= self.now {
                    timer.active = false;
                    self.timers[i] = Some(timer);
                    fired += 1;
                }
            }
            i += 1;
        }
        Ok(fired)
    }

    /// Prepare a context-switch target. The caller must pass the returned token
    /// to the architecture completion path; selecting a target does not publish
    /// it as the current process.
    pub fn schedule(&mut self) -> Result<DispatchTarget, AcError> {
        self.core.schedule().map_err(AcError::Core)
    }

    pub fn persist_outgoing_frame(
        &mut self,
        process: ProcessId,
        frame: &SwitchFrame,
    ) -> Result<(), AcError> {
        self.core
            .persist_outgoing_frame(process, frame)
            .map_err(AcError::Core)
    }

    pub fn confirm_dispatch(&mut self, target: DispatchTarget) -> Result<(), AcError> {
        self.core.confirm_dispatch(target).map_err(AcError::Core)
    }

    pub fn cancel_dispatch(&mut self, target: DispatchTarget) -> Result<(), AcError> {
        self.core.cancel_dispatch(target).map_err(AcError::Core)
    }

    pub fn current(&self) -> Option<ProcessId> {
        self.core.current()
    }

    pub fn trace_len(&self) -> usize {
        self.trace_len
    }

    fn trace(&mut self, process: ProcessId, kind: u16, value: u64) -> Result<(), AcError> {
        if self.trace_len == MAX_TRACE {
            return Err(AcError::TraceFull);
        }
        self.trace[self.trace_len] = Some(TraceEvent {
            process,
            kind,
            value,
        });
        self.trace_len += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cpu_context() -> CpuContext {
        CpuContext::kernel_entry(0x1000, 0x2000, 0)
    }

    fn budget() -> ResourceBudget {
        ResourceBudget {
            cpu_ticks: 8,
            memory_bytes: 8192,
            ipc_messages: 4,
        }
    }

    #[test]
    fn user_memory_is_bounded_and_overflow_safe() {
        let mut rt: AcRuntime<2> = AcRuntime::new();
        let p = rt.create_process(budget(), cpu_context()).unwrap();
        rt.map_user_range(p, 0x1000, 0x2000).unwrap();
        assert!(rt.validate_user_buffer(p, 0x1800, 0x800).is_ok());
        assert_eq!(
            rt.validate_user_buffer(p, 0x2fff, 2),
            Err(AcError::InvalidRange)
        );
        assert_eq!(
            rt.validate_user_buffer(p, u64::MAX - 3, 8),
            Err(AcError::InvalidRange)
        );
    }

    #[test]
    fn ipc_is_fifo_and_has_a_hard_bound() {
        let mut rt: AcRuntime<1> = AcRuntime::new();
        let p = rt.create_process(budget(), cpu_context()).unwrap();
        for i in 0..MAX_IPC {
            rt.send_ipc(p, i as u32, i as u64).unwrap();
        }
        assert_eq!(rt.send_ipc(p, 99, 99), Err(AcError::QueueFull));
        assert_eq!(rt.recv_ipc().unwrap().opcode, 0);
        assert_eq!(rt.recv_ipc().unwrap().opcode, 1);
    }

    #[test]
    fn capability_revocation_is_fail_closed() {
        let mut rt: AcRuntime<1> = AcRuntime::new();
        let p = rt.create_process(budget(), cpu_context()).unwrap();
        rt.grant_capability(p, 7, 0b11).unwrap();
        assert!(rt.check_capability(p, 7, 0b01).is_ok());
        rt.revoke_capability(p, 7).unwrap();
        assert_eq!(
            rt.check_capability(p, 7, 0b01),
            Err(AcError::CapabilityDenied)
        );
    }

    #[test]
    fn syscall_and_timer_boundaries_reject_bad_input() {
        let mut rt: AcRuntime<1> = AcRuntime::new();
        let p = rt.create_process(budget(), cpu_context()).unwrap();
        assert!(rt.validate_syscall(3, 2, 8, 4).is_ok());
        assert_eq!(
            rt.validate_syscall(9, 0, 8, 4),
            Err(AcError::InvalidSyscall)
        );
        assert_eq!(
            rt.validate_syscall(1, 5, 8, 4),
            Err(AcError::InvalidArgument)
        );
        rt.arm_timer(p, 5, 1).unwrap();
        assert_eq!(rt.advance_time(4).unwrap(), 0);
        assert_eq!(rt.advance_time(1).unwrap(), 1);
    }
}
