#![no_std]

/// Validate a 48-bit x86_64 canonical virtual address (four-level paging).
/// Bits 63:48 must be an exact sign extension of bit 47. This deliberately
/// does not claim support for five-level paging (LA57).
pub const fn is_canonical_x86_64_48(address: u64) -> bool {
    let upper = address >> 48;
    if address & (1u64 << 47) == 0 {
        upper == 0
    } else {
        upper == 0xffff
    }
}

/// Validate a non-null user pointer in the lower canonical half of a
/// four-level x86_64 address space. Mapping and access permissions are checked
/// separately by the active address-space implementation.
pub const fn is_canonical_user_address_x86_64_48(address: u64) -> bool {
    address != 0 && address >> 47 == 0
}

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct VirtualAddress(pub u64);

impl VirtualAddress {
    pub const PAGE_SIZE: u64 = 4096;
    pub const fn is_aligned(self) -> bool {
        self.0 & (Self::PAGE_SIZE - 1) == 0
    }
    pub const fn is_canonical(self) -> bool {
        is_canonical_x86_64_48(self.0)
    }
    pub const fn align_down(self) -> Self {
        Self(self.0 & !(Self::PAGE_SIZE - 1))
    }
    pub const fn align_up(self) -> Option<Self> {
        let mask = Self::PAGE_SIZE - 1;
        match self.0.checked_add(mask) {
            Some(v) => Some(Self(v & !mask)),
            None => None,
        }
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PhysicalAddress(pub u64);

impl PhysicalAddress {
    pub const PAGE_SIZE: u64 = 4096;
    pub const fn is_aligned(self) -> bool {
        self.0 & (Self::PAGE_SIZE - 1) == 0
    }
    pub const fn align_down(self) -> Self {
        Self(self.0 & !(Self::PAGE_SIZE - 1))
    }
    pub const fn align_up(self) -> Option<Self> {
        let mask = Self::PAGE_SIZE - 1;
        match self.0.checked_add(mask) {
            Some(v) => Some(Self(v & !mask)),
            None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alignment_is_overflow_safe() {
        assert_eq!(VirtualAddress(0x1234).align_down().0, 0x1000);
        assert_eq!(VirtualAddress(0x1001).align_up().unwrap().0, 0x2000);
        assert!(VirtualAddress(u64::MAX).align_up().is_none());
        assert!(PhysicalAddress(0x4000).is_aligned());
    }

    #[test]
    fn canonical_address_validation_checks_sign_extension() {
        assert!(is_canonical_x86_64_48(0x0000_7fff_ffff_ffff));
        assert!(is_canonical_x86_64_48(0xffff_8000_0000_0000));
        assert!(is_canonical_x86_64_48(0xffff_ffff_ffff_ffff));
        assert!(!is_canonical_x86_64_48(0x0000_8000_0000_0000));
        assert!(!is_canonical_x86_64_48(0xffff_7fff_ffff_ffff));
        assert!(!VirtualAddress(0x0000_8000_0000_0000).is_canonical());
    }

    #[test]
    fn user_pointer_validation_is_lower_half_only() {
        assert!(is_canonical_user_address_x86_64_48(0x0000_7fff_ffff_ffff));
        assert!(!is_canonical_user_address_x86_64_48(0));
        assert!(!is_canonical_user_address_x86_64_48(0x0000_8000_0000_0000));
        assert!(!is_canonical_user_address_x86_64_48(0xffff_8000_0000_0000));
    }
}
