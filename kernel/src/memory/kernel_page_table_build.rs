#![no_std]

//! Supervisor-only page-table builder for the kernel half of a CellKernel
//! x86_64 address space.
//!
//! This module is the kernel-half counterpart of `UserPageTableBuilder`. It
//! consumes the validated higher-half layout contract from `kernel_map` and
//! installs PML4 -> PDPT -> PD -> PT entries through the same `TableMemory`
//! backend, with three hard rules:
//!
//! 1. **Kernel half only.** No entry is ever installed at a PML4 index below
//!    256; the user half of every root stays untouched.
//! 2. **Supervisor only.** The `USER` flag is rejected on leaf mappings and
//!    never set on intermediate entries, so kernel mappings cannot be
//!    traversed from Ring 3.
//! 3. **Planned regions only.** Every mapping must start inside a planned
//!    higher-half region from `kernel_map`, must not cross that region's
//!    half-open boundary, and a writable mapping must be non-executable
//!    (W^X).
//!
//! This layer never touches CR3, never invalidates the TLB, and never frees
//! table frames: allocated tables are recorded in allocation order so the
//! owner can reclaim them. It explicitly does not activate the mapping it
//! builds; activation and QEMU layout/permission evidence are later steps.

use super::address::is_canonical_x86_64_48;
use super::kernel_map::{
    DIRECT_MAP_BASE, DIRECT_MAP_MAX_PHYSICAL, KernelRegionKind, kernel_region_of, pml4_index,
};
use super::page_table_build::TableMemory;
use super::paging::{PageFlags, PageTableEntry, indices, valid_mapping};

/// 4 KiB page size used by this builder (large pages are not installed).
pub const KERNEL_PAGE_SIZE: u64 = 0x1000;

/// First byte past the bounded direct-map virtual window.
pub const DIRECT_MAP_WINDOW_END: u64 = DIRECT_MAP_BASE + DIRECT_MAP_MAX_PHYSICAL;

/// Errors returned while building kernel-half page tables.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KernelPageBuildError {
    /// The PML4 root frame is zero, unaligned, or not representable.
    InvalidRoot,
    /// The frame source is exhausted; the build fails closed.
    OutOfFrames,
    /// More intermediate tables were allocated than the builder tracks.
    TableLimit,
    /// A table could not be accessed through the memory backend.
    MemoryUnreadable,
    /// An install would target the user half of the PML4 (index < 256).
    UserHalfForbidden,
    /// The mapping is empty, misaligned, or has a non-representable shape.
    InvalidMapping,
    /// Large-page (`HUGE`) leaves are not supported by this builder.
    HugeForbidden,
    /// A mapping carried the USER flag; kernel mappings stay supervisor-only.
    UserFlagForbidden,
    /// A writable kernel mapping was not non-executable (W^X violation).
    WritableExecutable,
    /// A writable mapping was planned inside the kernel text region.
    WritableTextRegion,
    /// The virtual start address is not inside any planned kernel region.
    OutsidePlannedRegion,
    /// The mapping would cross the half-open boundary of its planned region.
    CrossesRegionBoundary,
    /// Address arithmetic overflowed while walking or validating a range.
    Overflow,
    /// A leaf slot is already present; existing mappings are never overwritten.
    AlreadyMapped,
    /// An intermediate entry is a huge-page leaf, not a next-level table.
    HugePageConflict,
    /// An existing intermediate table is not a supervisor-only writable table.
    IntermediatePermissionConflict,
    /// A table frame is zero, unaligned, or outside the representable range.
    InvalidTableFrame,
    /// The table allocator returned a frame already owned by this builder.
    DuplicateTableFrame,
    /// Two mappings in one install batch cover at least one common virtual page.
    OverlappingMappings,
}

/// Errors reported when validating that physical ranges fit the bounded
/// direct-map window.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DirectMapFitError {
    /// A zero-length range carries no information and is rejected.
    EmptyRange,
    /// `base + length` overflowed.
    Overflow,
    /// The range extends past the bounded direct-map window; boot must fail
    /// or reject the memory map instead of growing the window.
    BeyondWindow { end: u64 },
}

/// Validate that one physical byte range fits inside the bounded direct map.
pub const fn fits_direct_map_window(base: u64, length: u64) -> Result<(), DirectMapFitError> {
    if length == 0 {
        return Err(DirectMapFitError::EmptyRange);
    }
    let end = match base.checked_add(length) {
        Some(end) => end,
        None => return Err(DirectMapFitError::Overflow),
    };
    if end > DIRECT_MAP_MAX_PHYSICAL {
        return Err(DirectMapFitError::BeyondWindow { end });
    }
    Ok(())
}

/// Validate that every physical byte range in a boot memory map fits inside
/// the bounded direct-map window. The first offending range is reported by
/// index so boot diagnostics can name the exact entry.
pub fn validate_ram_ranges(ranges: &[(u64, u64)]) -> Result<(), (usize, DirectMapFitError)> {
    for (index, &(base, length)) in ranges.iter().enumerate() {
        if let Err(e) = fits_direct_map_window(base, length) {
            return Err((index, e));
        }
    }
    Ok(())
}

/// One planned kernel-half mapping request.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct KernelMapping {
    /// Virtual start address inside a planned kernel-half region.
    pub virtual_start: u64,
    /// Physical start address; 4 KiB aligned.
    pub physical_start: u64,
    /// Number of 4 KiB pages.
    pub page_count: u64,
    /// Leaf flags. `USER` and `HUGE` are forbidden.
    pub flags: PageFlags,
}

/// Validate a kernel mapping independently of any page-table memory backend.
///
/// Keeping this check outside the generic builder makes the policy reusable
/// by boot planning and host-side validation without requiring a dummy
/// `TableMemory` type parameter.
pub fn validate_kernel_mapping(mapping: &KernelMapping) -> Result<(), KernelPageBuildError> {
    if mapping.page_count == 0
        || mapping.virtual_start & (KERNEL_PAGE_SIZE - 1) != 0
        || mapping.physical_start & (KERNEL_PAGE_SIZE - 1) != 0
        || !valid_mapping(mapping.physical_start, mapping.flags)
    {
        return Err(KernelPageBuildError::InvalidMapping);
    }
    if mapping.flags.contains(PageFlags::HUGE) {
        return Err(KernelPageBuildError::HugeForbidden);
    }
    if mapping.flags.contains(PageFlags::USER) {
        return Err(KernelPageBuildError::UserFlagForbidden);
    }
    if !is_canonical_x86_64_48(mapping.virtual_start) {
        return Err(KernelPageBuildError::InvalidMapping);
    }
    if pml4_index(mapping.virtual_start) < 256 {
        return Err(KernelPageBuildError::UserHalfForbidden);
    }
    let bytes = mapping
        .page_count
        .checked_mul(KERNEL_PAGE_SIZE)
        .ok_or(KernelPageBuildError::Overflow)?;
    let virtual_end = mapping
        .virtual_start
        .checked_add(bytes)
        .ok_or(KernelPageBuildError::Overflow)?;
    let physical_end = mapping
        .physical_start
        .checked_add(bytes)
        .ok_or(KernelPageBuildError::Overflow)?;
    let physical_limit = PageTableEntry::ADDRESS_MASK + KERNEL_PAGE_SIZE;
    if physical_end > physical_limit {
        return Err(KernelPageBuildError::Overflow);
    }
    let region = kernel_region_of(mapping.virtual_start)
        .ok_or(KernelPageBuildError::OutsidePlannedRegion)?;
    if kernel_region_of(virtual_end - 1) != Some(region) {
        return Err(KernelPageBuildError::CrossesRegionBoundary);
    }
    check_region_policy(region, mapping.flags)
}

/// Flag policy enforced for every kernel-half leaf mapping.
fn check_region_policy(
    region: KernelRegionKind,
    flags: PageFlags,
) -> Result<(), KernelPageBuildError> {
    if flags.contains(PageFlags::USER) {
        return Err(KernelPageBuildError::UserFlagForbidden);
    }
    if flags.contains(PageFlags::HUGE) {
        return Err(KernelPageBuildError::HugeForbidden);
    }
    // W^X: any writable kernel mapping must be non-executable.
    if flags.contains(PageFlags::WRITABLE) && !flags.contains(PageFlags::NO_EXECUTE) {
        return Err(KernelPageBuildError::WritableExecutable);
    }
    // Kernel text is executable and read-only by construction; a writable
    // mapping inside the text region can never be part of a safe plan.
    if region == KernelRegionKind::KernelText && flags.contains(PageFlags::WRITABLE) {
        return Err(KernelPageBuildError::WritableTextRegion);
    }
    Ok(())
}

/// Intermediate entries are supervisor-only and writable so leaf permissions
/// remain the effective access boundary. GLOBAL is a leaf attribute and is
/// intentionally not set on non-leaf entries.
const KERNEL_INTERMEDIATE_FLAGS: PageFlags = PageFlags::PRESENT.union(PageFlags::WRITABLE);

/// Upper bound on intermediate table frames tracked by one builder.
const MAX_TRACKED_TABLES: usize = 512;

/// Builds supervisor-only page tables for the kernel half of one address
/// space, inside the planned higher-half regions.
pub struct KernelPageTableBuilder<'m, M: TableMemory> {
    pml4: u64,
    memory: &'m mut M,
    allocated: [u64; MAX_TRACKED_TABLES],
    allocated_count: usize,
}

impl<'m, M: TableMemory> KernelPageTableBuilder<'m, M> {
    /// Create a builder over a page-aligned, nonzero PML4 root frame.
    pub fn new(pml4_physical: u64, memory: &'m mut M) -> Result<Self, KernelPageBuildError> {
        if pml4_physical == 0 || !valid_mapping(pml4_physical, PageFlags::PRESENT) {
            return Err(KernelPageBuildError::InvalidRoot);
        }
        Ok(Self {
            pml4: pml4_physical,
            memory,
            allocated: [0; MAX_TRACKED_TABLES],
            allocated_count: 0,
        })
    }

    /// The PML4 root frame this builder installs into.
    pub const fn pml4_physical(&self) -> u64 {
        self.pml4
    }

    /// Number of intermediate table frames allocated so far.
    pub const fn allocated_table_count(&self) -> usize {
        self.allocated_count
    }

    /// The intermediate table frames allocated so far, in allocation order.
    /// The PML4 root itself is not included: it is caller-owned.
    pub fn allocated_tables(&self) -> &[u64] {
        &self.allocated[..self.allocated_count]
    }

    fn record(&mut self, frame: u64) -> Result<(), KernelPageBuildError> {
        if frame == 0 || !valid_mapping(frame, PageFlags::PRESENT) {
            return Err(KernelPageBuildError::InvalidTableFrame);
        }
        if frame == self.pml4 || self.allocated[..self.allocated_count].contains(&frame) {
            return Err(KernelPageBuildError::DuplicateTableFrame);
        }
        if self.allocated_count >= MAX_TRACKED_TABLES {
            return Err(KernelPageBuildError::TableLimit);
        }
        self.allocated[self.allocated_count] = frame;
        self.allocated_count += 1;
        Ok(())
    }

    /// Check every existing path before installing any page in this mapping.
    /// A rejected preflight leaves every table unchanged, so a partially
    /// installed range can never be observed.
    pub fn preflight(&self, mapping: &KernelMapping) -> Result<(), KernelPageBuildError> {
        validate_kernel_mapping(mapping)?;
        let mut page = 0u64;
        while page < mapping.page_count {
            let offset = page
                .checked_mul(KERNEL_PAGE_SIZE)
                .ok_or(KernelPageBuildError::Overflow)?;
            let vpage = mapping
                .virtual_start
                .checked_add(offset)
                .ok_or(KernelPageBuildError::Overflow)?;
            let idx = indices(vpage);
            if idx[0] < 256 {
                return Err(KernelPageBuildError::UserHalfForbidden);
            }

            let mut table_phys = self.pml4;
            let mut level = 0usize;
            while level < 3 {
                let entry = self
                    .memory
                    .table(table_phys)
                    .ok_or(KernelPageBuildError::MemoryUnreadable)?
                    .get(idx[level])
                    .ok_or(KernelPageBuildError::MemoryUnreadable)?;
                if !entry.is_present() {
                    break;
                }
                if entry.flags().contains(PageFlags::HUGE) {
                    return Err(KernelPageBuildError::HugePageConflict);
                }
                if entry.flags().contains(PageFlags::USER)
                    || !entry.flags().contains(PageFlags::WRITABLE)
                {
                    return Err(KernelPageBuildError::IntermediatePermissionConflict);
                }
                let child = entry.address();
                if child == 0 || !valid_mapping(child, PageFlags::PRESENT) {
                    return Err(KernelPageBuildError::InvalidTableFrame);
                }
                table_phys = child;
                level += 1;
            }

            if level == 3 {
                let leaf = self
                    .memory
                    .table(table_phys)
                    .ok_or(KernelPageBuildError::MemoryUnreadable)?
                    .get(idx[3])
                    .ok_or(KernelPageBuildError::MemoryUnreadable)?;
                if leaf.is_present() {
                    return Err(KernelPageBuildError::AlreadyMapped);
                }
            }
            page += 1;
        }
        Ok(())
    }

    /// Validate the full batch, reject virtual-range overlaps, and preflight
    /// existing page-table entries before mutation. If a later allocation or
    /// backend access fails during installation, partial tables may remain;
    /// the caller must keep the root unpublished and discard it on any error.
    pub fn install_all(&mut self, mappings: &[KernelMapping]) -> Result<(), KernelPageBuildError> {
        for mapping in mappings {
            self.preflight(mapping)?;
        }

        let mut i = 0usize;
        while i < mappings.len() {
            let a_bytes = mappings[i]
                .page_count
                .checked_mul(KERNEL_PAGE_SIZE)
                .ok_or(KernelPageBuildError::Overflow)?;
            let a_end = mappings[i]
                .virtual_start
                .checked_add(a_bytes)
                .ok_or(KernelPageBuildError::Overflow)?;
            let mut j = i + 1;
            while j < mappings.len() {
                let b_bytes = mappings[j]
                    .page_count
                    .checked_mul(KERNEL_PAGE_SIZE)
                    .ok_or(KernelPageBuildError::Overflow)?;
                let b_end = mappings[j]
                    .virtual_start
                    .checked_add(b_bytes)
                    .ok_or(KernelPageBuildError::Overflow)?;
                if mappings[i].virtual_start < b_end && mappings[j].virtual_start < a_end {
                    return Err(KernelPageBuildError::OverlappingMappings);
                }
                j += 1;
            }
            i += 1;
        }

        for mapping in mappings {
            self.install_pages(*mapping)?;
        }
        Ok(())
    }

    /// Validate and install one mapping page by page, creating missing
    /// intermediate tables. This operation does not roll back if a backend
    /// error occurs after an earlier page was installed; on error, the caller
    /// must keep the root unpublished and discard it.
    pub fn install_pages(&mut self, mapping: KernelMapping) -> Result<(), KernelPageBuildError> {
        validate_kernel_mapping(&mapping)?;
        let mut page = 0u64;
        while page < mapping.page_count {
            let offset = page
                .checked_mul(KERNEL_PAGE_SIZE)
                .ok_or(KernelPageBuildError::Overflow)?;
            let vpage = mapping
                .virtual_start
                .checked_add(offset)
                .ok_or(KernelPageBuildError::Overflow)?;
            let ppage = mapping
                .physical_start
                .checked_add(offset)
                .ok_or(KernelPageBuildError::Overflow)?;
            self.install_page(vpage, ppage, mapping.flags)?;
            page += 1;
        }
        Ok(())
    }

    /// Install one 4 KiB kernel page. This builder never writes the user
    /// half of the PML4 and never sets USER bits.
    fn install_page(
        &mut self,
        vpage: u64,
        ppage: u64,
        flags: PageFlags,
    ) -> Result<(), KernelPageBuildError> {
        let idx = indices(vpage);
        if idx[0] < 256 {
            return Err(KernelPageBuildError::UserHalfForbidden);
        }
        check_region_policy(
            kernel_region_of(vpage).ok_or(KernelPageBuildError::OutsidePlannedRegion)?,
            flags,
        )?;
        let mut table_phys = self.pml4;
        let mut level = 0usize;
        while level < 3 {
            let next = {
                let table = self
                    .memory
                    .table(table_phys)
                    .ok_or(KernelPageBuildError::MemoryUnreadable)?;
                let entry = table
                    .get(idx[level])
                    .ok_or(KernelPageBuildError::MemoryUnreadable)?;
                if entry.is_present() {
                    if entry.flags().contains(PageFlags::HUGE) {
                        return Err(KernelPageBuildError::HugePageConflict);
                    }
                    if entry.flags().contains(PageFlags::USER)
                        || !entry.flags().contains(PageFlags::WRITABLE)
                    {
                        return Err(KernelPageBuildError::IntermediatePermissionConflict);
                    }
                    let child = entry.address();
                    if child == 0 || !valid_mapping(child, PageFlags::PRESENT) {
                        return Err(KernelPageBuildError::InvalidTableFrame);
                    }
                    child
                } else {
                    0
                }
            };
            if next == 0 {
                // Check tracker capacity before asking the backend for a
                // frame so an untrackable frame can never leak.
                if self.allocated_count >= MAX_TRACKED_TABLES {
                    return Err(KernelPageBuildError::TableLimit);
                }
                let frame = self
                    .memory
                    .alloc_table()
                    .ok_or(KernelPageBuildError::OutOfFrames)?;
                self.record(frame)?;
                let child_table = self
                    .memory
                    .table_mut(frame)
                    .ok_or(KernelPageBuildError::MemoryUnreadable)?;
                // Do not trust stale contents from a buggy frame backend.
                let mut slot = 0usize;
                while slot < 512 {
                    child_table.clear(slot);
                    slot += 1;
                }
                let parent = self
                    .memory
                    .table_mut(table_phys)
                    .ok_or(KernelPageBuildError::MemoryUnreadable)?;
                if !parent.set(
                    idx[level],
                    PageTableEntry::new(frame, KERNEL_INTERMEDIATE_FLAGS),
                ) {
                    return Err(KernelPageBuildError::MemoryUnreadable);
                }
                table_phys = frame;
            } else {
                table_phys = next;
            }
            level += 1;
        }
        let table = self
            .memory
            .table_mut(table_phys)
            .ok_or(KernelPageBuildError::MemoryUnreadable)?;
        if table
            .get(idx[3])
            .ok_or(KernelPageBuildError::MemoryUnreadable)?
            .is_present()
        {
            return Err(KernelPageBuildError::AlreadyMapped);
        }
        if !table.set(idx[3], PageTableEntry::new(ppage, flags)) {
            return Err(KernelPageBuildError::MemoryUnreadable);
        }
        Ok(())
    }

    /// Walk a kernel-half virtual page exactly as the hardware would and
    /// return the present leaf entry, if any. User-half and noncanonical
    /// addresses return `None`; this is a diagnostics helper, not a
    /// dereference.
    pub fn leaf_entry(&self, virtual_address: u64) -> Option<PageTableEntry> {
        if pml4_index(virtual_address) < 256 || !is_canonical_x86_64_48(virtual_address) {
            return None;
        }
        let idx = indices(virtual_address);
        let mut table_phys = self.pml4;
        let mut level = 0usize;
        while level < 3 {
            let entry = self.memory.table(table_phys)?.get(idx[level])?;
            if !entry.is_present() || entry.flags().contains(PageFlags::HUGE) {
                return None;
            }
            table_phys = entry.address();
            if table_phys == 0 {
                return None;
            }
            level += 1;
        }
        let leaf = self.memory.table(table_phys)?.get(idx[3])?;
        if leaf.is_present() { Some(leaf) } else { None }
    }

    /// True if nothing is present below PML4 index 256. The user half of the
    /// root must stay untouched by construction; this makes that property
    /// observable in tests and boot diagnostics.
    pub fn user_half_is_untouched(&self) -> bool {
        for index in 0..256 {
            match self.memory.table(self.pml4).and_then(|t| t.get(index)) {
                Some(entry) if entry.is_present() => return false,
                Some(_) => {}
                None => return false,
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::kernel_map::{
        KERNEL_HEAP_BASE, KERNEL_PERCPU_BASE, KERNEL_TEXT_BASE, KERNEL_TEXT_SIZE,
    };
    use crate::memory::paging::PageTable;

    const KERNEL_MAP_BASE: u64 = DIRECT_MAP_BASE;

    fn vm(
        virtual_start: u64,
        physical_start: u64,
        page_count: u64,
        flags: PageFlags,
    ) -> KernelMapping {
        KernelMapping {
            virtual_start,
            physical_start,
            page_count,
            flags,
        }
    }

    /// Minimal fixed-capacity table memory backend for host tests.
    struct FakeMemory {
        next_frame: u64,
        phys: [u64; FAKE_TABLES],
        tables: [PageTable; FAKE_TABLES],
        live: [bool; FAKE_TABLES],
        used: usize,
    }

    const FAKE_TABLES: usize = 64;

    impl FakeMemory {
        fn new() -> Self {
            Self {
                next_frame: 0x10_0000,
                phys: [0; FAKE_TABLES],
                tables: [const { PageTable::new() }; FAKE_TABLES],
                live: [false; FAKE_TABLES],
                used: 0,
            }
        }

        fn slot(&self, physical: u64) -> Option<usize> {
            let mut i = 0;
            while i < self.used {
                if self.phys[i] == physical {
                    return Some(i);
                }
                i += 1;
            }
            None
        }

        fn exhaust(&mut self) {
            while self.alloc_table().is_some() {}
        }
    }

    impl TableMemory for FakeMemory {
        fn alloc_table(&mut self) -> Option<u64> {
            if self.used >= FAKE_TABLES {
                return None;
            }
            let phys = self.next_frame;
            self.next_frame += KERNEL_PAGE_SIZE;
            self.phys[self.used] = phys;
            self.tables[self.used] = PageTable::new();
            self.live[self.used] = true;
            self.used += 1;
            Some(phys)
        }

        fn table_mut(&mut self, physical: u64) -> Option<&mut PageTable> {
            let slot = self.slot(physical)?;
            if !self.live[slot] {
                return None;
            }
            Some(&mut self.tables[slot])
        }

        fn table(&self, physical: u64) -> Option<&PageTable> {
            let slot = self.slot(physical)?;
            if !self.live[slot] {
                return None;
            }
            Some(&self.tables[slot])
        }
    }

    /// Kernel text leaf flags: present, executable (no NX), read-only.
    const TEXT: PageFlags = PageFlags::PRESENT;
    /// Read-only, non-executable kernel data flags.
    const RNX: PageFlags = PageFlags::PRESENT.union(PageFlags::NO_EXECUTE);
    /// Writable, non-executable kernel data flags.
    const WNX: PageFlags = PageFlags::PRESENT
        .union(PageFlags::WRITABLE)
        .union(PageFlags::NO_EXECUTE);
    const WNX_GLOBAL: PageFlags = WNX.union(PageFlags::GLOBAL);

    #[test]
    fn user_half_install_is_forbidden_everywhere() {
        let mapping = vm(0x0000_1000, 0x30_0000, 1, PageFlags::PRESENT);
        assert_eq!(
            validate_kernel_mapping(&mapping),
            Err(KernelPageBuildError::UserHalfForbidden)
        );
        let mut memory = FakeMemory::new();
        let root = memory.alloc_table().expect("root frame");
        let mut builder = KernelPageTableBuilder::new(root, &mut memory).unwrap();
        assert_eq!(
            builder.install_all(&[mapping]),
            Err(KernelPageBuildError::UserHalfForbidden)
        );
        assert_eq!(builder.allocated_table_count(), 0);
    }

    #[test]
    fn user_flag_is_rejected() {
        let mapping = vm(
            KERNEL_TEXT_BASE,
            0x30_0000,
            1,
            PageFlags::PRESENT.union(PageFlags::USER),
        );
        assert_eq!(
            validate_kernel_mapping(&mapping),
            Err(KernelPageBuildError::UserFlagForbidden)
        );
    }

    #[test]
    fn huge_flag_is_rejected() {
        let mapping = vm(
            KERNEL_MAP_BASE,
            0x30_0000,
            1,
            PageFlags::PRESENT.union(PageFlags::HUGE),
        );
        assert_eq!(
            validate_kernel_mapping(&mapping),
            Err(KernelPageBuildError::HugeForbidden)
        );
    }

    #[test]
    fn mapping_outside_planned_region_is_rejected() {
        // Inside the gap between the text region end and the heap start.
        let mapping = vm(0xffff_c110_0000_0000, 0x30_0000, 1, RNX);
        assert_eq!(
            validate_kernel_mapping(&mapping),
            Err(KernelPageBuildError::OutsidePlannedRegion)
        );
    }

    #[test]
    fn mapping_crossing_region_end_is_rejected() {
        let base = KERNEL_TEXT_BASE + KERNEL_TEXT_SIZE - KERNEL_PAGE_SIZE;
        let mapping = vm(base, 0x30_0000, 2, RNX);
        assert_eq!(
            validate_kernel_mapping(&mapping),
            Err(KernelPageBuildError::CrossesRegionBoundary)
        );
    }

    #[test]
    fn writable_kernel_text_is_rejected() {
        let mapping = vm(KERNEL_TEXT_BASE, 0x30_0000, 1, WNX);
        assert_eq!(
            validate_kernel_mapping(&mapping),
            Err(KernelPageBuildError::WritableTextRegion)
        );
    }

    #[test]
    fn writable_without_nx_is_rejected_everywhere() {
        let mapping = vm(
            KERNEL_HEAP_BASE,
            0x30_0000,
            1,
            PageFlags::PRESENT.union(PageFlags::WRITABLE),
        );
        assert_eq!(
            validate_kernel_mapping(&mapping),
            Err(KernelPageBuildError::WritableExecutable)
        );
    }

    #[test]
    fn misaligned_zero_and_bad_flag_requests_fail_closed() {
        let misaligned = vm(KERNEL_MAP_BASE + 0x800, 0x30_0000, 1, RNX);
        let empty = vm(KERNEL_MAP_BASE, 0x30_0000, 0, RNX);
        let unaligned_phys = vm(KERNEL_MAP_BASE, 0x30_0123, 1, RNX);
        for mapping in [misaligned, empty, unaligned_phys] {
            assert_eq!(
                validate_kernel_mapping(&mapping),
                Err(KernelPageBuildError::InvalidMapping)
            );
        }
    }

    #[test]
    fn install_writes_supervisor_only_tables_and_readback_works() {
        let mut memory = FakeMemory::new();
        let root = memory.alloc_table().expect("root frame");
        let mappings = [
            vm(KERNEL_MAP_BASE, 0x30_0000, 2, WNX_GLOBAL),
            vm(KERNEL_TEXT_BASE, 0x40_0000, 2, TEXT),
            vm(KERNEL_HEAP_BASE, 0x50_0000, 1, WNX),
            vm(KERNEL_PERCPU_BASE, 0x60_0000, 1, RNX),
        ];
        let mut builder = KernelPageTableBuilder::new(root, &mut memory).unwrap();
        assert_eq!(builder.install_all(&mappings), Ok(()));
        assert!(builder.user_half_is_untouched());

        for region_base in [
            KERNEL_MAP_BASE,
            KERNEL_MAP_BASE + KERNEL_PAGE_SIZE,
            KERNEL_TEXT_BASE,
            KERNEL_HEAP_BASE,
            KERNEL_PERCPU_BASE,
        ] {
            assert!(
                builder.leaf_entry(region_base).is_some(),
                "no leaf at {region_base:#x}"
            );
        }
        assert!(builder.leaf_entry(0x0000_1000).is_none());
        assert!(builder.leaf_entry(0xffff_c110_0000_0000).is_none());

        let direct_leaf = builder.leaf_entry(KERNEL_MAP_BASE).unwrap();
        assert_eq!(direct_leaf.address(), 0x30_0000);
        assert!(!direct_leaf.flags().contains(PageFlags::USER));
        assert!(direct_leaf.flags().contains(PageFlags::NO_EXECUTE));
        assert!(direct_leaf.flags().contains(PageFlags::WRITABLE));
        assert!(direct_leaf.flags().contains(PageFlags::GLOBAL));

        let text_leaf = builder.leaf_entry(KERNEL_TEXT_BASE).unwrap();
        assert!(!text_leaf.flags().contains(PageFlags::WRITABLE));
        assert!(!text_leaf.flags().contains(PageFlags::NO_EXECUTE));

        // Intermediate entries are supervisor-only; GLOBAL is applied only
        // to leaf entries when requested by the mapping policy.
        let idx = indices(KERNEL_MAP_BASE);
        let pml4 = memory.table(root).expect("pml4 readable");
        let entry = pml4.get(idx[0]).expect("pml4 entry installed");
        assert!(!entry.flags().contains(PageFlags::USER));
        assert!(entry.flags().contains(PageFlags::WRITABLE));
        assert!(!entry.flags().contains(PageFlags::GLOBAL));
        assert_eq!(entry.address() & 0xfff, 0);
    }

    #[test]
    fn double_install_is_rejected_without_growth() {
        let mut memory = FakeMemory::new();
        let root = memory.alloc_table().expect("root frame");
        let mapping = vm(KERNEL_MAP_BASE, 0x30_0000, 2, WNX);
        let mut builder = KernelPageTableBuilder::new(root, &mut memory).unwrap();
        assert_eq!(builder.install_all(&[mapping]), Ok(()));
        let before = builder.allocated_table_count();
        let conflicting = vm(KERNEL_MAP_BASE, 0x70_0000, 2, WNX);
        assert_eq!(
            builder.install_all(&[conflicting]),
            Err(KernelPageBuildError::AlreadyMapped)
        );
        assert_eq!(builder.allocated_table_count(), before);
    }

    #[test]
    fn overlapping_batch_mappings_are_rejected_before_mutation() {
        let mut memory = FakeMemory::new();
        let root = memory.alloc_table().expect("root frame");
        let first = vm(KERNEL_MAP_BASE, 0x30_0000, 2, WNX);
        let overlap = vm(KERNEL_MAP_BASE + KERNEL_PAGE_SIZE, 0x70_0000, 1, WNX);
        let mut builder = KernelPageTableBuilder::new(root, &mut memory).unwrap();

        assert_eq!(
            builder.install_all(&[first, overlap]),
            Err(KernelPageBuildError::OverlappingMappings)
        );
        assert_eq!(builder.allocated_table_count(), 0);
        assert!(builder.leaf_entry(KERNEL_MAP_BASE).is_none());
        assert!(builder.user_half_is_untouched());
    }

    #[test]
    fn a_bad_later_mapping_blocks_the_whole_batch() {
        let mut memory = FakeMemory::new();
        let root = memory.alloc_table().expect("root frame");
        let good = vm(KERNEL_MAP_BASE, 0x30_0000, 2, WNX);
        let bad = vm(0xffff_c110_0000_0000, 0x30_0000, 1, RNX);
        let mut builder = KernelPageTableBuilder::new(root, &mut memory).unwrap();
        assert_eq!(
            builder.install_all(&[good, bad]),
            Err(KernelPageBuildError::OutsidePlannedRegion)
        );
        assert_eq!(builder.allocated_table_count(), 0);
        assert!(builder.leaf_entry(KERNEL_MAP_BASE).is_none());
    }

    #[test]
    fn direct_map_fit_accepts_in_window_ranges() {
        assert_eq!(fits_direct_map_window(0, 0x1000), Ok(()));
        assert_eq!(fits_direct_map_window(0x30_0000, 0x30_0000), Ok(()));
        assert_eq!(
            fits_direct_map_window(DIRECT_MAP_MAX_PHYSICAL - 0x1000, 0x1000),
            Ok(())
        );
    }

    #[test]
    fn direct_map_fit_rejects_out_of_window_empty_and_overflowing_ranges() {
        assert_eq!(
            fits_direct_map_window(DIRECT_MAP_MAX_PHYSICAL, 0x1000),
            Err(DirectMapFitError::BeyondWindow {
                end: DIRECT_MAP_MAX_PHYSICAL + 0x1000
            })
        );
        assert_eq!(
            fits_direct_map_window(DIRECT_MAP_MAX_PHYSICAL - 0x800, 0x1000),
            Err(DirectMapFitError::BeyondWindow {
                end: DIRECT_MAP_MAX_PHYSICAL + 0x800
            })
        );
        assert_eq!(
            fits_direct_map_window(0, 0),
            Err(DirectMapFitError::EmptyRange)
        );
        assert!(matches!(
            fits_direct_map_window(0xffff_ffff_ffff_f000, 0x2000),
            Err(DirectMapFitError::Overflow)
        ));
    }

    #[test]
    fn ram_range_validation_names_the_first_offender() {
        let ranges = [
            (0u64, 0x30_0000u64),
            (0x50_0000, 0x30_0000),
            (0x90_0000, 0x30_0000),
        ];
        assert_eq!(validate_ram_ranges(&ranges), Ok(()));
        let bad = [(0u64, 0x30_0000u64), (DIRECT_MAP_MAX_PHYSICAL, 0x30_0000)];
        assert_eq!(
            validate_ram_ranges(&bad),
            Err((
                1,
                DirectMapFitError::BeyondWindow {
                    end: DIRECT_MAP_MAX_PHYSICAL + 0x30_0000
                }
            ))
        );
    }

    #[test]
    fn invalid_root_is_rejected() {
        let mut memory = FakeMemory::new();
        assert!(matches!(
            KernelPageTableBuilder::new(0, &mut memory),
            Err(KernelPageBuildError::InvalidRoot)
        ));
        assert!(matches!(
            KernelPageTableBuilder::new(1, &mut memory),
            Err(KernelPageBuildError::InvalidRoot)
        ));
    }

    #[test]
    fn out_of_frames_fails_closed() {
        let mut memory = FakeMemory::new();
        let root = memory.alloc_table().expect("root frame");
        memory.exhaust();
        let mapping = vm(KERNEL_MAP_BASE, 0x30_0000, 1, WNX);
        let mut builder = KernelPageTableBuilder::new(root, &mut memory).unwrap();
        assert_eq!(
            builder.install_all(&[mapping]),
            Err(KernelPageBuildError::OutOfFrames)
        );
        assert!(builder.user_half_is_untouched());
    }
}
