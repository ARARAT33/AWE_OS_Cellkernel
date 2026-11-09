#![no_std]

use crate::process::ProcessId;
use crate::memory::address::is_canonical_x86_64_48;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Context {
    pub process: ProcessId,
    pub instruction_pointer: u64,
    pub stack_pointer: u64,
    pub flags: u64,
    pub address_space: u64,
}

impl Context {
    pub const fn new(process: ProcessId, instruction_pointer: u64, stack_pointer: u64, flags: u64, address_space: u64) -> Self {
        Self { process, instruction_pointer, stack_pointer, flags, address_space }
    }

    pub const fn is_canonical_address(value: u64) -> bool {
        is_canonical_x86_64_48(value)
    }

    pub const fn validate(self) -> bool {
        self.instruction_pointer != 0
            && self.stack_pointer != 0
            && Self::is_canonical_address(self.instruction_pointer)
            && Self::is_canonical_address(self.stack_pointer)
            && (self.stack_pointer & 0x7) == 0
            && self.address_space != 0
            && (self.address_space & 0xfff) == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_context_is_accepted() {
        let c = Context::new(ProcessId(7), 0x0000_0000_0040_0000, 0x0000_0000_0080_0000, 0x202, 0x1000);
        assert!(c.validate());
    }

    #[test]
    fn invalid_context_is_rejected() {
        let c = Context::new(ProcessId(7), 0x0001_0000_0000_0000, 0x1001, 0, 0);
        assert!(!c.validate());
    }

    #[test]
    fn rejects_noncanonical_sign_extension_gaps_and_invalid_roots() {
        assert!(!Context::new(ProcessId(7), 0x0000_8000_0000_0000, 0x8000, 0x202, 0x1000).validate());
        assert!(!Context::new(ProcessId(7), 0x1000, 0xffff_7fff_ffff_fff8, 0x202, 0x1000).validate());
        assert!(!Context::new(ProcessId(7), 0, 0x8000, 0x202, 0x1000).validate());
        assert!(!Context::new(ProcessId(7), 0x1000, 0, 0x202, 0x1000).validate());
        assert!(!Context::new(ProcessId(7), 0x1000, 0x8000, 0x202, 0x1001).validate());
    }

    #[test]
    fn accepts_high_canonical_kernel_context() {
        assert!(Context::new(
            ProcessId(7),
            0xffff_8000_0000_1000,
            0xffff_8000_0000_8000,
            0x202,
            0x2000,
        ).validate());
    }
}
