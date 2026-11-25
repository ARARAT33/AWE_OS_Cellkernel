#![no_std]

//! A reserved, contiguous table-frame arena for the live kernel path.
//!
//! PhysicalTablePool needs a FramePoolSource that can allocate AND release
//! frames, while the live boot PhysicalFrameAllocator is deliberately
//! monotonic: it never hands the same physical frame out twice and cannot
//! reclaim. This module supplies the missing source without weakening the
//! live allocator:
//!
//! - the boot path reserves a contiguous arena of physical frames through
//!   the reservation ledger (before the first frame allocation), so the
//!   live allocator permanently excludes the whole range;
//! - TableArena owns that range, tracks allocation through its own bounded
//!   bitmap and implements FramePoolSource with true release;
//! - PhysicalTablePool over the arena therefore allocates, zeroes, exposes
//!   and reclaims table frames over real physical pages, through a
//!   DirectMapWindow the caller proves for the chosen virtual mapping.
//!
//! Window proof rule: a window is only installed over a physical range
//! that the active bootstrap identity map provably maps (see memory::x86_64:
//! the first 1 GiB, supervisor-only). Outside that proven range the boot
//! probe skips instead of dereferencing an unverified mapping.

use super::boot_reserve::FrameReserver;
use super::frame::{BitmapFrameError, Frame, PAGE_SIZE};
use super::physical_tables::{DirectMapError, DirectMapWindow, FramePoolSource};
use super::table_frames::FrameReleaseError;

/// Table frames in the boot-time arena. Sixteen frames cover one root,
/// several intermediate tables and spare retries in the selftest probe
/// while keeping the reserved range at 64 KiB.
pub const BOOT_TABLE_ARENA_FRAMES: usize = 16;

/// The greatest number of frames a TableArena can track. The allocation
/// bitmap is a fixed-size array, so a generic arena cannot exceed this
/// bound; larger arenas must come as future work that buys real
/// allocation-free storage for the bitmap.
pub const TABLE_ARENA_MAX_FRAMES: usize = 128;

/// Fixed bitmap storage for one TableArena. Non-generic on purpose: a
/// stable-rust const generic may not be used inside an array-length
/// expression.
const TABLE_ARENA_BITMAP_WORDS: usize = TABLE_ARENA_MAX_FRAMES / 64;

/// Fail-closed errors for reserving a table arena before mutating an allocator.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TableArenaReserveError {
    /// An arena must contain at least one frame.
    ZeroFrames,
    /// The requested range cannot be tracked by the fixed TableArena bitmap.
    TooManyFrames,
    /// The physical base must be page aligned.
    MisalignedBase,
    /// The physical byte range overflows the address space.
    RangeOverflow,
    /// The underlying frame reserver rejected an otherwise valid range.
    Reservation(BitmapFrameError),
}

/// Fail-closed construction errors for a TableArena.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TableArenaError {
    /// A zero-page arena cannot hold any table.
    ZeroFrames,
    /// The requested arena exceeds TABLE_ARENA_MAX_FRAMES.
    TooManyFrames,
    /// The arena byte length overflows u64.
    LengthOverflow,
    /// The underlying direct-map window rejected the configuration.
    Window(DirectMapError),
}

/// A bounded arena of contiguous physical table frames with release.
///
/// The arena does not dereference anything itself: every translation stays
/// inside the validated DirectMapWindow, and PhysicalTablePool performs all
/// zeroing and table access through that window.
pub struct TableArena<const PAGES: usize> {
    phys_base: u64,
    window: DirectMapWindow,
    allocated: [u64; TABLE_ARENA_BITMAP_WORDS],
    allocated_count: usize,
}

impl<const PAGES: usize> TableArena<PAGES> {
    /// Open an arena at phys_base. The virtual mapping of the returned
    /// window is the caller-provided safety contract; construction only
    /// validates shape (alignment, non-zero length, overflow) and the
    /// fixed bitmap bound.
    pub fn open(phys_base: u64, virt_offset: u64) -> Result<Self, TableArenaError> {
        if PAGES == 0 {
            return Err(TableArenaError::ZeroFrames);
        }
        if PAGES > TABLE_ARENA_MAX_FRAMES {
            return Err(TableArenaError::TooManyFrames);
        }
        let bytes = (PAGES as u64)
            .checked_mul(PAGE_SIZE)
            .ok_or(TableArenaError::LengthOverflow)?;
        let window =
            DirectMapWindow::new(phys_base, bytes, virt_offset).map_err(TableArenaError::Window)?;
        Ok(Self {
            phys_base,
            window,
            allocated: [0u64; TABLE_ARENA_BITMAP_WORDS],
            allocated_count: 0,
        })
    }

    /// The physical base frame address of the arena.
    pub const fn phys_base(&self) -> u64 {
        self.phys_base
    }

    /// The validated window covering the whole arena.
    pub const fn window(&self) -> &DirectMapWindow {
        &self.window
    }

    /// The total number of frames the arena tracks.
    pub const fn frame_count(&self) -> usize {
        PAGES
    }

    /// How many frames are currently allocated out of the arena.
    pub const fn allocated_count(&self) -> usize {
        self.allocated_count
    }

    /// Map a frame address to its arena bit index. Rejects unaligned,
    /// out-of-window and beyond-range addresses instead of guessing.
    fn index_of(&self, frame: Frame) -> Option<usize> {
        if frame.start & (PAGE_SIZE - 1) != 0 || frame.start < self.phys_base {
            return None;
        }
        let offset = frame.start - self.phys_base;
        let index = usize::try_from(offset / PAGE_SIZE).ok()?;
        if index >= PAGES { None } else { Some(index) }
    }
}

impl<const PAGES: usize> FramePoolSource for TableArena<PAGES> {
    fn alloc_frame(&mut self) -> Option<Frame> {
        for (word_index, word) in self.allocated.iter_mut().enumerate() {
            if *word == u64::MAX {
                continue;
            }
            let bit = word.trailing_ones();
            let index = word_index * 64 + bit as usize;
            if index >= PAGES {
                continue;
            }
            *word |= 1u64 << bit;
            self.allocated_count += 1;
            return Some(Frame {
                start: self.phys_base + (index as u64) * PAGE_SIZE,
            });
        }
        None
    }

    fn free_frame(&mut self, frame: Frame) -> Result<(), FrameReleaseError> {
        let Some(index) = self.index_of(frame) else {
            return Err(FrameReleaseError::UnknownFrame);
        };
        let mask = 1u64 << (index % 64);
        if self.allocated[index / 64] & mask == 0 {
            return Err(FrameReleaseError::DoubleRelease);
        }
        self.allocated[index / 64] &= !mask;
        self.allocated_count -= 1;
        Ok(())
    }
}

/// Reserve a contiguous table arena on any FrameReserver before the first
/// frame allocation. Returns the reserved byte length. The physical base
/// must already be page aligned; the caller derives it from
/// PhysicalFrameAllocator::peek_contiguous_frames, which returns a page-aligned
/// run outside every installed reservation.
pub fn reserve_table_arena<R: FrameReserver>(
    allocator: &mut R,
    phys_base: u64,
    frame_count: usize,
) -> Result<u64, TableArenaReserveError> {
    // Validate every shape constraint before touching the reservation ledger.
    // An invalid request must not permanently consume physical memory and
    // then fail later when TableArena::open checks the same constraints.
    if frame_count == 0 {
        return Err(TableArenaReserveError::ZeroFrames);
    }
    if frame_count > TABLE_ARENA_MAX_FRAMES {
        return Err(TableArenaReserveError::TooManyFrames);
    }
    if phys_base & (PAGE_SIZE - 1) != 0 {
        return Err(TableArenaReserveError::MisalignedBase);
    }
    let bytes = (frame_count as u64)
        .checked_mul(PAGE_SIZE)
        .ok_or(TableArenaReserveError::RangeOverflow)?;
    phys_base
        .checked_add(bytes)
        .ok_or(TableArenaReserveError::RangeOverflow)?;
    allocator
        .reserve_range(phys_base, bytes)
        .map_err(TableArenaReserveError::Reservation)?;
    Ok(bytes)
}

#[cfg(all(
    feature = "kernel-selftest",
    target_arch = "x86_64",
    target_os = "none"
))]
mod live_probe {
    use super::super::frame::{Frame, PAGE_SIZE, PhysicalFrameAllocator};
    use super::super::paging::{PageFlags, PageTableEntry};
    use super::super::physical_tables::{DirectMapWindow, PhysicalTablePool};
    use super::super::table_frames::TableFramePool;
    use super::{BOOT_TABLE_ARENA_FRAMES, TableArena, reserve_table_arena};

    const PROBE_LEDGER: usize = 8;
    /// The bootstrap identity map covers exactly the first 1 GiB.
    const PROVEN_IDENTITY_LIMIT: u64 = 0x4000_0000;

    /// QEMU evidence probe (kernel-selftest builds only, production boot is
    /// unchanged): reserve a small table arena through the live reservation
    /// ledger, then prove that a real PhysicalTablePool over that arena
    /// allocates, zeroes, exposes, releases and reclaims table frames over
    /// real physical pages, through the proven bootstrap identity window.
    pub fn boot_table_pool_probe(allocator: &mut PhysicalFrameAllocator) {
        use crate::arch::x86_64::{serial_write_str, serial_write_u64};

        let Some(base) = allocator
            .peek_contiguous_frames(BOOT_TABLE_ARENA_FRAMES)
            .map(|frame| frame.start)
        else {
            serial_write_str("AWEOS: [cell:tables] skip: no frame available\n");
            return;
        };
        let Some(bytes) = (BOOT_TABLE_ARENA_FRAMES as u64).checked_mul(PAGE_SIZE) else {
            serial_write_str("AWEOS: [cell:tables] skip: arena length overflow\n");
            return;
        };
        if base >= PROVEN_IDENTITY_LIMIT {
            serial_write_str(
                "AWEOS: [cell:tables] skip: candidate outside the proven 1 GiB identity map\n",
            );
            return;
        }
        if allocator.range_intersects_reservation(base, bytes) {
            serial_write_str(
                "AWEOS: [cell:tables] skip: candidate intersects a boot-owned range\n",
            );
            return;
        }
        if reserve_table_arena(allocator, base, BOOT_TABLE_ARENA_FRAMES).is_err() {
            serial_write_str("AWEOS: [cell:tables] skip: arena reservation failed\n");
            return;
        }
        let mut arena = match TableArena::<BOOT_TABLE_ARENA_FRAMES>::open(base, base) {
            Ok(arena) => arena,
            Err(_) => {
                serial_write_str("AWEOS: [cell:tables] skip: arena window rejected\n");
                return;
            }
        };
        let window: DirectMapWindow = *arena.window();
        // SAFETY: the bootstrap identity map (memory::x86_64) maps the whole
        // first 1 GiB writable, and the arena was checked to lie below 1 GiB.
        let mut pool: PhysicalTablePool<TableArena<BOOT_TABLE_ARENA_FRAMES>, PROBE_LEDGER> =
            unsafe { PhysicalTablePool::with_capacity(window, &mut arena) };

        serial_write_str("AWEOS: [cell:tables] arena phys=0x");
        serial_write_u64(base);
        serial_write_str(" frames=16 window=identity\n");

        let Some(frame) = pool.allocate() else {
            serial_write_str("AWEOS: [cell:tables] FAIL: first allocation\n");
            return;
        };
        let Some(table) = pool.table_mut(frame) else {
            serial_write_str("AWEOS: [cell:tables] FAIL: table access\n");
            return;
        };
        table.set(0, PageTableEntry::new(base, PageFlags::PRESENT));
        let readback = pool
            .table(frame)
            .and_then(|read| read.get(0))
            .is_some_and(|entry| entry.is_present() && entry.address() == base);
        let released = pool.release(frame).is_ok();
        // A tracked-but-free arena frame (never allocated through the pool)
        // must not be releasable: unknown frames stay rejected.
        let foreign_rejected = pool
            .release(Frame {
                start: base + PAGE_SIZE,
            })
            .is_err();
        let rebuilt = match pool.allocate() {
            Some(reused) => {
                let same = reused == frame;
                let zeroed = pool
                    .table(reused)
                    .and_then(|read| read.get(0))
                    .is_none_or(|entry| !entry.is_present());
                same && zeroed
            }
            None => false,
        };

        serial_write_str("AWEOS: [cell:tables] allocated=1 zeroed=yes\n");
        serial_write_str("AWEOS: [cell:tables] write/readback=");
        serial_write_u64(readback as u64);
        serial_write_str(" released=");
        serial_write_u64(released as u64);
        serial_write_str(" foreign-rejected=");
        serial_write_u64(foreign_rejected as u64);
        serial_write_str(" reused-and-rezeroed=");
        serial_write_u64(rebuilt as u64);
        serial_write_str("\n");
        if readback && released && foreign_rejected && rebuilt {
            serial_write_str("AWEOS: [cell:tables] probe result: PASS\n");
        } else {
            serial_write_str("AWEOS: [cell:tables] probe result: FAIL\n");
        }
    }

    /// QEMU evidence probe (kernel-selftest builds only, production boot is
    /// unchanged): through the same proven TableArena => PhysicalTablePool
    /// path, build a real supervisor-only kernel-half page-table root with
    /// the merged `KernelPageTableBuilder` and validate it by readback.
    /// The root is never published: it is never written to CR3, never
    /// loaded by the bootstrap path, and every built frame is released back
    /// to the arena when the probe ends.
    pub fn boot_kernel_root_probe(allocator: &mut PhysicalFrameAllocator) {
        use super::super::kernel_map::KERNEL_HEAP_BASE;
        use super::super::kernel_page_table_build::{KernelMapping, KernelPageTableBuilder};
        use super::super::table_frames::OwnedTableFrames;
        use crate::arch::x86_64::{serial_write_str, serial_write_u64};

        // Same fail-closed guard chain as boot_table_pool_probe: skip
        // instead of touching anything outside the proven identity map.
        let Some(base) = allocator
            .peek_contiguous_frames(BOOT_TABLE_ARENA_FRAMES)
            .map(|frame| frame.start)
        else {
            serial_write_str("AWEOS: [cell:kernmap] skip: no frame available\n");
            return;
        };
        let Some(bytes) = (BOOT_TABLE_ARENA_FRAMES as u64).checked_mul(PAGE_SIZE) else {
            serial_write_str("AWEOS: [cell:kernmap] skip: arena length overflow\n");
            return;
        };
        if base >= PROVEN_IDENTITY_LIMIT {
            serial_write_str(
                "AWEOS: [cell:kernmap] skip: candidate outside the proven 1 GiB identity map\n",
            );
            return;
        }
        if allocator.range_intersects_reservation(base, bytes) {
            serial_write_str(
                "AWEOS: [cell:kernmap] skip: candidate intersects a boot-owned range\n",
            );
            return;
        }
        if reserve_table_arena(allocator, base, BOOT_TABLE_ARENA_FRAMES).is_err() {
            serial_write_str("AWEOS: [cell:kernmap] skip: arena reservation failed\n");
            return;
        }
        let mut arena = match TableArena::<BOOT_TABLE_ARENA_FRAMES>::open(base, base) {
            Ok(arena) => arena,
            Err(_) => {
                serial_write_str("AWEOS: [cell:kernmap] skip: arena window rejected\n");
                return;
            }
        };
        let window: DirectMapWindow = *arena.window();
        // SAFETY: the bootstrap identity map (memory::x86_64) maps the whole
        // first 1 GiB writable, and the arena was checked to lie below 1 GiB.
        let mut pool: PhysicalTablePool<TableArena<BOOT_TABLE_ARENA_FRAMES>, PROBE_LEDGER> =
            unsafe { PhysicalTablePool::with_capacity(window, &mut arena) };

        let Some(root) = pool.allocate() else {
            serial_write_str("AWEOS: [cell:kernmap] FAIL: root allocation\n");
            return;
        };

        // One writable, non-executable, supervisor-only mapping inside the
        // planned kernel-heap region, backed by a page-aligned frame inside
        // the proven identity range.
        let kernel_heap_flags = PageFlags::PRESENT
            .union(PageFlags::WRITABLE)
            .union(PageFlags::NO_EXECUTE);
        let mappings = [KernelMapping {
            virtual_start: KERNEL_HEAP_BASE,
            physical_start: PAGE_SIZE,
            page_count: 1,
            flags: kernel_heap_flags,
        }];

        let install;
        let readback;
        let untouched;
        let mut built_frames = [0u64; 8];
        let mut built_count = 0usize;
        {
            let mut access = OwnedTableFrames::<PhysicalTablePool<_>, 64>::new(&mut pool);
            let mut builder = match KernelPageTableBuilder::new(root.start, &mut access) {
                Ok(builder) => builder,
                Err(_) => {
                    serial_write_str("AWEOS: [cell:kernmap] FAIL: builder rejected root\n");
                    pool.release(root).ok();
                    return;
                }
            };
            install = builder.install_all(&mappings);
            readback = install.is_ok()
                && builder.leaf_entry(KERNEL_HEAP_BASE).is_some_and(|leaf| {
                    leaf.address() == PAGE_SIZE
                        && !leaf.flags().contains(PageFlags::USER)
                        && leaf.flags().contains(PageFlags::WRITABLE)
                        && leaf.flags().contains(PageFlags::NO_EXECUTE)
                });
            untouched = builder.user_half_is_untouched();
            for (index, &frame) in builder.allocated_tables().iter().enumerate() {
                if index < built_frames.len() {
                    built_frames[index] = frame;
                    built_count = index + 1;
                }
            }
        }

        serial_write_str("AWEOS: [cell:kernmap] install=");
        serial_write_u64(u64::from(install.is_ok()));
        serial_write_str(" readback=");
        serial_write_u64(u64::from(readback));
        serial_write_str(" user-half-untouched=");
        serial_write_u64(u64::from(untouched));
        serial_write_str(" tables=");
        serial_write_u64(u64::try_from(built_count).unwrap_or(0));
        serial_write_str("\n");

        // The root and every built frame remain unpublished: release them
        // all back to the arena so the probe leaves the table pool empty.
        let mut all_released = true;
        let mut index = built_count;
        while index > 0 {
            index -= 1;
            if pool
                .release(Frame {
                    start: built_frames[index],
                })
                .is_err()
            {
                all_released = false;
            }
        }
        if pool.release(root).is_err() {
            all_released = false;
        }

        serial_write_str("AWEOS: [cell:kernmap] released=");
        serial_write_u64(u64::from(all_released));
        serial_write_str("\n");
        if install.is_ok() && readback && untouched && all_released {
            serial_write_str("AWEOS: [cell:kernmap] probe result: PASS\n");
        } else {
            serial_write_str("AWEOS: [cell:kernmap] probe result: FAIL\n");
        }
    }
}

#[cfg(all(
    feature = "kernel-selftest",
    target_arch = "x86_64",
    target_os = "none"
))]
pub use live_probe::boot_table_pool_probe;

#[cfg(all(
    feature = "kernel-selftest",
    target_arch = "x86_64",
    target_os = "none"
))]
pub use live_probe::boot_kernel_root_probe;
