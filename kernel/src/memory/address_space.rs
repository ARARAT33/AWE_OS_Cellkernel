#![no_std]

use super::address::is_canonical_user_address_x86_64_48;
use super::paging::{PageFlags, valid_user_mapping};

/// Size of one page in bytes, re-declared locally to keep this module free of
/// frame-allocator dependencies.
pub const ADDRESS_SPACE_PAGE_SIZE: u64 = 4096;

/// Upper bound of the canonical user half for a four-level x86_64 address
/// space (exclusive). The lower canonical half is 0x0000_8000_0000_0000 bytes.
pub const USER_HALF_END: u64 = 0x0000_8000_0000_0000;

/// Errors returned by the address-space and checked-user-copy contract layer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UserCopyError {
    /// Null or zero-length accesses are rejected outright.
    InvalidRange,
    /// The address is not in the lower canonical user half.
    NotUserAddress,
    /// The range wraps the address space or crosses into the kernel half.
    RangeOverflow,
    /// The range is not fully covered by present user mappings.
    Unmapped,
    /// A page in the range is mapped, but lacks the required permission.
    PermissionDenied,
    /// The bounded mapping table is full.
    TableFull,
    /// The requested mapping is not registered.
    NotMapped,
    /// The physical or virtual arguments are not page aligned, or the flags
    /// are not a valid user mapping.
    InvalidMapping,
    /// The virtual range of a new mapping overlaps an existing mapping.
    Overlap,
}

/// Validate that [address, address + len) is a fully contained user range:
/// nonzero, canonical user half only, no wraparound.
pub const fn checked_user_range(address: u64, len: u64) -> Result<(), UserCopyError> {
    if len == 0 {
        return Err(UserCopyError::InvalidRange);
    }
    if !is_canonical_user_address_x86_64_48(address) {
        return Err(UserCopyError::NotUserAddress);
    }
    let end = match address.checked_add(len) {
        Some(end) => end,
        None => return Err(UserCopyError::RangeOverflow),
    };
    if end > USER_HALF_END {
        return Err(UserCopyError::RangeOverflow);
    }
    Ok(())
}

/// One registered user mapping: a page-aligned virtual range backed by a
/// contiguous page-aligned physical range with uniform flags.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct UserMapping {
    pub virtual_start: u64,
    pub physical_start: u64,
    /// Number of 4 KiB pages covered by this mapping. Always >= 1.
    pub page_count: u64,
    pub flags: PageFlags,
}

impl UserMapping {
    /// Exclusive virtual end, overflow-checked at registration time.
    pub const fn virtual_end(&self) -> u64 {
        self.virtual_start + self.page_count * ADDRESS_SPACE_PAGE_SIZE
    }

    pub const fn covers(&self, page: u64) -> bool {
        page >= self.virtual_start && page < self.virtual_end()
    }
}

/// A per-process address-space contract object: an owning PML4 root frame and
/// a bounded registry of user mappings with checked access validation.
///
/// This is the software contract layer for per-process address spaces. It
/// validates and records every user mapping and every checked user copy
/// before any hardware page-table activation. It does not itself install
/// page tables, touch CR3, or perform the physical copy; the architecture
/// bootstrap must wire the root frame and the checked copy must be performed
/// only after this validation succeeds.
pub struct AddressSpace<const MAX_MAPPINGS: usize> {
    root_pml4_physical: u64,
    mappings: [Option<UserMapping>; MAX_MAPPINGS],
    used: usize,
}

impl<const MAX_MAPPINGS: usize> AddressSpace<MAX_MAPPINGS> {
    /// Create an address space over a page-aligned PML4 root frame.
    /// A zero root is rejected: frame zero is never a valid root here.
    pub fn new(root_pml4_physical: u64) -> Result<Self, UserCopyError> {
        if root_pml4_physical == 0 || root_pml4_physical & (ADDRESS_SPACE_PAGE_SIZE - 1) != 0 {
            return Err(UserCopyError::InvalidMapping);
        }
        Ok(Self {
            root_pml4_physical,
            mappings: [None; MAX_MAPPINGS],
            used: 0,
        })
    }

    pub const fn root_pml4_physical(&self) -> u64 {
        self.root_pml4_physical
    }

    pub const fn mapping_count(&self) -> usize {
        self.used
    }

    /// Register a user mapping. Both addresses must be page aligned, the
    /// flags must satisfy valid_user_mapping, the virtual range must lie in
    /// the user half, and the range must not overlap any existing mapping.
    pub fn map_user(
        &mut self,
        virtual_start: u64,
        physical_start: u64,
        page_count: u64,
        flags: PageFlags,
    ) -> Result<(), UserCopyError> {
        if page_count == 0 {
            return Err(UserCopyError::InvalidRange);
        }
        if virtual_start & (ADDRESS_SPACE_PAGE_SIZE - 1) != 0
            || physical_start & (ADDRESS_SPACE_PAGE_SIZE - 1) != 0
            || !valid_user_mapping(physical_start, flags)
        {
            return Err(UserCopyError::InvalidMapping);
        }
        let bytes = match page_count.checked_mul(ADDRESS_SPACE_PAGE_SIZE) {
            Some(bytes) => bytes,
            None => return Err(UserCopyError::RangeOverflow),
        };
        let virtual_end = match virtual_start.checked_add(bytes) {
            Some(end) => end,
            None => return Err(UserCopyError::RangeOverflow),
        };
        if virtual_end > USER_HALF_END {
            return Err(UserCopyError::RangeOverflow);
        }
        if self.used >= MAX_MAPPINGS {
            return Err(UserCopyError::TableFull);
        }
        for slot in self.mappings.iter().flatten() {
            let other_end = slot.virtual_end();
            if virtual_start < other_end && slot.virtual_start < virtual_end {
                return Err(UserCopyError::Overlap);
            }
        }
        self.mappings[self.used] = Some(UserMapping {
            virtual_start,
            physical_start,
            page_count,
            flags,
        });
        self.used += 1;
        Ok(())
    }

    /// Remove a mapping previously registered at exactly virtual_start.
    pub fn unmap_user(&mut self, virtual_start: u64) -> Result<(), UserCopyError> {
        let index = self
            .mappings
            .iter()
            .position(|slot| matches!(slot, Some(m) if m.virtual_start == virtual_start))
            .ok_or(UserCopyError::NotMapped)?;
        self.mappings[index] = None;
        self.used -= 1;
        Ok(())
    }

    /// Translate a page-aligned user virtual page to its physical page.
    pub fn translate_page(&self, virtual_page: u64) -> Option<u64> {
        if virtual_page & (ADDRESS_SPACE_PAGE_SIZE - 1) != 0 {
            return None;
        }
        self.mappings
            .iter()
            .flatten()
            .find(|m| m.covers(virtual_page))
            .map(|m| m.physical_start + (virtual_page - m.virtual_start))
    }

    /// Checked user access validation for [address, address + len).
    ///
    /// Every touched page must be registered, present, carry USER, and carry
    /// WRITABLE when write is requested. Zero-length and non-user ranges are
    /// rejected before any mapping lookup. This is the gate that must pass
    /// before a kernel-side copy touches user memory.
    pub fn check_user_access(
        &self,
        address: u64,
        len: u64,
        write: bool,
    ) -> Result<(), UserCopyError> {
        checked_user_range(address, len)?;
        let start_page = address & !(ADDRESS_SPACE_PAGE_SIZE - 1);
        let end = address + len;
        let mut page = start_page;
        while page < end {
            let mapping = self
                .mappings
                .iter()
                .flatten()
                .find(|m| m.covers(page))
                .ok_or(UserCopyError::Unmapped)?;
            if !mapping.flags.contains(PageFlags::USER)
                || !mapping.flags.contains(PageFlags::PRESENT)
            {
                return Err(UserCopyError::PermissionDenied);
            }
            if write && !mapping.flags.contains(PageFlags::WRITABLE) {
                return Err(UserCopyError::PermissionDenied);
            }
            page += ADDRESS_SPACE_PAGE_SIZE;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: PageFlags = PageFlags::PRESENT.union(PageFlags::USER);
    const RW: PageFlags = R.union(PageFlags::WRITABLE);

    #[test]
    fn root_validation_rejects_zero_and_misaligned() {
        assert!(AddressSpace::<4>::new(0).is_err());
        assert!(AddressSpace::<4>::new(0x1000_1001).is_err());
        assert!(AddressSpace::<4>::new(0x5000).is_ok());
    }

    #[test]
    fn checked_user_range_rejects_kernel_null_and_overflow() {
        assert_eq!(
            checked_user_range(0x1000, 0),
            Err(UserCopyError::InvalidRange)
        );
        assert_eq!(checked_user_range(0, 8), Err(UserCopyError::NotUserAddress));
        assert_eq!(
            checked_user_range(0x0000_8000_0000_0000, 8),
            Err(UserCopyError::NotUserAddress)
        );
        assert_eq!(
            checked_user_range(0x0000_7fff_ffff_ffff, 8),
            Err(UserCopyError::RangeOverflow)
        );
        assert!(checked_user_range(0x0000_7fff_ffff_fff8, 8).is_ok());
        assert!(checked_user_range(0x1000, 16).is_ok());
    }

    #[test]
    fn map_user_rejects_invalid_and_overlapping_ranges() {
        let mut space = AddressSpace::<4>::new(0x5000).unwrap();
        assert!(space.map_user(0x1001, 0x9000, 1, RW).is_err());
        assert!(space.map_user(0x1000, 0x9001, 1, RW).is_err());
        assert!(
            space
                .map_user(0x1000, 0x9000, 1, PageFlags::PRESENT)
                .is_err()
        );
        assert!(space.map_user(0x1000, 0x9000, 0, RW).is_err());
        assert!(space.map_user(0x1000, 0x9000, 2, RW).is_ok());
        assert_eq!(
            space.map_user(0x2000, 0xa000, 1, RW),
            Err(UserCopyError::Overlap)
        );
        assert!(space.map_user(0x3000, 0xa000, 1, RW).is_ok());
    }

    #[test]
    fn check_user_access_requires_mapping_and_permissions() {
        let mut space = AddressSpace::<4>::new(0x5000).unwrap();
        space.map_user(0x1000, 0x9000, 1, R).unwrap();
        space.map_user(0x2000, 0xa000, 1, RW).unwrap();

        assert!(space.check_user_access(0x1000, 8, false).is_ok());
        assert_eq!(
            space.check_user_access(0x1000, 8, true),
            Err(UserCopyError::PermissionDenied)
        );
        assert!(space.check_user_access(0x2000, 8, true).is_ok());
        assert_eq!(
            space.check_user_access(0x1400, 0x800, true),
            Err(UserCopyError::Unmapped)
        );
        assert_eq!(
            space.check_user_access(0x4000, 8, false),
            Err(UserCopyError::Unmapped)
        );
        // A range spanning both mappings is readable but not writable.
        assert!(space.check_user_access(0x1ff8, 16, false).is_ok());
        assert_eq!(
            space.check_user_access(0x1ff8, 16, true),
            Err(UserCopyError::PermissionDenied)
        );
    }

    #[test]
    fn translate_page_resolves_offset_within_mapping() {
        let mut space = AddressSpace::<4>::new(0x5000).unwrap();
        space.map_user(0x1000, 0x9000, 2, RW).unwrap();
        assert_eq!(space.translate_page(0x1000), Some(0x9000));
        assert_eq!(space.translate_page(0x2000), Some(0xa000));
        assert_eq!(space.translate_page(0x3000), None);
        assert_eq!(space.translate_page(0x1001), None);
    }

    #[test]
    fn unmap_removes_exactly_the_matching_mapping() {
        let mut space = AddressSpace::<4>::new(0x5000).unwrap();
        space.map_user(0x1000, 0x9000, 1, RW).unwrap();
        assert_eq!(space.unmap_user(0x2000), Err(UserCopyError::NotMapped));
        assert!(space.unmap_user(0x1000).is_ok());
        assert_eq!(space.mapping_count(), 0);
        assert_eq!(space.translate_page(0x1000), None);
    }

    #[test]
    fn table_full_fails_closed() {
        let mut space = AddressSpace::<2>::new(0x5000).unwrap();
        assert!(space.map_user(0x1000, 0x9000, 1, RW).is_ok());
        assert!(space.map_user(0x3000, 0xb000, 1, RW).is_ok());
        assert_eq!(
            space.map_user(0x5000, 0xd000, 1, RW),
            Err(UserCopyError::TableFull)
        );
    }
}
