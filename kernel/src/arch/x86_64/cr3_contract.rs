#![no_std]

//! CR3 and TLB activation contract for CellKernel on x86_64.
//!
//! This module is the fail-closed gate every future CR3 switch must pass
//! before it may write CR3 or rely on a TLB flush. It exists because of a
//! structural finding, not a missing feature:
//!
//! The current kernel image executes at user-half identity virtual
//! addresses (the bootstrap identity map, `memory::x86_64`). A root built
//! by `KernelPageTableBuilder` is kernel-half-only by design and contains
//! no user-half mappings, so the instruction fetch immediately after a
//! `mov cr3, <kernel-half-root>` would fault: the running code, its stack
//! and its serial/MMIO output all live in the user half, which the target
//! root does not map. Activating such a root from this image would
//! triple-fault before any new mapping could be used.
//!
//! Consequences, enforced here:
//!
//! - `identity_image_can_activate_kernel_half_root()` is `const false`.
//!   A CR3 switch becomes legal only after the kernel itself executes from
//!   kernel-half virtual addresses (higher-half kernel loading), which is
//!   a separate, QEMU-proven milestone.
//! - `validate_root_physical` rejects zero, misaligned and physically
//!   truncated roots before any switch.
//! - `check_activation_preconditions` validates the roots, refuses no-op
//!   switches, and refuses activation until the structural precondition
//!   above is satisfied by a future higher-half image.
//! - TLB policy: a CR3 write flushes every non-global translation. Any
//!   entry carrying `PageFlags::GLOBAL` survives it and therefore needs
//!   CR4.PGE management or an explicit `invlpg`.
//!   `requires_pge_or_explicit_invalidation` names that obligation so a
//!   future switch cannot silently keep stale global entries.

use crate::memory::paging::PageFlags;

/// Bits of a CR3 value that carry the root physical address (bits 12..51).
pub const CR3_PHYSICAL_MASK: u64 = 0x000f_ffff_ffff_f000;

/// First address past the four-level physical address space (bit 51 is the
/// highest usable bit).
pub const MAX_PHYSICAL_LIMIT: u64 = 1 << 52;

/// Failure modes of page-table root validation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RootValidationError {
    /// The zero root is never a valid page-table root.
    ZeroRoot,
    /// A root must be 4 KiB aligned.
    Misaligned,
    /// The root exceeds the four-level physical address space.
    BeyondPhysicalAddressSpace,
}

/// Validate a page-table root physical address before it may ever be
/// written to CR3.
pub const fn validate_root_physical(physical: u64) -> Result<(), RootValidationError> {
    if physical == 0 {
        return Err(RootValidationError::ZeroRoot);
    }
    if physical & 0xfff != 0 {
        return Err(RootValidationError::Misaligned);
    }
    if physical & !CR3_PHYSICAL_MASK != 0 {
        return Err(RootValidationError::BeyondPhysicalAddressSpace);
    }
    Ok(())
}

/// Extract the root physical address from a CR3 value, ignoring the low
/// control flags (PWT, PCD and future PCID fields).
pub const fn cr3_root_of(value: u64) -> u64 {
    value & CR3_PHYSICAL_MASK
}

/// The CR3 control bits below bit 12. They are legal for the hardware but
/// change cache/write policy, so probes report them explicitly.
pub const fn cr3_low_flags(value: u64) -> u64 {
    value & 0x0fff
}

/// Failure modes of the activation precondition check.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ActivationPreconditionError {
    /// The target root failed root validation.
    InvalidTargetRoot(RootValidationError),
    /// Writing the same root would be a no-op, not an activation.
    TargetEqualsCurrent,
    /// The running image cannot prove that the target root maps its
    /// instruction stream, stack and device windows. True for the current
    /// identity-mapped image by construction.
    IdentityImageCannotSwitch,
}

/// Structural precondition of any CR3 switch from the current image.
///
/// `const false` on purpose: the kernel executes at user-half identity
/// addresses, and a kernel-half-only root cannot map them. A future
/// higher-half kernel image must replace this predicate with a real,
/// QEMU-proven proof before any switch is wired.
pub const fn identity_image_can_activate_kernel_half_root() -> bool {
    false
}

/// Fail-closed preconditions for a CR3 switch.
///
/// This validates the target root, rejects no-op switches, and refuses
/// activation while the structural identity-image precondition fails. It
/// never writes CR3 and never mutates anything. A caller must additionally
/// prove that the target root maps everything the running code touches
/// (instruction stream, stack, serial/MMIO windows) once the structural
/// precondition is unlocked.
pub const fn check_activation_preconditions(
    current_root: u64,
    target_root: u64,
) -> Result<(), ActivationPreconditionError> {
    match validate_root_physical(target_root) {
        Ok(()) => {}
        Err(error) => {
            return Err(ActivationPreconditionError::InvalidTargetRoot(error));
        }
    }
    if target_root == cr3_root_of(current_root) {
        return Err(ActivationPreconditionError::TargetEqualsCurrent);
    }
    if !identity_image_can_activate_kernel_half_root() {
        return Err(ActivationPreconditionError::IdentityImageCannotSwitch);
    }
    Ok(())
}

/// Entries carrying GLOBAL survive a CR3 write; a switch involving them
/// needs CR4.PGE management or an explicit invlpg, or stale translations
/// persist across the switch.
pub const fn requires_pge_or_explicit_invalidation(flags: PageFlags) -> bool {
    flags.contains(PageFlags::GLOBAL)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::x86_64::{BOOTSTRAP_LEAF_FLAGS, BOOTSTRAP_TABLE_FLAGS};

    #[test]
    fn valid_roots_pass_validation() {
        assert_eq!(validate_root_physical(0x1000), Ok(()));
        assert_eq!(validate_root_physical(0x4000_0000), Ok(()));
        assert_eq!(validate_root_physical(MAX_PHYSICAL_LIMIT - 0x1000), Ok(()));
    }

    #[test]
    fn invalid_roots_fail_with_distinct_errors() {
        assert_eq!(
            validate_root_physical(0),
            Err(RootValidationError::ZeroRoot)
        );
        assert_eq!(
            validate_root_physical(0x1001),
            Err(RootValidationError::Misaligned)
        );
        assert_eq!(
            validate_root_physical(MAX_PHYSICAL_LIMIT),
            Err(RootValidationError::BeyondPhysicalAddressSpace)
        );
        assert_eq!(
            validate_root_physical(0xffff_f000_0000_0000),
            Err(RootValidationError::BeyondPhysicalAddressSpace)
        );
    }

    #[test]
    fn cr3_value_is_split_into_root_and_low_flags() {
        assert_eq!(cr3_root_of(0x1000 | 0x8), 0x1000);
        assert_eq!(cr3_low_flags(0x1000 | 0x8), 0x8);
        assert_eq!(cr3_low_flags(0x1000), 0);
        // Reserved high bits are not part of the low CR3 control/PCID field.
        assert_eq!(cr3_low_flags(0x1000 | 0x8 | (1 << 60)), 0x8);
    }

    #[test]
    fn preconditions_reject_zero_and_noop_switches() {
        assert_eq!(
            check_activation_preconditions(0x1000, 0),
            Err(ActivationPreconditionError::InvalidTargetRoot(
                RootValidationError::ZeroRoot
            ))
        );
        // The current value may carry low flags; the no-op check uses roots.
        assert_eq!(
            check_activation_preconditions(0x1000 | 0x8, 0x1000),
            Err(ActivationPreconditionError::TargetEqualsCurrent)
        );
    }

    #[test]
    fn activation_is_structurally_blocked_from_the_identity_image() {
        assert!(!identity_image_can_activate_kernel_half_root());
        assert_eq!(
            check_activation_preconditions(0x1000, 0x2000),
            Err(ActivationPreconditionError::IdentityImageCannotSwitch)
        );
    }

    #[test]
    fn bootstrap_flags_carry_no_global_bit() {
        let table_flags = PageFlags(BOOTSTRAP_TABLE_FLAGS);
        let leaf_flags = PageFlags(BOOTSTRAP_LEAF_FLAGS);
        assert!(!requires_pge_or_explicit_invalidation(table_flags));
        assert!(!requires_pge_or_explicit_invalidation(leaf_flags));
        assert_eq!(BOOTSTRAP_TABLE_FLAGS & PageFlags::GLOBAL.0, 0);
        assert_eq!(BOOTSTRAP_LEAF_FLAGS & PageFlags::GLOBAL.0, 0);
    }

    #[test]
    fn global_entries_are_flagged_as_requiring_explicit_invalidation() {
        assert!(requires_pge_or_explicit_invalidation(
            PageFlags::PRESENT.union(PageFlags::GLOBAL)
        ));
    }
}
