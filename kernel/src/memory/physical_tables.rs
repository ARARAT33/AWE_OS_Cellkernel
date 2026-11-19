#![no_std]

//! Kernel-side `TableFramePool` over a real physical frame allocator.
//!
//! This module closes the gap left open by `table_frames`: a pool whose
//! frames are real physical frames that the kernel can actually dereference
//! through an explicitly supplied, validated physical-to-virtual window.
//!
//! Components:
//! - `DirectMapWindow`: a bounded physical window with a fixed virtual
//!   offset. Every translation is checked (alignment, containment, overflow)
//!   and every construction-time invariant is validated, so a misconfigured
//!   window is rejected instead of dereferenced.
//! - `FramePoolSource`: allocation and release over a frame allocator that
//!   tracks ownership (for example `BitmapFrameAllocator`).
//! - `PhysicalTablePool`: implements `TableFramePool` by allocating from the
//!   source, refusing (fail-closed) any frame the window cannot translate,
//!   zeroing the frame through the mapped virtual address, and exposing
//!   `PageTable` access through the same window.
//!
//! Safety contract: the caller guarantees that `virt_offset` really maps
//! `[phys_base, phys_base + length)` writable into the current address
//! space. The pool cannot verify this and dereferences through the window
//! as instructed. A window that is merely well-formed arithmetically but
//! not actually mapped is a caller error, not something this module fakes.
//!
//! Still explicit non-goals: CR3 activation, TLB invalidation and boot-time
//! wiring of the live allocator. `build_address_space` may only be called
//! once the boot path has installed a real window over live frames.

use super::frame::{BitmapFrameAllocator, BitmapFrameError, Frame, PAGE_SIZE};
use super::paging::PageTable;
use super::table_frames::{FrameReleaseError, TableFramePool};

/// Errors returned when constructing a `DirectMapWindow`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DirectMapError {
    /// The physical base, length or virtual offset is not page aligned.
    Misaligned,
    /// The window covers no bytes.
    EmptyWindow,
    /// The window or its virtual translation would overflow `u64`/`usize`.
    Overflow,
}

/// A validated physical window with a fixed virtual offset.
///
/// Translates physical addresses inside `[phys_base, phys_base + length)`
/// to `virt_offset + (physical - phys_base)`. All arithmetic is checked.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DirectMapWindow {
    phys_base: u64,
    length: u64,
    virt_offset: u64,
}

impl DirectMapWindow {
    /// Create a window. All three values must be page aligned, the length
    /// must be nonzero, and the physical end and the highest virtual
    /// translation must not overflow.
    pub fn new(phys_base: u64, length: u64, virt_offset: u64) -> Result<Self, DirectMapError> {
        if phys_base & (PAGE_SIZE - 1) != 0
            || length & (PAGE_SIZE - 1) != 0
            || virt_offset & (PAGE_SIZE - 1) != 0
        {
            return Err(DirectMapError::Misaligned);
        }
        if length == 0 {
            return Err(DirectMapError::EmptyWindow);
        }
        phys_base
            .checked_add(length)
            .ok_or(DirectMapError::Overflow)?;
        // Highest translated address must stay representable in usize.
        let highest = virt_offset
            .checked_add(length - PAGE_SIZE)
            .ok_or(DirectMapError::Overflow)?;
        usize::try_from(highest).map_err(|_| DirectMapError::Overflow)?;
        Ok(Self {
            phys_base,
            length,
            virt_offset,
        })
    }

    /// The physical base of the window.
    pub const fn phys_base(&self) -> u64 {
        self.phys_base
    }

    /// The window length in bytes.
    pub const fn length(&self) -> u64 {
        self.length
    }

    /// The virtual offset added to every physical address.
    pub const fn virt_offset(&self) -> u64 {
        self.virt_offset
    }

    /// Translate a page-aligned physical address inside the window to its
    /// virtual address. Returns `None` for unaligned or out-of-window
    /// addresses instead of guessing.
    pub fn translate(&self, physical: u64) -> Option<usize> {
        if physical & (PAGE_SIZE - 1) != 0 || physical < self.phys_base {
            return None;
        }
        let offset = physical.checked_sub(self.phys_base)?;
        if offset >= self.length {
            return None;
        }
        let virt = self.virt_offset.checked_add(offset)?;
        usize::try_from(virt).ok()
    }
}

/// A frame allocator that both allocates and releases frames it tracks.
///
/// The pool delegates ownership accounting to the source, so a frame the
/// source never allocated is reported as unknown rather than silently freed.
pub trait FramePoolSource {
    /// Allocate one frame.
    fn alloc_frame(&mut self) -> Option<Frame>;
    /// Release a previously allocated frame.
    fn free_frame(&mut self, frame: Frame) -> Result<(), FrameReleaseError>;
}

impl<const WORDS: usize> FramePoolSource for BitmapFrameAllocator<WORDS> {
    fn alloc_frame(&mut self) -> Option<Frame> {
        if !self.is_valid() {
            return None;
        }
        self.allocate_frame()
    }

    fn free_frame(&mut self, frame: Frame) -> Result<(), FrameReleaseError> {
        match self.try_free_frame(frame) {
            Ok(()) => Ok(()),
            Err(BitmapFrameError::DoubleFree) => Err(FrameReleaseError::DoubleRelease),
            Err(_) => Err(FrameReleaseError::UnknownFrame),
        }
    }
}

/// Maximum number of distinct physical frame addresses tracked by one pool.
///
/// Released addresses remain recorded so a stale second release can be
/// distinguished from a frame that was never owned by this pool. A released
/// address may be reused by the source allocator without consuming another
/// ledger entry. Once the ledger has seen this many distinct addresses, new
/// addresses are rejected fail-closed; choose a larger const capacity if the
/// pool's lifetime requires it.
pub const DEFAULT_PHYSICAL_TABLE_POOL_CAPACITY: usize = 256;

#[derive(Clone, Copy, PartialEq, Eq)]
enum FrameState {
    Empty,
    Live,
    Released,
}

#[derive(Clone, Copy)]
struct FrameRecord {
    start: u64,
    state: FrameState,
}

impl FrameRecord {
    const EMPTY: Self = Self {
        start: 0,
        state: FrameState::Empty,
    };
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TrackAllocationError {
    AlreadyLive,
    CapacityExhausted,
}

/// A `TableFramePool` over real physical frames accessed through a
/// `DirectMapWindow`.
///
/// Every allocation is recorded in a bounded ownership ledger. Table access
/// is allowed only while the frame is live in this pool; frames allocated
/// directly from the underlying source and frames already released are
/// rejected. Released addresses remain as tombstones for double-release
/// detection and can be marked live again if the source allocator reuses the
/// same address.
///
/// The default ledger tracks 256 distinct frame addresses over the pool's
/// lifetime. If it is full, allocation fails closed and returns a newly
/// allocated frame to the source where possible.
pub struct PhysicalTablePool<
    's,
    S: FramePoolSource,
    const CAP: usize = DEFAULT_PHYSICAL_TABLE_POOL_CAPACITY,
> {
    window: DirectMapWindow,
    source: &'s mut S,
    records: [FrameRecord; CAP],
}

impl<'s, S: FramePoolSource, const CAP: usize> PhysicalTablePool<'s, S, CAP> {
    /// Wrap a frame source with a translated window.
    ///
    /// # Safety
    /// The caller must guarantee that `window.virt_offset` actually maps the
    /// whole physical window writable into the current address space. The
    /// pool dereferences through the window on every table access.
    pub unsafe fn new(window: DirectMapWindow, source: &'s mut S) -> Self {
        Self {
            window,
            source,
            records: [FrameRecord::EMPTY; CAP],
        }
    }

    /// The validated window this pool translates through.
    pub const fn window(&self) -> &DirectMapWindow {
        &self.window
    }

    fn record_index(&self, frame: Frame) -> Option<usize> {
        self.records.iter().position(|record| {
            record.state != FrameState::Empty && record.start == frame.start
        })
    }

    fn is_live(&self, frame: Frame) -> bool {
        match self.record_index(frame) {
            Some(index) => self.records[index].state == FrameState::Live,
            None => false,
        }
    }

    fn track_allocation(&mut self, frame: Frame) -> Result<(), TrackAllocationError> {
        if let Some(index) = self.record_index(frame) {
            if self.records[index].state == FrameState::Released {
                self.records[index].state = FrameState::Live;
                return Ok(());
            }
            return Err(TrackAllocationError::AlreadyLive);
        }

        let Some(index) = self
            .records
            .iter()
            .position(|record| record.state == FrameState::Empty)
        else {
            return Err(TrackAllocationError::CapacityExhausted);
        };
        self.records[index] = FrameRecord {
            start: frame.start,
            state: FrameState::Live,
        };
        Ok(())
    }

    fn zero_frame(&self, virtual_address: usize) {
        // SAFETY: the caller of `new` guarantees the window maps this
        // allocated frame writable, and `translate` checked containment and
        // overflow. A page table is 512 u64 entries.
        let ptr = virtual_address as *mut u64;
        for index in 0..512usize {
            unsafe {
                ptr.add(index).write_volatile(0);
            }
        }
    }
}

impl<S: FramePoolSource, const CAP: usize> TableFramePool for PhysicalTablePool<'_, S, CAP> {
    fn allocate(&mut self) -> Option<Frame> {
        let frame = self.source.alloc_frame()?;
        let Some(virtual_address) = self.window.translate(frame.start) else {
            // Fail closed: never hand out a frame the window cannot access.
            let _ = self.source.free_frame(frame);
            return None;
        };

        match self.track_allocation(frame) {
            Ok(()) => {
                self.zero_frame(virtual_address);
                Some(frame)
            }
            Err(TrackAllocationError::AlreadyLive) => {
                // The source violated its unique-allocation contract. Do not
                // free the address: it may still back a live table.
                None
            }
            Err(TrackAllocationError::CapacityExhausted) => {
                // Preserve source accounting when the bounded ledger is full.
                let _ = self.source.free_frame(frame);
                None
            }
        }
    }

    fn release(&mut self, frame: Frame) -> Result<(), FrameReleaseError> {
        let Some(index) = self.record_index(frame) else {
            return Err(FrameReleaseError::UnknownFrame);
        };
        match self.records[index].state {
            FrameState::Live => {
                self.source.free_frame(frame)?;
                self.records[index].state = FrameState::Released;
                Ok(())
            }
            FrameState::Released => Err(FrameReleaseError::DoubleRelease),
            FrameState::Empty => Err(FrameReleaseError::UnknownFrame),
        }
    }

    fn table(&self, frame: Frame) -> Option<&PageTable> {
        if !self.is_live(frame) {
            return None;
        }
        let virtual_address = self.window.translate(frame.start)?;
        // SAFETY: see `new`; the frame is live, page aligned and a whole
        // `PageTable` (4096 bytes) fits in the mapped page.
        Some(unsafe { &*(virtual_address as *const PageTable) })
    }

    fn table_mut(&mut self, frame: Frame) -> Option<&mut PageTable> {
        if !self.is_live(frame) {
            return None;
        }
        let virtual_address = self.window.translate(frame.start)?;
        // SAFETY: see `new`; the frame is live, page aligned and the
        // exclusive borrow prevents another mutable access through this pool.
        Some(unsafe { &mut *(virtual_address as *mut PageTable) })
    }
}

#[cfg(test)]
mod tests {
    use super::super::address_space::AddressSpace;
    use super::super::page_table_build::UserPageTableBuilder;
    use super::super::paging::{PageFlags, PageTableEntry};
    use super::super::table_frames::{OwnedTableFrames, build_address_space};
    use super::*;

    const RW: PageFlags = PageFlags::PRESENT
        .union(PageFlags::USER)
        .union(PageFlags::WRITABLE)
        .union(PageFlags::NO_EXECUTE);

    /// Real, page-aligned memory standing in for physical frames so the
    /// tests can dereference through an identity window on the host.
    #[repr(C, align(4096))]
    struct PhysicalMemory {
        pages: [[u64; 512]; 8],
    }

    impl PhysicalMemory {
        fn new() -> Self {
            Self {
                pages: [[0u64; 512]; 8],
            }
        }
        fn as_u64(&self) -> u64 {
            self.pages.as_ptr() as u64
        }
    }

    fn identity_window(base: u64) -> DirectMapWindow {
        DirectMapWindow::new(base, 8 * PAGE_SIZE, base).expect("identity window")
    }

    #[test]
    fn window_rejects_misaligned_empty_and_overflowing_configuration() {
        assert_eq!(
            DirectMapWindow::new(0x1001, PAGE_SIZE, 0).err(),
            Some(DirectMapError::Misaligned)
        );
        assert_eq!(
            DirectMapWindow::new(0x1000, 0xfff, 0).err(),
            Some(DirectMapError::Misaligned)
        );
        assert_eq!(
            DirectMapWindow::new(0x1000, 0, 0).err(),
            Some(DirectMapError::EmptyWindow)
        );
        assert_eq!(
            DirectMapWindow::new(u64::MAX - 0xfff, 0x2000, 0).err(),
            Some(DirectMapError::Overflow)
        );
        assert_eq!(
            DirectMapWindow::new(0x1000, PAGE_SIZE, u64::MAX - 0xfff).err(),
            Some(DirectMapError::Overflow)
        );
    }

    #[test]
    fn window_translate_is_checked_and_exact() {
        let window = DirectMapWindow::new(0x200_0000, 4 * PAGE_SIZE, 0xffff_8000_0200_0000)
            .expect("valid window");
        assert_eq!(
            window.translate(0x200_0000),
            usize::try_from(0xffff_8000_0200_0000).ok()
        );
        assert_eq!(
            window.translate(0x200_3000),
            usize::try_from(0xffff_8000_0200_3000).ok()
        );
        // Before, after, unaligned.
        assert_eq!(window.translate(0x1ff_f000), None);
        assert_eq!(window.translate(0x204_0000), None);
        assert_eq!(window.translate(0x200_0800), None);
    }

    #[test]
    fn allocated_frames_are_zeroed_and_accessible() {
        let memory = PhysicalMemory::new();
        let base = memory.as_u64();
        let mut source = BitmapFrameAllocator::<1>::new(base, 8);
        // SAFETY: the identity window maps the buffer itself.
        let mut pool = unsafe { PhysicalTablePool::new(identity_window(base), &mut source) };

        let frame = pool.allocate().expect("frame");
        assert_eq!(frame.start, base);
        // The handed-out frame must be fully zeroed.
        for index in 0..512usize {
            assert_eq!(memory.pages[0][index], 0);
        }
        // Table access resolves through the window and is writable.
        pool.table_mut(frame).expect("table").set(
            3,
            PageTableEntry::new(0x9000, PageFlags::PRESENT.union(PageFlags::USER)),
        );
        assert_eq!(memory.pages[0][3], 0x9005); // 0x9000 | PRESENT | USER
        let table = pool.table(frame).expect("table");
        assert!(table.get(3).expect("entry").is_present());

        pool.release(frame).expect("release");
        assert!(pool.table(frame).is_none());
        assert!(pool.table_mut(frame).is_none());
        assert_eq!(pool.release(frame), Err(FrameReleaseError::DoubleRelease));

        // The source may reuse a released physical address. The ledger must
        // reactivate that exact record and zero the page before reuse.
        let reused = pool.allocate().expect("reused frame");
        assert_eq!(reused, frame);
        assert!(pool.table(reused).is_some());
        assert!(!pool
            .table(reused)
            .expect("reused table")
            .get(3)
            .expect("cleared entry")
            .is_present());
    }

    #[test]
    fn foreign_in_window_frame_is_not_accessible_or_releasable() {
        let mut memory = PhysicalMemory::new();
        memory.pages[0][0] = 0xfeed_cafe_dead_beef;
        let base = memory.as_u64();
        let mut source = BitmapFrameAllocator::<1>::new(base, 8);
        // Allocate directly from the source before the pool exists.
        let foreign = source.allocate_frame().expect("foreign frame");
        // SAFETY: the identity window maps the backing buffer itself.
        let mut pool = unsafe { PhysicalTablePool::new(identity_window(base), &mut source) };

        assert!(pool.table(foreign).is_none());
        assert!(pool.table_mut(foreign).is_none());
        assert_eq!(
            pool.release(foreign),
            Err(FrameReleaseError::UnknownFrame)
        );

        // The pool must not free or zero the source owner's frame.
        let owned = pool.allocate().expect("pool-owned frame");
        assert_eq!(owned.start, base + PAGE_SIZE);
        assert_eq!(memory.pages[0][0], 0xfeed_cafe_dead_beef);

        drop(pool);
        source.try_free_frame(foreign).expect("foreign owner still owns frame");
    }

    #[test]
    fn out_of_window_allocation_fails_closed_and_returns_the_frame() {
        let memory = PhysicalMemory::new();
        let base = memory.as_u64();
        let mut source = BitmapFrameAllocator::<1>::new(base, 8);
        // The window covers only the first four frames of the allocator.
        let window = DirectMapWindow::new(base, 4 * PAGE_SIZE, base).expect("window");
        // SAFETY: the identity window maps the buffer itself.
        let mut pool = unsafe { PhysicalTablePool::new(window, &mut source) };

        for _ in 0..4 {
            assert!(pool.allocate().is_some());
        }
        // The fifth frame is outside the window: fail closed, frame returned.
        assert!(pool.allocate().is_none());
        // The rejected frame is returned to the source. End the pool's
        // exclusive borrow before checking the source allocator directly.
        drop(pool);
        let returned = source.allocate_frame().expect("returned out-of-window frame");
        assert_eq!(returned.start, base + 4 * PAGE_SIZE);
        source.try_free_frame(returned).expect("return to free list");
    }

    #[test]
    fn build_address_space_runs_over_real_frames_and_translates() {
        let memory = PhysicalMemory::new();
        let base = memory.as_u64();
        let mut source = BitmapFrameAllocator::<1>::new(base, 8);
        // SAFETY: the identity window maps the buffer itself.
        let mut pool = unsafe { PhysicalTablePool::new(identity_window(base), &mut source) };

        // Probe frame for the contract object, then released back.
        let probe = pool.allocate().expect("probe");
        let mut space = AddressSpace::<4>::new(probe.start).expect("contract root");
        space
            .map_user(0x0040_0000, 0x0090_0000, 2, RW)
            .expect("valid mapping");
        pool.release(probe).expect("probe release");

        let built = build_address_space::<PhysicalTablePool<_>, 64, 4>(&mut pool, &space)
            .expect("build over real frames");
        let root = built.root().expect("root allocated by the wrapper");
        assert_eq!(built.frame_count(), 4); // PML4 + PDPT + PD + PT

        // Hardware walk through the real tables matches the contract.
        let mut access = OwnedTableFrames::<PhysicalTablePool<_>, 64>::new(&mut pool);
        let translate = {
            let builder = UserPageTableBuilder::new(root, &mut access).expect("valid root");
            (
                builder.translate(0x0040_0000),
                builder.translate(0x0040_1000),
                builder.translate(0x0040_2000),
            )
        };
        assert_eq!(translate.0, Some(0x0090_0000));
        assert_eq!(translate.1, Some(0x0090_1000));
        assert_eq!(translate.2, None);

        // End the temporary inspection wrapper's exclusive borrow before
        // accessing the pool directly. The returned BuiltTables remains the
        // owner of these live frames until we explicitly release them.
        drop(access);

        // The PML4 root lives in real memory and its first entry points at
        // one of the built frames.
        let pml4 = pool.table(Frame { start: root }).expect("pml4");
        let entry = pml4.get(0).expect("entry 0");
        assert!(entry.is_present());
        assert!(built.frames().contains(&entry.address()));

        for &physical in built.frames().iter().rev() {
            pool.release(Frame { start: physical })
                .expect("release built frame");
        }
    }
}
