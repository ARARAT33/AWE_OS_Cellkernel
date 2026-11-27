#![no_std]

//! Pre-activation full-root validation for CellKernel kernel-half page
//! tables.

//! Before a page-table root may ever be written into CR3, this module walks
//! the complete tree through the read-only half of the 'TableMemory'
//! backend and fails closed on any violation of the kernel root contract:

//! 1. **User half empty.** Any present entry at PML4 index < 256 is fatal.
//! 2. **Supervisor only.** The USER flag is forbidden on intermediate
//!    entries and on leaves; kernel mappings are never Ring-3 reachable.
//! 3. **No large pages.** HUGE leaves are forbidden at every level; the
//!    system is built on 4 KiB pages only.
//! 4. **W^X on leaves.** A writable leaf must carry NO_EXECUTE, and no
//!    leaf may be writable inside the kernel text region.
//! 5. **Planned regions only.** Every leaf virtual address must fall inside
//!    a planned higher-half kernel region.
//! 6. **Structural integrity.** Every intermediate child frame must be
//!    page-aligned, nonzero, and representable; a table frame reachable via
//!    two different paths (frame sharing or a cycle) is fatal.
//! 7. **Bounded work.** The walk visits at most a configured number of
//!    distinct tables and reports budget exhaustion instead of looping.

//! This module is read-only diagnostics: it never allocates, never writes
//! CR3, never invalidates the TLB, and never frees frames. Activation is a
//! separate, later step that must call 'RootValidator::validate' on the
//! exact root it is about to publish.

use super::kernel_map::{KernelRegionKind, kernel_region_of};
use super::page_table_build::TableMemory;
use super::paging::{PageFlags, PageTableEntry, valid_mapping};

/// Default budget on distinct tables visited by one validation walk.
pub const DEFAULT_ROOT_VALIDATION_TABLE_BUDGET: usize = 1024;

/// Hard upper bound on the fixed-capacity visited-frame set.
pub const MAX_ROOT_VALIDATION_TABLES: usize = 1024;

/// Fail-closed errors reported while validating a page-table root.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RootValidationError {
    /// The PML4 root frame is zero, unaligned, or not representable.
    InvalidRoot,
    /// A table could not be read through the memory backend.
    MemoryUnreadable,
    /// More distinct tables were reached than the configured budget allows.
    TableBudgetExceeded { visited: usize },
    /// A present entry exists at PML4 index < 256.
    UserHalfPopulated { index: usize },
    /// An intermediate entry carried the USER flag.
    UserFlagForbidden { level: u8 },
    /// An intermediate entry is a large-page leaf (HUGE set).
    HugeForbidden { level: u8 },
    /// An intermediate child frame is zero, unaligned, or unrepresentable.
    InvalidChildFrame { level: u8 },
    /// The same table frame is reachable through two different paths.
    SharedTableFrame { level: u8 },
    /// A leaf carried the USER flag.
    UserLeafForbidden { virtual_address: u64 },
    /// A leaf frame or flag word is invalid (misaligned, truncated, or
    /// overlapping the address mask).
    InvalidLeaf { virtual_address: u64 },
    /// A writable leaf without NO_EXECUTE (W^X violation).
    WritableExecutableLeaf { virtual_address: u64 },
    /// A writable leaf inside the kernel text region.
    WritableTextLeaf { virtual_address: u64 },
    /// A leaf virtual address outside every planned kernel region.
    LeafOutsidePlannedRegion { virtual_address: u64 },
}

/// Counts reported after a successful validation walk.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RootValidationReport {
    /// Distinct tables visited, including the PML4 root.
    pub tables_visited: usize,
    /// Present 4 KiB leaf pages found in the kernel half.
    pub leaf_pages: usize,
}

/// Validates one fully built page-table root before activation.
pub struct RootValidator {
    budget: usize,
}

impl RootValidator {
    /// Create a validator with a custom table budget. The budget must be at
    /// least 1 and at most MAX_ROOT_VALIDATION_TABLES because the visited
    /// set is a fixed-capacity, allocation-free array.
    pub fn new(budget: usize) -> Result<Self, RootValidationError> {
        if budget == 0 || budget > MAX_ROOT_VALIDATION_TABLES {
            return Err(RootValidationError::TableBudgetExceeded { visited: 0 });
        }
        Ok(Self { budget })
    }

    /// Create a validator with the default table budget.
    pub const fn with_default_budget() -> Self {
        Self {
            budget: DEFAULT_ROOT_VALIDATION_TABLE_BUDGET,
        }
    }

    /// The configured table budget.
    pub const fn budget(&self) -> usize {
        self.budget
    }

    /// Validate the complete tree rooted at 'pml4'. Read-only over the
    /// memory backend: no allocation, no mutation, no activation.
    pub fn validate<M: TableMemory>(
        &self,
        pml4: u64,
        memory: &M,
    ) -> Result<RootValidationReport, RootValidationError> {
        if pml4 == 0 || !valid_mapping(pml4, PageFlags::PRESENT) {
            return Err(RootValidationError::InvalidRoot);
        }
        let mut walker = Walker {
            memory,
            visited: [0; MAX_ROOT_VALIDATION_TABLES],
            visited_count: 0,
            leaf_pages: 0,
            budget: self.budget,
        };
        walker.visit_table(pml4, 0, KERNEL_HALF_PREFIX)?;
        Ok(RootValidationReport {
            tables_visited: walker.visited_count,
            leaf_pages: walker.leaf_pages,
        })
    }
}

/// Canonical base of the kernel half; every validated virtual address is
/// built from this prefix plus the table indices.
const KERNEL_HALF_PREFIX: u64 = 0xffff_8000_0000_0000;

struct Walker<'a, M: TableMemory> {
    memory: &'a M,
    visited: [u64; MAX_ROOT_VALIDATION_TABLES],
    visited_count: usize,
    leaf_pages: usize,
    budget: usize,
}

impl<'a, M: TableMemory> Walker<'a, M> {
    fn visit_table(
        &mut self,
        table_phys: u64,
        level: u8,
        prefix: u64,
    ) -> Result<(), RootValidationError> {
        if table_phys == 0 || !valid_mapping(table_phys, PageFlags::PRESENT) {
            return Err(RootValidationError::InvalidChildFrame { level });
        }
        let mut slot = 0usize;
        while slot < self.visited_count {
            if self.visited[slot] == table_phys {
                return Err(RootValidationError::SharedTableFrame { level });
            }
            slot += 1;
        }
        if self.visited_count >= self.budget {
            return Err(RootValidationError::TableBudgetExceeded {
                visited: self.visited_count,
            });
        }
        self.visited[self.visited_count] = table_phys;
        self.visited_count += 1;

        let table = self
            .memory
            .table(table_phys)
            .ok_or(RootValidationError::MemoryUnreadable)?;
        let shift: u32 = match level {
            0 => 39,
            1 => 30,
            2 => 21,
            _ => 12,
        };
        for index in 0..512usize {
            let entry = table
                .get(index)
                .ok_or(RootValidationError::MemoryUnreadable)?;
            if !entry.is_present() {
                continue;
            }
            if level == 0 && index < 256 {
                return Err(RootValidationError::UserHalfPopulated { index });
            }
            let virtual_address = prefix | ((index as u64) << shift);
            if level == 3 {
                self.check_leaf(entry, virtual_address)?;
            } else {
                if entry.flags().contains(PageFlags::USER) {
                    return Err(RootValidationError::UserFlagForbidden { level });
                }
                if entry.flags().contains(PageFlags::HUGE) {
                    return Err(RootValidationError::HugeForbidden { level });
                }
                let child = entry.address();
                self.visit_table(child, level + 1, virtual_address)?;
            }
        }
        Ok(())
    }

    fn check_leaf(
        &mut self,
        entry: PageTableEntry,
        virtual_address: u64,
    ) -> Result<(), RootValidationError> {
        if entry.flags().contains(PageFlags::USER) {
            return Err(RootValidationError::UserLeafForbidden { virtual_address });
        }
        if entry.flags().contains(PageFlags::HUGE) {
            return Err(RootValidationError::HugeForbidden { level: 3 });
        }
        if !valid_mapping(entry.address(), entry.flags()) {
            return Err(RootValidationError::InvalidLeaf { virtual_address });
        }
        let region = kernel_region_of(virtual_address)
            .ok_or(RootValidationError::LeafOutsidePlannedRegion { virtual_address })?;
        if entry.flags().contains(PageFlags::WRITABLE) {
            if !entry.flags().contains(PageFlags::NO_EXECUTE) {
                return Err(RootValidationError::WritableExecutableLeaf { virtual_address });
            }
            if region == KernelRegionKind::KernelText {
                return Err(RootValidationError::WritableTextLeaf { virtual_address });
            }
        }
        self.leaf_pages += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::kernel_map::{KERNEL_HEAP_BASE, KERNEL_MAP_BASE, KERNEL_TEXT_BASE};
    use crate::memory::kernel_page_table_build::{KernelMapping, KernelPageTableBuilder};
    use crate::memory::paging::{PageTable, indices, pml4_index as _};

    const FAKE_TABLES: usize = 64;

    struct FakeMemory {
        next_frame: u64,
        phys: [u64; FAKE_TABLES],
        tables: [PageTable; FAKE_TABLES],
        live: [bool; FAKE_TABLES],
        used: usize,
    }

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

        fn alloc_table(&mut self) -> Option<u64> {
            if self.used >= FAKE_TABLES {
                return None;
            }
            let phys = self.next_frame;
            self.next_frame += 0x1000;
            self.phys[self.used] = phys;
            self.tables[self.used] = PageTable::new();
            self.live[self.used] = true;
            self.used += 1;
            Some(phys)
        }

        fn insert_table_at(&mut self, physical: u64) {
            match self.slot(physical) {
                Some(slot) => self.live[slot] = true,
                None => {
                    assert!(self.used < FAKE_TABLES, "fake backend full");
                    self.phys[self.used] = physical;
                    self.tables[self.used] = PageTable::new();
                    self.live[self.used] = true;
                    self.used += 1;
                }
            }
        }

        fn set_entry(&mut self, table_phys: u64, index: usize, entry: PageTableEntry) {
            assert!(self.table_mut(table_phys).expect("table").set(index, entry));
        }

        fn install_chain(&mut self, virtual_address: u64) -> u64 {
            let root = self.alloc_table().expect("root");
            let pdpt = self.alloc_table().expect("pdpt");
            let pd = self.alloc_table().expect("pd");
            let pt = self.alloc_table().expect("pt");
            let inter = PageFlags::PRESENT.union(PageFlags::WRITABLE);
            let idx = indices(virtual_address);
            self.set_entry(root, idx[0], PageTableEntry::new(pdpt, inter));
            self.set_entry(pdpt, idx[1], PageTableEntry::new(pd, inter));
            self.set_entry(pd, idx[2], PageTableEntry::new(pt, inter));
            root
        }
    }

    impl TableMemory for FakeMemory {
        fn alloc_table(&mut self) -> Option<u64> {
            FakeMemory::alloc_table(self)
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

    const TEXT: PageFlags = PageFlags::PRESENT;
    const RNX: PageFlags = PageFlags::PRESENT.union(PageFlags::NO_EXECUTE);
    const WNX: PageFlags = PageFlags::PRESENT
        .union(PageFlags::WRITABLE)
        .union(PageFlags::NO_EXECUTE);

    fn vm(virtual_start: u64, physical_start: u64, page_count: u64, flags: PageFlags) -> KernelMapping {
        KernelMapping {
            virtual_start,
            physical_start,
            page_count,
            flags,
        }
    }

    /// Build a fully valid kernel-half root through the real builder.
    fn build_valid_root(memory: &mut FakeMemory) -> u64 {
        let root = memory.alloc_table().expect("root frame");
        let mappings = [
            vm(KERNEL_MAP_BASE, 0x30_0000, 2, WNX),
            vm(KERNEL_TEXT_BASE, 0x40_0000, 2, TEXT),
            vm(KERNEL_HEAP_BASE, 0x50_0000, 1, WNX),
        ];
        let mut builder = KernelPageTableBuilder::new(root, memory).unwrap();
        builder.install_all(&mappings).expect("install");
        root
    }

    #[test]
    fn invalid_root_is_rejected() {
        let memory = FakeMemory::new();
        let validator = RootValidator::with_default_budget();
        assert_eq!(
            validator.validate(0, &memory),
            Err(RootValidationError::InvalidRoot)
        );
        assert_eq!(
            validator.validate(1, &memory),
            Err(RootValidationError::InvalidRoot)
        );
    }

    #[test]
    fn empty_root_is_valid() {
        let mut memory = FakeMemory::new();
        let root = memory.alloc_table().expect("root frame");
        let report = RootValidator::with_default_budget()
            .validate(root, &memory)
            .expect("empty root is valid");
        assert_eq!(report.tables_visited, 1);
        assert_eq!(report.leaf_pages, 0);
    }

    #[test]
    fn fully_built_kernel_root_passes() {
        let mut memory = FakeMemory::new();
        let root = build_valid_root(&mut memory);
        let report = RootValidator::with_default_budget()
            .validate(root, &memory)
            .expect("built root is valid");
        assert_eq!(report.leaf_pages, 5);
        assert!(report.tables_visited > 1);
    }

    #[test]
    fn user_half_populated_is_rejected() {
        let mut memory = FakeMemory::new();
        let root = memory.alloc_table().expect("root frame");
        memory.set_entry(root, 0, PageTableEntry::new(0x20_0000, PageFlags::PRESENT));
        assert_eq!(
            RootValidator::with_default_budget().validate(root, &memory),
            Err(RootValidationError::UserHalfPopulated { index: 0 })
        );
    }

    #[test]
    fn user_flagged_intermediate_is_rejected() {
        let mut memory = FakeMemory::new();
        let root = memory.alloc_table().expect("root frame");
        let pdpt = memory.alloc_table().expect("pdpt");
        memory.set_entry(
            root,
            256,
            PageTableEntry::new(pdpt, PageFlags::PRESENT.union(PageFlags::USER)),
        );
        assert_eq!(
            RootValidator::with_default_budget().validate(root, &memory),
            Err(RootValidationError::UserFlagForbidden { level: 0 })
        );
    }

    #[test]
    fn huge_intermediate_is_rejected() {
        let mut memory = FakeMemory::new();
        let root = memory.alloc_table().expect("root frame");
        memory.set_entry(
            root,
            256,
            PageTableEntry::new(
                0x20_0000,
                PageFlags::PRESENT.union(PageFlags::WRITABLE).union(PageFlags::HUGE),
            ),
        );
        assert_eq!(
            RootValidator::with_default_budget().validate(root, &memory),
            Err(RootValidationError::HugeForbidden { level: 0 })
        );
    }

    #[test]
    fn unaligned_child_frame_is_rejected() {
        let mut memory = FakeMemory::new();
        let root = memory.alloc_table().expect("root frame");
        memory.set_entry(
            root,
            256,
            PageTableEntry::new(0x20_0800, PageFlags::PRESENT.union(PageFlags::WRITABLE)),
        );
        assert_eq!(
            RootValidator::with_default_budget().validate(root, &memory),
            Err(RootValidationError::InvalidChildFrame { level: 0 })
        );
    }

    #[test]
    fn shared_table_frame_is_rejected() {
        let mut memory = FakeMemory::new();
        let root = memory.alloc_table().expect("root frame");
        let pdpt = memory.alloc_table().expect("pdpt");
        let shared_flags = PageFlags::PRESENT.union(PageFlags::WRITABLE);
        memory.set_entry(root, 256, PageTableEntry::new(pdpt, shared_flags));
        memory.set_entry(root, 257, PageTableEntry::new(pdpt, shared_flags));
        assert_eq!(
            RootValidator::with_default_budget().validate(root, &memory),
            Err(RootValidationError::SharedTableFrame { level: 0 })
        );
    }

    #[test]
    fn writable_leaf_without_nx_is_rejected() {
        let mut memory = FakeMemory::new();
        let root = memory.install_chain(KERNEL_HEAP_BASE);
        let idx = indices(KERNEL_HEAP_BASE);
        let pt = {
            // Recover the PT frame installed by install_chain at idx[2].
            let pd = memory.table(memory.table(root).unwrap().get(idx[0]).unwrap().address())
                .unwrap()
                .get(idx[1])
                .unwrap()
                .address();
            memory.table(pd).unwrap().get(idx[2]).unwrap().address()
        };
        memory.set_entry(
            pt,
            idx[3],
            PageTableEntry::new(
                0x50_0000,
                PageFlags::PRESENT.union(PageFlags::WRITABLE),
            ),
        );
        assert_eq!(
            RootValidator::with_default_budget().validate(root, &memory),
            Err(RootValidationError::WritableExecutableLeaf {
                virtual_address: KERNEL_HEAP_BASE
            })
        );
    }

    #[test]
    fn writable_text_leaf_is_rejected() {
        let mut memory = FakeMemory::new();
        let root = memory.install_chain(KERNEL_TEXT_BASE);
        let idx = indices(KERNEL_TEXT_BASE);
        let pd = memory.table(root).unwrap().get(idx[0]).unwrap().address();
        let pt = memory
            .table(memory.table(pd).unwrap().get(idx[1]).unwrap().address())
            .unwrap()
            .get(idx[2])
            .unwrap()
            .address();
        memory.set_entry(
            pt,
            idx[3],
            PageTableEntry::new(
                0x40_0000,
                PageFlags::PRESENT.union(PageFlags::WRITABLE),
            ),
        );
        assert_eq!(
            RootValidator::with_default_budget().validate(root, &memory),
            Err(RootValidationError::WritableExecutableLeaf {
                virtual_address: KERNEL_TEXT_BASE
            })
        );
    }

    #[test]
    fn leaf_outside_planned_region_is_rejected() {
        let mut memory = FakeMemory::new();
        let gap = 0xffff_c110_0000_0000u64;
        let root = memory.install_chain(gap);
        let idx = indices(gap);
        let pd = memory.table(root).unwrap().get(idx[0]).unwrap().address();
        let pt = memory
            .table(memory.table(pd).unwrap().get(idx[1]).unwrap().address())
            .unwrap()
            .get(idx[2])
            .unwrap()
            .address();
        memory.set_entry(pt, idx[3], PageTableEntry::new(0x50_0000, RNX));
        assert_eq!(
            RootValidator::with_default_budget().validate(root, &memory),
            Err(RootValidationError::LeafOutsidePlannedRegion { virtual_address: gap })
        );
    }

    #[test]
    fn table_budget_is_enforced() {
        let mut memory = FakeMemory::new();
        let root = memory.alloc_table().expect("root frame");
        let shared_flags = PageFlags::PRESENT.union(PageFlags::WRITABLE);
        for index in 256..259usize {
            let child = memory.alloc_table().expect("child");
            memory.set_entry(root, index, PageTableEntry::new(child, shared_flags));
        }
        let validator = RootValidator::new(2).expect("budget valid");
        assert_eq!(
            validator.validate(root, &memory),
            Err(RootValidationError::TableBudgetExceeded { visited: 2 })
        );
        assert_eq!(
            RootValidator::new(0),
            Err(RootValidationError::TableBudgetExceeded { visited: 0 })
        );
        assert_eq!(
            RootValidator::new(MAX_ROOT_VALIDATION_TABLES + 1),
            Err(RootValidationError::TableBudgetExceeded { visited: 0 })
        );
    }

    #[test]
    fn unreadable_backend_fails_closed() {
        let mut memory = FakeMemory::new();
        let root = memory.alloc_table().expect("root frame");
        memory.set_entry(
            root,
            256,
            PageTableEntry::new(0x90_0000, PageFlags::PRESENT.union(PageFlags::WRITABLE)),
        );
        // 0x90_0000 was never inserted into the fake backend: unreachable.
        assert_eq!(
            RootValidator::with_default_budget().validate(root, &memory),
            Err(RootValidationError::InvalidChildFrame { level: 0 })
        );
    }

    #[test]
    fn root_frame_is_counted_in_dedupe_set() {
        let mut memory = FakeMemory::new();
        let root = memory.alloc_table().expect("root frame");
        // A cycle back to the PML4 root is caught as a shared frame.
        memory.set_entry(
            root,
            256,
            PageTableEntry::new(root, PageFlags::PRESENT.union(PageFlags::WRITABLE)),
        );
        assert_eq!(
            RootValidator::with_default_budget().validate(root, &memory),
            Err(RootValidationError::SharedTableFrame { level: 0 })
        );
    }
}
