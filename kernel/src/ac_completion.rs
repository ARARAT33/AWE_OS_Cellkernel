#![no_std]

//! A-C completion primitives: bounded bring-up, execution and evidence gates.
//!
//! These types are intentionally small and deterministic. They model the
//! state that the real architecture backends must prove before exposing a
//! subsystem as ready; they do not pretend to execute privileged CPU
//! instructions in unit-test builds.

use crate::memory::address::{is_canonical_user_address_x86_64_48, is_canonical_x86_64_48};

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BringupPhase {
    Reset = 0,
    BootInfoValidated = 1,
    GdtTssReady = 2,
    IdtReady = 3,
    InterruptsReady = 4,
    ApicReady = 5,
    MemoryReady = 6,
    PagingReady = 7,
    HeapReady = 8,
    SmpReady = 9,
    KernelReady = 10,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BringupError {
    InvalidTransition,
    MissingPrerequisite,
    InvalidCpuTopology,
    InvalidMemoryRange,
    MisalignedPage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuTopology {
    pub boot_cpu: u16,
    pub online: u16,
    pub max_supported: u16,
}

impl CpuTopology {
    pub const fn validate(self) -> Result<(), BringupError> {
        if self.online == 0 || self.boot_cpu >= self.online || self.online > self.max_supported {
            return Err(BringupError::InvalidCpuTopology);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryRange {
    pub start: u64,
    pub len: u64,
}

impl MemoryRange {
    pub const fn end(self) -> Option<u64> {
        self.start.checked_add(self.len)
    }

    pub const fn page_aligned(self, page_size: u64) -> bool {
        page_size != 0 && self.start.is_multiple_of(page_size) && self.len.is_multiple_of(page_size)
    }

    pub const fn validate(self, page_size: u64) -> Result<(), BringupError> {
        if self.len == 0 || self.end().is_none() {
            return Err(BringupError::InvalidMemoryRange);
        }
        if !self.page_aligned(page_size) {
            return Err(BringupError::MisalignedPage);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BringupGate {
    phase: BringupPhase,
}

impl BringupGate {
    pub const fn new() -> Self {
        Self {
            phase: BringupPhase::Reset,
        }
    }

    pub const fn phase(self) -> BringupPhase {
        self.phase
    }

    pub const fn advance(self, next: BringupPhase) -> Result<Self, BringupError> {
        let is_valid = match (self.phase, next) {
            (p1, p2) if p1 as u8 == p2 as u8 => p1 as u8 == BringupPhase::KernelReady as u8,
            (p1, p2) => (p1 as u8 + 1) == p2 as u8,
        };
        if is_valid {
            Ok(Self { phase: next })
        } else {
            Err(BringupError::InvalidTransition)
        }
    }
}

impl Default for BringupGate {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContextFrame {
    pub instruction_pointer: u64,
    pub stack_pointer: u64,
    pub flags: u64,
    pub address_space: u64,
}

impl ContextFrame {
    pub const fn validate(self) -> bool {
        self.instruction_pointer != 0
            && self.stack_pointer != 0
            && is_canonical_x86_64_48(self.instruction_pointer)
            && is_canonical_x86_64_48(self.stack_pointer)
            && self.address_space != 0
            && (self.stack_pointer & 0xf) == 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyscallBoundary {
    pub number: u32,
    pub arg_count: u8,
    pub max_args: u8,
    pub user_pointer: Option<u64>,
}

impl SyscallBoundary {
    pub const fn validate(self) -> bool {
        let ptr_ok = match self.user_pointer {
            Some(p) => is_canonical_user_address_x86_64_48(p),
            None => true,
        };
        self.arg_count <= self.max_args && self.max_args <= 6 && ptr_ok
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IpcQuota {
    pub messages: u16,
    pub bytes: u32,
}

impl IpcQuota {
    pub const fn permits(self, messages: u16, bytes: u32) -> bool {
        messages <= self.messages && bytes <= self.bytes
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapabilityAudit {
    pub capability: u64,
    pub generation: u32,
    pub revoked: bool,
}

impl CapabilityAudit {
    pub const fn usable(self, generation: u32) -> bool {
        self.capability != 0 && !self.revoked && self.generation == generation
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimerDeadline {
    pub now: u64,
    pub deadline: u64,
}

impl TimerDeadline {
    pub const fn expired(self) -> bool {
        self.now >= self.deadline
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraceEvent {
    pub sequence: u64,
    pub kind: u16,
    pub subject: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bringup_is_strictly_ordered() {
        let gate = BringupGate::new();
        assert_eq!(
            gate.advance(BringupPhase::IdtReady),
            Err(BringupError::InvalidTransition)
        );
        let gate = gate.advance(BringupPhase::BootInfoValidated).unwrap();
        let gate = gate.advance(BringupPhase::GdtTssReady).unwrap();
        let gate = gate.advance(BringupPhase::IdtReady).unwrap();
        assert_eq!(gate.phase(), BringupPhase::IdtReady);
    }

    #[test]
    fn memory_validation_is_overflow_and_alignment_safe() {
        assert!(
            MemoryRange {
                start: 0x1000,
                len: 0x2000
            }
            .validate(0x1000)
            .is_ok()
        );
        assert_eq!(
            MemoryRange {
                start: u64::MAX,
                len: 1
            }
            .validate(0x1000),
            Err(BringupError::InvalidMemoryRange)
        );
        assert_eq!(
            MemoryRange {
                start: 0x1001,
                len: 0x1000
            }
            .validate(0x1000),
            Err(BringupError::MisalignedPage)
        );
    }

    #[test]
    fn cpu_topology_rejects_invalid_online_sets() {
        assert!(
            CpuTopology {
                boot_cpu: 0,
                online: 4,
                max_supported: 8
            }
            .validate()
            .is_ok()
        );
        assert_eq!(
            CpuTopology {
                boot_cpu: 4,
                online: 4,
                max_supported: 8
            }
            .validate(),
            Err(BringupError::InvalidCpuTopology)
        );
        assert_eq!(
            CpuTopology {
                boot_cpu: 0,
                online: 9,
                max_supported: 8
            }
            .validate(),
            Err(BringupError::InvalidCpuTopology)
        );
    }

    #[test]
    fn context_and_syscall_boundaries_fail_closed() {
        assert!(
            ContextFrame {
                instruction_pointer: 1,
                stack_pointer: 0x1000,
                flags: 0,
                address_space: 2
            }
            .validate()
        );
        assert!(
            !ContextFrame {
                instruction_pointer: 1,
                stack_pointer: 0x1008,
                flags: 0,
                address_space: 2
            }
            .validate()
        );
        assert!(
            !ContextFrame {
                instruction_pointer: 0x0000_8000_0000_0000,
                stack_pointer: 0x1000,
                flags: 0,
                address_space: 2
            }
            .validate()
        );
        assert!(
            !ContextFrame {
                instruction_pointer: 0x1000,
                stack_pointer: 0xffff_7fff_ffff_fff0,
                flags: 0,
                address_space: 2
            }
            .validate()
        );
        assert!(
            SyscallBoundary {
                number: 3,
                arg_count: 2,
                max_args: 6,
                user_pointer: Some(0x1000)
            }
            .validate()
        );
        assert!(
            !SyscallBoundary {
                number: 3,
                arg_count: 7,
                max_args: 6,
                user_pointer: Some(0x1000)
            }
            .validate()
        );
        assert!(
            !SyscallBoundary {
                number: 3,
                arg_count: 1,
                max_args: 6,
                user_pointer: Some(0x0000_8000_0000_0000)
            }
            .validate()
        );
        assert!(
            !SyscallBoundary {
                number: 3,
                arg_count: 1,
                max_args: 6,
                user_pointer: Some(0xffff_8000_0000_0000)
            }
            .validate()
        );
    }

    #[test]
    fn quotas_cap_ipc_and_capability_rejects_stale_authority() {
        let quota = IpcQuota {
            messages: 8,
            bytes: 4096,
        };
        assert!(quota.permits(8, 4096));
        assert!(!quota.permits(9, 1));
        let cap = CapabilityAudit {
            capability: 42,
            generation: 7,
            revoked: false,
        };
        assert!(cap.usable(7));
        assert!(!cap.usable(6));
        assert!(
            !CapabilityAudit {
                revoked: true,
                ..cap
            }
            .usable(7)
        );
    }

    #[test]
    fn timer_and_trace_are_deterministic_values() {
        assert!(
            !TimerDeadline {
                now: 9,
                deadline: 10
            }
            .expired()
        );
        assert!(
            TimerDeadline {
                now: 10,
                deadline: 10
            }
            .expired()
        );
        assert_eq!(
            TraceEvent {
                sequence: 4,
                kind: 2,
                subject: 9
            }
            .sequence,
            4
        );
    }
}
