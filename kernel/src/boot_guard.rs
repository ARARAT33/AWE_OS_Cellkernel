#![no_std]

use crate::memory::address::is_canonical_x86_64_48;

/// Deterministic early-boot invariants used before architecture-specific setup.
/// The guard is deliberately pure so it can be exercised in host-side tests.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BootGuardError {
    NonCanonicalEntry,
    NonCanonicalStack,
    UnalignedStack,
    EmptyStack,
    InvalidInterruptVector,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BootGuard {
    entry: u64,
    stack_top: u64,
}

impl BootGuard {
    pub const STACK_ALIGNMENT: u64 = 16;

    pub const fn validate(
        entry: u64,
        stack_top: u64,
        first_vector: u16,
    ) -> Result<Self, BootGuardError> {
        if !is_canonical_x86_64_48(entry) {
            return Err(BootGuardError::NonCanonicalEntry);
        }
        if stack_top == 0 {
            return Err(BootGuardError::EmptyStack);
        }
        if !is_canonical_x86_64_48(stack_top) {
            return Err(BootGuardError::NonCanonicalStack);
        }
        if !stack_top.is_multiple_of(Self::STACK_ALIGNMENT) {
            return Err(BootGuardError::UnalignedStack);
        }
        if first_vector >= 256 {
            return Err(BootGuardError::InvalidInterruptVector);
        }
        Ok(Self { entry, stack_top })
    }

    pub const fn entry(&self) -> u64 {
        self.entry
    }
    pub const fn stack_top(&self) -> u64 {
        self.stack_top
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_boot_boundary() {
        let guard = BootGuard::validate(0x0000_7fff_1234_5000, 0xffff_8000_0000_1000, 32).unwrap();
        assert_eq!(guard.entry(), 0x0000_7fff_1234_5000);
    }

    #[test]
    fn rejects_noncanonical_entry() {
        assert_eq!(
            BootGuard::validate(0x0001_0000_0000_0000, 0x1000, 32),
            Err(BootGuardError::NonCanonicalEntry)
        );
    }

    #[test]
    fn rejects_noncanonical_entry_and_stack_gaps() {
        assert_eq!(
            BootGuard::validate(0x0000_8000_0000_0000, 0x1000, 32),
            Err(BootGuardError::NonCanonicalEntry)
        );
        assert_eq!(
            BootGuard::validate(0x1000, 0x0000_8000_0000_0000, 32),
            Err(BootGuardError::NonCanonicalStack)
        );
        assert_eq!(
            BootGuard::validate(0x1000, 0xffff_7fff_ffff_fff0, 32),
            Err(BootGuardError::NonCanonicalStack)
        );
    }

    #[test]
    fn rejects_bad_stack_and_vector() {
        assert_eq!(
            BootGuard::validate(0x1000, 0, 32),
            Err(BootGuardError::EmptyStack)
        );
        assert_eq!(
            BootGuard::validate(0x1000, 0x1008, 32),
            Err(BootGuardError::UnalignedStack)
        );
        assert_eq!(
            BootGuard::validate(0x1000, 0x2000, 256),
            Err(BootGuardError::InvalidInterruptVector)
        );
    }
}
