#![no_std]

//! Frame ownership, rollback and teardown for page-table construction.
//!
//! The `UserPageTableBuilder` allocates intermediate table frames through a
//! `TableMemory` backend but never frees them: teardown was explicitly left to
//! the owner. This module supplies that owner. `OwnedTableFrames` wraps any
//! `TableFramePool`, records every frame it hands to a builder (including the
//! PML4 root when allocated here), and can release them all: on success via
//! the `take_ownership` transfer, on failure via `rollback` or `teardown`,
//! and on drop as a best-effort safety net.
//!
//! Explicit non-goal: nothing in this module yet dereferences real physical
//! frames. A kernel-side `TableFramePool` over the live frame allocator plus
//! a verified physical-to-virtual mapping is still an open milestone; until
//! it exists, `build_address_space` must not be called from boot code.
//! Nothing here fakes that mapping.

use super::address_space::AddressSpace;
use super::frame::Frame;
use super::page_table_build::{PageBuildError, TableMemory, UserPageTableBuilder};
use super::paging::PageTable;

/// Error returned when a frame cannot be returned to its pool.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FrameReleaseError {
    /// The pool does not know this frame.
    UnknownFrame,
    /// The frame was already released; ownership was violated.
    DoubleRelease,
}

/// A pool of page-table frames with allocation, release and access.
///
/// The kernel will implement this over the live physical frame allocator once
/// a verified physical-to-virtual mapping exists. Tests implement it over an
/// in-memory mirror. Implementations must hand out zeroed, page-aligned
/// frames and must be able to access every frame they handed out.
pub trait TableFramePool {
    /// Allocate one zeroed page-table frame.
    fn allocate(&mut self) -> Option<Frame>;
    /// Return a previously allocated frame to the pool.
    ///
    /// # Errors
    /// Returns `FrameReleaseError::UnknownFrame` for a frame the pool did not
    /// allocate and `FrameReleaseError::DoubleRelease` when the frame was
    /// already free.
    fn release(&mut self, frame: Frame) -> Result<(), FrameReleaseError>;
    /// Read-only access to the table stored at a frame of this pool.
    fn table(&self, frame: Frame) -> Option<&PageTable>;
    /// Mutable access to the table stored at a frame of this pool.
    fn table_mut(&mut self, frame: Frame) -> Option<&mut PageTable>;
}

/// Upper bound on frames owned by one `OwnedTableFrames`.
pub const DEFAULT_OWNED_CAPACITY: usize = 256;

/// Owns every page-table frame allocated through it, with rollback.
///
/// Implements `TableMemory`, so it can be handed directly to a
/// `UserPageTableBuilder`. Every frame returned by `alloc_table` is recorded
/// as owned; the root allocated through `alloc_root` is owned too. Ownership
/// ends only through `take_ownership`, `rollback`, `teardown` or drop.
pub struct OwnedTableFrames<'p, P: TableFramePool, const CAP: usize = DEFAULT_OWNED_CAPACITY> {
    pool: &'p mut P,
    root: Option<u64>,
    owned: [u64; CAP],
    owned_count: usize,
    batch_start: usize,
    transferred: bool,
}

impl<'p, P: TableFramePool, const CAP: usize> OwnedTableFrames<'p, P, CAP> {
    /// Wrap a pool. No frame is owned until one is allocated.
    pub fn new(pool: &'p mut P) -> Self {
        Self {
            pool,
            root: None,
            owned: [0; CAP],
            owned_count: 0,
            batch_start: 0,
            transferred: false,
        }
    }

    /// Allocate the PML4 root frame and record it as owned.
    ///
    /// # Errors
    /// Returns `PageBuildError::DuplicateTableFrame` when a root was already
    /// allocated here and `PageBuildError::TableLimit` or
    /// `PageBuildError::OutOfFrames` when capacity or the pool is exhausted.
    /// Nothing is leaked: the caller can roll back.
    pub fn alloc_root(&mut self) -> Result<u64, PageBuildError> {
        if self.root.is_some() {
            return Err(PageBuildError::DuplicateTableFrame);
        }
        if self.owned_count >= CAP {
            return Err(PageBuildError::TableLimit);
        }
        let frame = self.pool.allocate().ok_or(PageBuildError::OutOfFrames)?;
        self.owned[self.owned_count] = frame.start;
        self.owned_count += 1;
        self.root = Some(frame.start);
        Ok(frame.start)
    }

    /// The owned root frame, if one was allocated here.
    pub const fn root(&self) -> Option<u64> {
        self.root
    }

    /// Number of frames currently owned (root included when allocated here).
    pub const fn owned_count(&self) -> usize {
        self.owned_count
    }

    /// The owned frames, in allocation order. The root is included when it
    /// was allocated here.
    pub fn owned_frames(&self) -> &[u64] {
        &self.owned[..self.owned_count]
    }

    /// Mark the start of a rollback batch. Only frames allocated after this
    /// point are released by `rollback`.
    pub fn begin_batch(&mut self) {
        self.batch_start = self.owned_count;
    }

    /// Release every frame allocated since the last `begin_batch`, in reverse
    /// allocation order. Returns the number of frames released.
    ///
    /// A frame that fails to release is still removed from the owned list, so
    /// ownership cannot be silently duplicated; the first error is returned
    /// after every remaining frame has been processed.
    pub fn rollback(&mut self) -> Result<usize, FrameReleaseError> {
        let mut released = 0usize;
        let mut first_error: Option<FrameReleaseError> = None;
        while self.owned_count > self.batch_start {
            self.owned_count -= 1;
            let physical = self.owned[self.owned_count];
            self.owned[self.owned_count] = 0;
            match self.pool.release(Frame { start: physical }) {
                Ok(()) => released += 1,
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(released),
        }
    }

    /// Release every owned frame, including the root, in reverse allocation
    /// order. Returns the number of frames released.
    pub fn teardown(&mut self) -> Result<usize, FrameReleaseError> {
        self.batch_start = 0;
        self.root = None;
        self.rollback()
    }

    /// Transfer ownership of all frames to the caller and disable the
    /// drop-time release. The returned frames are live page tables; the
    /// caller is now responsible for their lifetime.
    #[must_use]
    pub fn take_ownership(mut self) -> BuiltTables<CAP> {
        self.transferred = true;
        BuiltTables {
            root: self.root,
            frames: self.owned,
            frame_count: self.owned_count,
        }
    }
}

impl<P: TableFramePool, const CAP: usize> Drop for OwnedTableFrames<'_, P, CAP> {
    fn drop(&mut self) {
        if !self.transferred {
            // Best-effort safety net: drop cannot report errors, so callers
            // that must observe release failures use teardown explicitly.
            let _ = self.teardown();
        }
    }
}

impl<P: TableFramePool, const CAP: usize> TableMemory for OwnedTableFrames<'_, P, CAP> {
    fn alloc_table(&mut self) -> Option<u64> {
        if self.owned_count >= CAP {
            return None;
        }
        let frame = self.pool.allocate()?;
        self.owned[self.owned_count] = frame.start;
        self.owned_count += 1;
        Some(frame.start)
    }

    fn table_mut(&mut self, physical: u64) -> Option<&mut PageTable> {
        self.pool.table_mut(Frame { start: physical })
    }

    fn table(&self, physical: u64) -> Option<&PageTable> {
        self.pool.table(Frame { start: physical })
    }
}

/// Frames transferred out of an `OwnedTableFrames` by `take_ownership`.
pub struct BuiltTables<const CAP: usize = DEFAULT_OWNED_CAPACITY> {
    root: Option<u64>,
    frames: [u64; CAP],
    frame_count: usize,
}

impl<const CAP: usize> BuiltTables<CAP> {
    /// The PML4 root frame, when it was allocated by the builder wrapper.
    pub const fn root(&self) -> Option<u64> {
        self.root
    }

    /// All owned frames (root included when allocated here), in allocation
    /// order.
    pub fn frames(&self) -> &[u64] {
        &self.frames[..self.frame_count]
    }

    /// Number of owned frames.
    pub const fn frame_count(&self) -> usize {
        self.frame_count
    }
}

/// Build one address space with full rollback on failure.
///
/// Allocates the PML4 root from `pool`, installs every mapping of `space`
/// through a `UserPageTableBuilder`, and on any error releases every frame
/// allocated during the build, including the root, before returning the
/// builder error. On success the frames are transferred to the returned
/// `BuiltTables`; the pool must outlive their use.
///
/// This function must not be called from boot code until a kernel-side
/// `TableFramePool` over real physical frames and a verified
/// physical-to-virtual mapping exists.
pub fn build_address_space<P: TableFramePool, const CAP: usize, const NM: usize>(
    pool: &mut P,
    space: &AddressSpace<NM>,
) -> Result<BuiltTables<CAP>, PageBuildError> {
    let mut memory = OwnedTableFrames::<P, CAP>::new(pool);
    let root = memory.alloc_root()?;
    {
        let mut builder = UserPageTableBuilder::new(root, &mut memory)?;
        builder.install_address_space(space)?;
    }
    Ok(memory.take_ownership())
}

#[cfg(test)]
mod tests {
    use super::super::paging::{PageFlags, PageTableEntry, indices as page_indices};
    use super::*;

    const RW: PageFlags = PageFlags::PRESENT
        .union(PageFlags::USER)
        .union(PageFlags::WRITABLE)
        .union(PageFlags::NO_EXECUTE);

    /// In-memory mirror pool: real allocation, release and ownership
    /// accounting, with table contents stored in kernel-side memory instead
    /// of physical frames (the physical mapping is an explicit non-goal).
    struct MirrorPool {
        addrs: [u64; 16],
        used: [bool; 16],
        tables: [PageTable; 16],
        next: usize,
    }

    impl MirrorPool {
        fn new() -> Self {
            Self {
                addrs: [0; 16],
                used: [false; 16],
                tables: core::array::from_fn(|_| PageTable::new()),
                next: 0,
            }
        }

        fn free_count(&self) -> usize {
            self.used.iter().filter(|used| !**used).count()
        }

        fn slot_of(&self, frame: Frame) -> Option<usize> {
            self.addrs
                .iter()
                .position(|&addr| addr != 0 && addr == frame.start)
        }
    }

    impl TableFramePool for MirrorPool {
        fn allocate(&mut self) -> Option<Frame> {
            let slot = self.used.iter().position(|used| !used)?;
            let physical = 0x0010_0000 + (self.next as u64) * 0x1000;
            self.next += 1;
            self.addrs[slot] = physical;
            self.used[slot] = true;
            self.tables[slot] = PageTable::new();
            Some(Frame { start: physical })
        }

        fn release(&mut self, frame: Frame) -> Result<(), FrameReleaseError> {
            let slot = self.slot_of(frame).ok_or(FrameReleaseError::UnknownFrame)?;
            if !self.used[slot] {
                return Err(FrameReleaseError::DoubleRelease);
            }
            self.used[slot] = false;
            Ok(())
        }

        fn table(&self, frame: Frame) -> Option<&PageTable> {
            let slot = self.slot_of(frame)?;
            if !self.used[slot] {
                return None;
            }
            self.tables.get(slot)
        }

        fn table_mut(&mut self, frame: Frame) -> Option<&mut PageTable> {
            let slot = self.slot_of(frame)?;
            if !self.used[slot] {
                return None;
            }
            self.tables.get_mut(slot)
        }
    }

    /// Register one two-page user mapping against a probe root, then return
    /// the probe frame so the pool stays fully free for the build itself.
    fn two_page_space<P: TableFramePool>(pool: &mut P) -> (AddressSpace<4>, Frame) {
        let probe = pool.allocate().expect("probe frame");
        let mut space = AddressSpace::<4>::new(probe.start).expect("contract root");
        space
            .map_user(0x0040_0000, 0x0090_0000, 2, RW)
            .expect("valid mapping");
        (space, probe)
    }

    #[test]
    fn successful_build_transfers_root_and_tables() {
        let mut pool = MirrorPool::new();
        let (space, probe) = two_page_space(&mut pool);
        pool.release(probe).expect("probe release");

        let built = build_address_space::<MirrorPool, 64, 4>(&mut pool, &space)
            .expect("build must succeed");
        let root = built.root().expect("root allocated by the builder wrapper");
        assert_eq!(built.frame_count(), 5); // root + PML4E + PDPT + PD + PT
        assert!(built.frames().contains(&root));

        // Translations resolve through the pool after ownership transfer.
        let mut access = OwnedTableFrames::<MirrorPool, 64>::new(&mut pool);
        let builder =
            UserPageTableBuilder::new(root, &mut access).expect("valid transferred root");
        assert_eq!(builder.translate(0x0040_0000), Some(0x0090_0000));
        assert_eq!(builder.translate(0x0040_1000), Some(0x0090_1000));
        assert_eq!(builder.translate(0x0040_2000), None);
    }

    #[test]
    fn failed_build_rolls_back_every_frame_including_root() {
        struct ExhaustingPool {
            inner: MirrorPool,
            remaining: usize,
        }
        impl TableFramePool for ExhaustingPool {
            fn allocate(&mut self) -> Option<Frame> {
                if self.remaining == 0 {
                    return None;
                }
                self.remaining -= 1;
                self.inner.allocate()
            }
            fn release(&mut self, frame: Frame) -> Result<(), FrameReleaseError> {
                self.inner.release(frame)
            }
            fn table(&self, frame: Frame) -> Option<&PageTable> {
                self.inner.table(frame)
            }
            fn table_mut(&mut self, frame: Frame) -> Option<&mut PageTable> {
                self.inner.table_mut(frame)
            }
        }

        let mut pool = ExhaustingPool {
            inner: MirrorPool::new(),
            remaining: 3,
        };
        let (space, probe) = two_page_space(&mut pool);
        pool.release(probe).expect("probe release");

        // The build needs a root plus at least three intermediate tables, but
        // only three frames remain, so it must fail closed and release every
        // frame it did take, including the root.
        let result = build_address_space::<ExhaustingPool, 64, 4>(&mut pool, &space);
        assert_eq!(result.err(), Some(PageBuildError::OutOfFrames));
        assert_eq!(pool.inner.free_count(), 16);
    }

    #[test]
    fn invalid_mapping_fails_before_any_allocation_and_rolls_back() {
        let mut pool = MirrorPool::new();
        let mut memory = OwnedTableFrames::<MirrorPool, 64>::new(&mut pool);
        let root = memory.alloc_root().expect("root");
        let mut builder =
            UserPageTableBuilder::new(root, &mut memory).expect("valid root");
        let kernel_half = super::super::address_space::USER_HALF_END;
        let bad = super::super::address_space::UserMapping {
            virtual_start: kernel_half,
            physical_start: 0x0090_0000,
            page_count: 1,
            flags: RW,
        };
        assert_eq!(
            builder.install_mapping(&bad).err(),
            Some(PageBuildError::KernelHalfForbidden)
        );
        assert_eq!(memory.owned_count(), 1); // only the root
        memory.teardown().expect("teardown");
        assert_eq!(pool.free_count(), 16);
    }

    #[test]
    fn rollback_releases_only_batch_frames() {
        let mut pool = MirrorPool::new();
        let mut memory = OwnedTableFrames::<MirrorPool, 64>::new(&mut pool);
        let root = memory.alloc_root().expect("root");
        let before = memory.owned_count();
        memory.begin_batch();
        let first = memory.alloc_table().expect("first batch frame");
        let second = memory.alloc_table().expect("second batch frame");
        assert_eq!(memory.rollback(), Ok(2));
        assert_eq!(memory.owned_count(), before);
        // The root is still owned and accessible; batch frames are gone.
        assert!(memory.table(root).is_some());
        assert!(memory.table(first).is_none());
        assert!(memory.table(second).is_none());
        memory.teardown().expect("teardown");
        assert_eq!(pool.free_count(), 16);
    }

    #[test]
    fn teardown_surfaces_double_release_but_keeps_going() {
        let mut pool = MirrorPool::new();
        let mut memory = OwnedTableFrames::<MirrorPool, 64>::new(&mut pool);
        let root = memory.alloc_root().expect("root");
        let first = memory.alloc_table().expect("table frame");
        // Simulate a foreign release of one owned frame.
        pool.release(Frame { start: first }).expect("foreign release");
        assert_eq!(memory.teardown(), Err(FrameReleaseError::DoubleRelease));
        // The remaining frames, including the root, were still released.
        assert_eq!(memory.owned_count(), 0);
        assert!(pool.table(Frame { start: root }).is_none());
    }

    #[test]
    fn drop_releases_unowned_frames_best_effort() {
        let mut pool = MirrorPool::new();
        {
            let mut memory = OwnedTableFrames::<MirrorPool, 64>::new(&mut pool);
            let _root = memory.alloc_root().expect("root");
            let _table = memory.alloc_table().expect("table frame");
            assert_eq!(memory.owned_count(), 2);
        }
        assert_eq!(pool.free_count(), 16);
    }

    #[test]
    fn take_ownership_disables_drop_release() {
        let mut pool = MirrorPool::new();
        let built = {
            let mut memory = OwnedTableFrames::<MirrorPool, 64>::new(&mut pool);
            let root = memory.alloc_root().expect("root");
            let _table = memory.alloc_table().expect("table frame");
            let built = memory.take_ownership();
            assert_eq!(built.root(), Some(root));
            assert_eq!(built.frame_count(), 2);
            built
        };
        // Frames remain held after drop; the caller releases them explicitly.
        assert_eq!(built.frames().len(), 2);
        for physical in built.frames() {
            pool.release(Frame { start: *physical })
                .expect("single release");
        }
        assert_eq!(pool.free_count(), 16);
    }

    #[test]
    fn alloc_root_twice_is_rejected_without_leak() {
        let mut pool = MirrorPool::new();
        let mut memory = OwnedTableFrames::<MirrorPool, 64>::new(&mut pool);
        assert!(memory.alloc_root().is_ok());
        assert_eq!(
            memory.alloc_root().err(),
            Some(PageBuildError::DuplicateTableFrame)
        );
        assert_eq!(memory.owned_count(), 1);
        memory.teardown().expect("teardown");
        assert_eq!(pool.free_count(), 16);
    }

    #[test]
    fn leaf_flags_survive_the_owned_backend() {
        let mut pool = MirrorPool::new();
        let (space, probe) = two_page_space(&mut pool);
        pool.release(probe).expect("probe release");

        let mut memory = OwnedTableFrames::<MirrorPool, 64>::new(&mut pool);
        let root = memory.alloc_root().expect("root");
        let mut builder =
            UserPageTableBuilder::new(root, &mut memory).expect("valid root");
        builder
            .install_address_space(&space)
            .expect("install must succeed");
        let leaf = builder.leaf_entry(0x0040_0000).expect("leaf present");
        assert_eq!(leaf.address(), 0x0090_0000);
        assert!(leaf.flags().contains(PageFlags::WRITABLE));
        assert!(leaf.flags().contains(PageFlags::NO_EXECUTE));
        assert!(leaf.flags().contains(PageFlags::USER));
        // Walk the hardware hierarchy manually through the owned backend.
        let idx = page_indices(0x0040_1000);
        let mut table_phys = root;
        for level in 0..3 {
            let entry = memory
                .table(table_phys)
                .expect("owned table")
                .get(idx[level])
                .expect("entry");
            assert!(entry.is_present());
            assert!(entry.flags().contains(PageFlags::USER));
            table_phys = entry.address();
        }
        let pte = memory
            .table(table_phys)
            .expect("owned leaf table")
            .get(idx[3])
            .expect("leaf entry");
        assert_eq!(pte.address(), 0x0090_1000);
        memory.teardown().expect("teardown");
        assert_eq!(pool.free_count(), 16);
    }

    #[test]
    fn leaf_pte_matches_contract_entry() {
        let mut pool = MirrorPool::new();
        let (space, probe) = two_page_space(&mut pool);
        pool.release(probe).expect("probe release");

        let mut memory = OwnedTableFrames::<MirrorPool, 64>::new(&mut pool);
        let root = memory.alloc_root().expect("root");
        let mut builder =
            UserPageTableBuilder::new(root, &mut memory).expect("valid root");
        builder
            .install_address_space(&space)
            .expect("install must succeed");
        let hardware = builder.leaf_entry(0x0040_0000).expect("hardware leaf");
        let contract = space.translate_page(0x0040_0000).expect("contract leaf");
        assert_eq!(hardware.address(), contract);
        memory.teardown().expect("teardown");
        assert_eq!(pool.free_count(), 16);
    }
}
