#![no_std]

//! Construction of real four-level x86_64 page tables for the user half of a
//! per-process address space.
//!
//! This module is the first hardware-facing layer behind the
//! `AddressSpace` contract object: it walks every registered user mapping and
//! installs PML4 -> PDPT -> PD -> PT entries through a caller-supplied
//! `TableMemory` backend. It does not touch CR3, does not assume an identity
//! map, and never writes entries into the kernel half of the PML4 (indexes
//! 256..512 stay untouched, preserving supervisor-only kernel mappings).
//!
//! Explicit non-goals of this layer:
//! - No frame deallocation or reference counting: allocated table frames are
//!   recorded so the owner can reclaim them, but teardown is not performed
//!   here.
//! - No TLB invalidation: nothing is activated while building.
//! - No W^X policy enforcement yet: leaf entries carry exactly the flags of
//!   the registered mapping. NX policy at all levels is the next step and is
//!   documented as unimplemented.

use super::address_space::{ADDRESS_SPACE_PAGE_SIZE, AddressSpace, UserMapping};
use super::paging::{PageFlags, PageTable, PageTableEntry, indices, valid_user_mapping};

/// Errors returned while building hardware page tables for an address space.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PageBuildError {
    /// The PML4 root frame is zero or not page aligned.
    InvalidRoot,
    /// The frame source is exhausted; the build fails closed.
    OutOfFrames,
    /// More intermediate tables were allocated than the bounded tracker can
    /// record.
    TableLimit,
    /// A table frame could not be accessed through the memory backend.
    MemoryUnreadable,
    /// A mapping would install into the kernel half of the PML4.
    KernelHalfForbidden,
    /// Address arithmetic overflowed while walking a mapping range.
    Overflow,
    /// The mapping is not a valid user mapping.
    InvalidMapping,
}

/// Memory contract used to build and inspect page tables.
///
/// The kernel wires this to its physical mapper; tests wire it to an in-memory
/// frame pool. Implementations must hand out zeroed, page-aligned frames and
/// must be able to resolve every frame they handed out (including the PML4
/// root passed to the builder).
pub trait TableMemory {
    /// Allocate one zeroed page-table frame and return its physical address.
    fn alloc_table(&mut self) -> Option<u64>;
    /// Mutable access to the table stored at a physical frame address.
    fn table_mut(&mut self, physical: u64) -> Option<&mut PageTable>;
    /// Read-only access to the table stored at a physical frame address.
    fn table(&self, physical: u64) -> Option<&PageTable>;
}

/// Flags installed on intermediate (non-leaf) entries. They must carry USER so
/// user code can traverse the hierarchy, and PRESENT|WRITABLE so the leaf
/// permissions are the only permission boundary.
const INTERMEDIATE_FLAGS: PageFlags = PageFlags::PRESENT
    .union(PageFlags::WRITABLE)
    .union(PageFlags::USER);

/// Upper bound on intermediate table frames tracked by one builder.
const MAX_TRACKED_TABLES: usize = 128;

/// Builds hardware page-table entries for the user half of one address space.
pub struct UserPageTableBuilder<'m, M: TableMemory> {
    pml4: u64,
    memory: &'m mut M,
    allocated: [u64; MAX_TRACKED_TABLES],
    allocated_count: usize,
}

impl<'m, M: TableMemory> UserPageTableBuilder<'m, M> {
    /// Create a builder over a page-aligned, nonzero PML4 root frame.
    pub fn new(pml4_physical: u64, memory: &'m mut M) -> Result<Self, PageBuildError> {
        if pml4_physical == 0 || pml4_physical & (ADDRESS_SPACE_PAGE_SIZE - 1) != 0 {
            return Err(PageBuildError::InvalidRoot);
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

    fn record(&mut self, frame: u64) -> Result<(), PageBuildError> {
        if frame == 0 || self.allocated_count >= MAX_TRACKED_TABLES {
            return Err(PageBuildError::TableLimit);
        }
        self.allocated[self.allocated_count] = frame;
        self.allocated_count += 1;
        Ok(())
    }

    /// Install one 4 KiB page, creating missing intermediate tables.
    fn install_page(
        &mut self,
        vpage: u64,
        ppage: u64,
        flags: PageFlags,
    ) -> Result<(), PageBuildError> {
        let idx = indices(vpage);
        // Fail closed: this builder never writes the kernel half of the PML4.
        if idx[0] >= 256 {
            return Err(PageBuildError::KernelHalfForbidden);
        }
        let mut table_phys = self.pml4;
        let mut level = 0;
        while level < 3 {
            let next = {
                let table = self
                    .memory
                    .table(table_phys)
                    .ok_or(PageBuildError::MemoryUnreadable)?;
                let entry = table
                    .get(idx[level])
                    .ok_or(PageBuildError::MemoryUnreadable)?;
                if entry.is_present() {
                    entry.address()
                } else {
                    0
                }
            };
            if next == 0 {
                let frame = self
                    .memory
                    .alloc_table()
                    .ok_or(PageBuildError::OutOfFrames)?;
                self.record(frame)?;
                let table = self
                    .memory
                    .table_mut(table_phys)
                    .ok_or(PageBuildError::MemoryUnreadable)?;
                table.set(idx[level], PageTableEntry::new(frame, INTERMEDIATE_FLAGS));
                table_phys = frame;
            } else {
                table_phys = next;
            }
            level += 1;
        }
        let table = self
            .memory
            .table_mut(table_phys)
            .ok_or(PageBuildError::MemoryUnreadable)?;
        if !table.set(idx[3], PageTableEntry::new(ppage, flags)) {
            return Err(PageBuildError::MemoryUnreadable);
        }
        Ok(())
    }

    /// Install every page of one registered user mapping. Returns the number
    /// of leaf pages installed.
    pub fn install_mapping(&mut self, mapping: &UserMapping) -> Result<u64, PageBuildError> {
        if !valid_user_mapping(mapping.physical_start, mapping.flags) {
            return Err(PageBuildError::InvalidMapping);
        }
        let mut installed: u64 = 0;
        let mut page: u64 = 0;
        while page < mapping.page_count {
            let offset = page
                .checked_mul(ADDRESS_SPACE_PAGE_SIZE)
                .ok_or(PageBuildError::Overflow)?;
            let vpage = mapping
                .virtual_start
                .checked_add(offset)
                .ok_or(PageBuildError::Overflow)?;
            let ppage = mapping
                .physical_start
                .checked_add(offset)
                .ok_or(PageBuildError::Overflow)?;
            self.install_page(vpage, ppage, mapping.flags)?;
            installed += 1;
            page += 1;
        }
        Ok(installed)
    }

    /// Install every live mapping of an address space. Returns the total
    /// number of leaf pages installed.
    pub fn install_address_space<const N: usize>(
        &mut self,
        space: &AddressSpace<N>,
    ) -> Result<u64, PageBuildError> {
        let mut total: u64 = 0;
        for mapping in space.mappings() {
            let installed = self.install_mapping(mapping)?;
            total = total
                .checked_add(installed)
                .ok_or(PageBuildError::Overflow)?;
        }
        Ok(total)
    }

    /// Hardware-style four-level walk of the built tables. Returns the mapped
    /// physical page base for a virtual address, or None when any level is
    /// absent. Only the caller's memory backend is used; no identity map is
    /// assumed.
    pub fn translate(&self, virtual_address: u64) -> Option<u64> {
        let idx = indices(virtual_address);
        let mut table_phys = self.pml4;
        let mut level = 0;
        while level < 3 {
            let table = self.memory.table(table_phys)?;
            let entry = table.get(idx[level])?;
            if !entry.is_present() {
                return None;
            }
            table_phys = entry.address();
            level += 1;
        }
        let table = self.memory.table(table_phys)?;
        let entry = table.get(idx[3])?;
        if !entry.is_present() {
            return None;
        }
        Some(entry.address())
    }
}

#[cfg(test)]
mod tests {
    use super::super::address_space::AddressSpace;
    use super::*;

    const R: PageFlags = PageFlags::PRESENT.union(PageFlags::USER);
    const RW: PageFlags = R.union(PageFlags::WRITABLE);
    const RWNX: PageFlags = RW.union(PageFlags::NO_EXECUTE);

    /// In-memory pool of page-table frames, used as a TableMemory backend.
    struct FakeMemory {
        frames: [PageTable; 48],
        next: usize,
        base: u64,
    }

    impl FakeMemory {
        fn new() -> Self {
            Self {
                frames: core::array::from_fn(|_| PageTable::new()),
                next: 0,
                base: 0x0010_0000,
            }
        }

        fn index_of(&self, physical: u64) -> Option<usize> {
            if physical & (ADDRESS_SPACE_PAGE_SIZE - 1) != 0 {
                return None;
            }
            let offset = physical.checked_sub(self.base)?;
            if offset >= (self.frames.len() as u64) * ADDRESS_SPACE_PAGE_SIZE {
                return None;
            }
            Some((offset / ADDRESS_SPACE_PAGE_SIZE) as usize)
        }
    }

    impl TableMemory for FakeMemory {
        fn alloc_table(&mut self) -> Option<u64> {
            if self.next >= self.frames.len() {
                return None;
            }
            let physical = self.base + (self.next as u64) * ADDRESS_SPACE_PAGE_SIZE;
            self.frames[self.next] = PageTable::new();
            self.next += 1;
            Some(physical)
        }

        fn table_mut(&mut self, physical: u64) -> Option<&mut PageTable> {
            let index = self.index_of(physical)?;
            self.frames.get_mut(index)
        }

        fn table(&self, physical: u64) -> Option<&PageTable> {
            let index = self.index_of(physical)?;
            self.frames.get(index)
        }
    }

    #[test]
    fn invalid_roots_are_rejected() {
        let mut memory = FakeMemory::new();
        assert_eq!(
            UserPageTableBuilder::new(0, &mut memory).err(),
            Some(PageBuildError::InvalidRoot)
        );
        assert_eq!(
            UserPageTableBuilder::new(0x0010_0001, &mut memory).err(),
            Some(PageBuildError::InvalidRoot)
        );
    }

    #[test]
    fn installed_mappings_translate_like_the_contract_layer() {
        let mut memory = FakeMemory::new();
        let root = memory.alloc_table().unwrap();
        let mut space = AddressSpace::<8>::new(root).unwrap();
        space.map_user(0x0040_0000, 0x0090_0000, 3, RW).unwrap();
        space.map_user(0x0000_4000_0000, 0x00a0_0000, 2, R).unwrap();

        let mut builder = UserPageTableBuilder::new(root, &mut memory).unwrap();
        let installed = builder.install_address_space(&space).unwrap();
        assert_eq!(installed, 5);
        assert!(builder.allocated_table_count() >= 4);

        for (vpage, ppage) in [
            (0x0040_0000, 0x0090_0000),
            (0x0040_1000, 0x0090_1000),
            (0x0040_2000, 0x0090_2000),
            (0x0000_4000_0000, 0x00a0_0000),
            (0x0000_4000_1000, 0x00a0_1000),
        ] {
            assert_eq!(builder.translate(vpage), Some(ppage));
            assert_eq!(builder.translate(vpage), space.translate_page(vpage));
        }
        assert_eq!(builder.translate(0x0040_3000), None);
        assert_eq!(builder.translate(0x0000_4000_2000), None);
    }

    #[test]
    fn kernel_half_of_pml4_stays_untouched() {
        let mut memory = FakeMemory::new();
        let root = memory.alloc_table().unwrap();
        let mut space = AddressSpace::<4>::new(root).unwrap();
        space.map_user(0x1000, 0x9000, 1, RW).unwrap();
        let mut builder = UserPageTableBuilder::new(root, &mut memory).unwrap();
        builder.install_address_space(&space).unwrap();

        let pml4 = memory.table(root).unwrap();
        let mut index = 256;
        while index < 512 {
            assert!(!pml4.get(index).unwrap().is_present());
            index += 1;
        }
    }

    #[test]
    fn intermediate_entries_carry_user_traversal_flags() {
        let mut memory = FakeMemory::new();
        let root = memory.alloc_table().unwrap();
        let mut space = AddressSpace::<4>::new(root).unwrap();
        space.map_user(0x1000, 0x9000, 1, R).unwrap();
        let mut builder = UserPageTableBuilder::new(root, &mut memory).unwrap();
        builder.install_address_space(&space).unwrap();

        let pml4 = memory.table(root).unwrap();
        let pdpt_entry = pml4.get(0).unwrap();
        assert!(pdpt_entry.is_present());
        assert!(pdpt_entry.flags().contains(PageFlags::USER));
        assert!(pdpt_entry.flags().contains(PageFlags::WRITABLE));
    }

    #[test]
    fn leaf_flags_are_preserved_exactly() {
        let mut memory = FakeMemory::new();
        let root = memory.alloc_table().unwrap();
        let mut space = AddressSpace::<4>::new(root).unwrap();
        space.map_user(0x1000, 0x9000, 1, R).unwrap();
        space.map_user(0x2000, 0xa000, 1, RW).unwrap();
        space.map_user(0x3000, 0xb000, 1, RWNX).unwrap();
        let mut builder = UserPageTableBuilder::new(root, &mut memory).unwrap();
        builder.install_address_space(&space).unwrap();

        let ro_page = builder.translate(0x1000).unwrap();
        let ro_leaf = memory.table(ro_page).unwrap();
        assert!(
            !ro_leaf
                .get(0)
                .unwrap()
                .flags()
                .contains(PageFlags::WRITABLE)
        );
        let rw_leaf = memory.table(builder.translate(0x2000).unwrap()).unwrap();
        assert!(
            rw_leaf
                .get(0)
                .unwrap()
                .flags()
                .contains(PageFlags::WRITABLE)
        );
        let nx_leaf = memory.table(builder.translate(0x3000).unwrap()).unwrap();
        assert!(
            nx_leaf
                .get(0)
                .unwrap()
                .flags()
                .contains(PageFlags::NO_EXECUTE)
        );
    }

    #[test]
    fn out_of_frames_fails_closed_and_records_allocations() {
        struct TwoFrames {
            inner: FakeMemory,
        }
        impl TableMemory for TwoFrames {
            fn alloc_table(&mut self) -> Option<u64> {
                self.inner.alloc_table()
            }
            fn table_mut(&mut self, physical: u64) -> Option<&mut PageTable> {
                self.inner.table_mut(physical)
            }
            fn table(&self, physical: u64) -> Option<&PageTable> {
                self.inner.table(physical)
            }
        }

        let mut inner = FakeMemory::new();
        let root = inner.alloc_table().unwrap();
        let mut memory = TwoFrames { inner };
        let mut space = AddressSpace::<4>::new(root).unwrap();
        // Two mappings in different PML4 branches need more than two tables.
        space
            .map_user(0x0000_4000_0000, 0x00a0_0000, 1, RW)
            .unwrap();
        space
            .map_user(0x0000_8000_0000, 0x00b0_0000, 1, RW)
            .unwrap();
        let mut builder = UserPageTableBuilder::new(root, &mut memory).unwrap();
        assert_eq!(
            builder.install_address_space(&space).err(),
            Some(PageBuildError::OutOfFrames)
        );
        assert_eq!(builder.allocated_table_count(), 2);
    }

    #[test]
    fn kernel_half_mapping_is_rejected_at_install() {
        let mut memory = FakeMemory::new();
        let root = memory.alloc_table().unwrap();
        let mapping = UserMapping {
            virtual_start: 0xffff_8000_0000_0000,
            physical_start: 0x9000,
            page_count: 1,
            flags: RW,
        };
        let mut builder = UserPageTableBuilder::new(root, &mut memory).unwrap();
        assert_eq!(
            builder.install_mapping(&mapping).err(),
            Some(PageBuildError::KernelHalfForbidden)
        );
        assert_eq!(builder.allocated_table_count(), 0);
    }

    #[test]
    fn invalid_mapping_is_rejected_before_any_install() {
        let mut memory = FakeMemory::new();
        let root = memory.alloc_table().unwrap();
        let mapping = UserMapping {
            virtual_start: 0x1000,
            physical_start: 0x9000,
            page_count: 1,
            // Missing USER: not a valid user mapping.
            flags: PageFlags::PRESENT.union(PageFlags::WRITABLE),
        };
        let mut builder = UserPageTableBuilder::new(root, &mut memory).unwrap();
        assert_eq!(
            builder.install_mapping(&mapping).err(),
            Some(PageBuildError::InvalidMapping)
        );
        assert_eq!(builder.allocated_table_count(), 0);
    }
}
