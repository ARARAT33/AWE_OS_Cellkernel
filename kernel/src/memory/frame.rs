#![no_std]

use core::mem::{align_of, size_of};

use awe_boot_protocol::{
    BootInfo, MAX_MEMORY_REGIONS, MEMORY_REGION_MMIO, MEMORY_REGION_RECLAIMABLE,
    MEMORY_REGION_RESERVED, MEMORY_REGION_USABLE, MemoryRegion,
};

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Frame {
    pub start: u64,
}

pub const PAGE_SIZE: u64 = 4096;
/// Fixed reservation ledger for boot-owned physical ranges. The v1 boot
/// protocol currently derives at most five ranges.
pub const MAX_PHYSICAL_RESERVED_RANGES: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq)]
struct ReservedPhysicalRange {
    start: u64,
    end: u64,
}

pub const fn frame_at(address: u64) -> Frame {
    Frame {
        start: address & !(PAGE_SIZE - 1),
    }
}

/// Legacy exclusive-end helper. Use checked_frame_end when overflow must be distinguished.
pub const fn frame_end(frame: Frame) -> u64 {
    frame.start.saturating_add(PAGE_SIZE)
}

pub const fn checked_frame_end(frame: Frame) -> Option<u64> {
    frame.start.checked_add(PAGE_SIZE)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FrameAllocatorError {
    InvalidBootInfo,
    MissingMemoryMap,
    TooManyRegions,
    MisalignedMemoryMap,
    MemoryMapAddressOverflow,
    EmptyRegion,
    RegionAddressOverflow,
    InvalidRegionKind,
    OverlappingOrUnsortedRegions,
}

pub struct PhysicalFrameAllocator {
    regions: *const MemoryRegion,
    region_count: u32,
    region_index: u32,
    cursor: u64,
    end: u64,
    reserved_ranges: [Option<ReservedPhysicalRange>; MAX_PHYSICAL_RESERVED_RANGES],
    reserved_count: usize,
    allocations_started: bool,
}

impl PhysicalFrameAllocator {
    pub const fn empty() -> Self {
        Self {
            regions: core::ptr::null(),
            region_count: 0,
            region_index: 0,
            cursor: 0,
            end: 0,
            reserved_ranges: [None; MAX_PHYSICAL_RESERVED_RANGES],
            reserved_count: 0,
            allocations_started: false,
        }
    }

    /// # Safety
    /// The memory-region array must be readable for memory_region_count
    /// descriptors, aligned for MemoryRegion, and remain valid and immutable
    /// for this allocator's lifetime. The BootInfo object must also be valid.
    pub unsafe fn try_from_boot_info(info: &BootInfo) -> Result<Self, FrameAllocatorError> {
        if !awe_boot_protocol::validate(info) {
            return Err(FrameAllocatorError::InvalidBootInfo);
        }
        if info.memory_region_count == 0 || info.memory_regions.is_null() {
            return Err(FrameAllocatorError::MissingMemoryMap);
        }
        if info.memory_region_count > MAX_MEMORY_REGIONS {
            return Err(FrameAllocatorError::TooManyRegions);
        }

        let regions = info.memory_regions;
        let address = regions as usize;
        if !address.is_multiple_of(align_of::<MemoryRegion>()) {
            return Err(FrameAllocatorError::MisalignedMemoryMap);
        }
        let count = info.memory_region_count as usize;
        let bytes = count
            .checked_mul(size_of::<MemoryRegion>())
            .ok_or(FrameAllocatorError::MemoryMapAddressOverflow)?;
        address
            .checked_add(bytes)
            .ok_or(FrameAllocatorError::MemoryMapAddressOverflow)?;

        let mut previous_end: Option<u64> = None;
        for index in 0..count {
            let region = unsafe { regions.add(index).read() };
            if region.length == 0 {
                return Err(FrameAllocatorError::EmptyRegion);
            }
            let end = region
                .base
                .checked_add(region.length)
                .ok_or(FrameAllocatorError::RegionAddressOverflow)?;
            if !matches!(
                region.kind,
                MEMORY_REGION_USABLE
                    | MEMORY_REGION_RESERVED
                    | MEMORY_REGION_RECLAIMABLE
                    | MEMORY_REGION_MMIO
            ) {
                return Err(FrameAllocatorError::InvalidRegionKind);
            }
            if previous_end.is_some_and(|previous| region.base < previous) {
                return Err(FrameAllocatorError::OverlappingOrUnsortedRegions);
            }
            previous_end = Some(end);
        }

        let mut allocator = Self {
            regions,
            region_count: info.memory_region_count,
            region_index: 0,
            cursor: 0,
            end: 0,
            reserved_ranges: [None; MAX_PHYSICAL_RESERVED_RANGES],
            reserved_count: 0,
            allocations_started: false,
        };
        allocator.select_next_region();
        Ok(allocator)
    }

    /// Compatibility wrapper for callers that only need a best-effort cursor.
    /// New boot code should use try_from_boot_info and handle its error.
    ///
    /// # Safety
    /// The memory-map pointer must satisfy the same validity requirements as
    /// try_from_boot_info.
    pub unsafe fn from_boot_info(info: &BootInfo) -> Self {
        unsafe { Self::try_from_boot_info(info) }.unwrap_or_else(|_| Self::empty())
    }

    fn select_next_region(&mut self) {
        while self.region_index < self.region_count {
            let region = unsafe { self.regions.add(self.region_index as usize).read() };
            self.region_index += 1;
            if region.kind != MEMORY_REGION_USABLE || region.length < PAGE_SIZE {
                continue;
            }

            let minimum = region.base.max(PAGE_SIZE);
            let start = match minimum.checked_add(PAGE_SIZE - 1) {
                Some(value) => value & !(PAGE_SIZE - 1),
                None => continue,
            };
            let raw_end = match region.base.checked_add(region.length) {
                Some(value) => value,
                None => continue,
            };
            let end = raw_end & !(PAGE_SIZE - 1);
            if start < end {
                self.cursor = start;
                self.end = end;
                return;
            }
        }
        self.cursor = 0;
        self.end = 0;
    }

    /// Preview a contiguous run of `frame_count` unreserved frames without
    /// allocating them. The cursor may advance past permanently reserved
    /// ranges or regions too short for the requested run, but the allocator
    /// remains in its pre-allocation state and the returned frames are still
    /// available to the next allocate() calls until a reservation is made.
    pub fn peek_contiguous_frames(&mut self, frame_count: usize) -> Option<Frame> {
        if frame_count == 0 {
            return None;
        }
        let bytes = u64::try_from(frame_count).ok()?.checked_mul(PAGE_SIZE)?;
        loop {
            if self.cursor == 0 || self.cursor >= self.end {
                self.select_next_region();
            }
            if self.cursor == 0 {
                return None;
            }

            let start = self.cursor;
            let run_end = start.checked_add(bytes)?;
            if run_end > self.end {
                // This usable region is too short; continue with the next one.
                self.cursor = self.end;
                continue;
            }
            if self.range_intersects_reservation(start, bytes) {
                // Move by one page so a later candidate can begin just after
                // the reservation while preserving a contiguous run.
                self.cursor = self.cursor.checked_add(PAGE_SIZE).unwrap_or(0);
                continue;
            }
            return Some(Frame { start });
        }
    }

    /// Preview the next frame that allocate() would return, skipping any
    /// installed reservation without consuming the frame.
    pub fn peek_next_frame(&mut self) -> Option<Frame> {
        self.peek_contiguous_frames(1)
    }

    /// Conservative check: whether any 4 KiB frame intersecting
    /// [start, start + length) is excluded by an installed reservation.
    /// An overflowing request is reported as intersecting, never free.
    pub fn range_intersects_reservation(&self, start: u64, length: u64) -> bool {
        let Some(end) = start.checked_add(length) else {
            return true;
        };
        self.reserved_ranges[..self.reserved_count]
            .iter()
            .flatten()
            .any(|range| range.start < end && start < range.end)
    }

    /// Check a physical byte range before reserving it. Reservations must be
    /// installed before the first allocation; exact/contained repeats remain
    /// idempotent so the boot handoff can be safely revalidated.
    pub fn check_reserve_range(&self, start: u64, length: u64) -> Result<(), BitmapFrameError> {
        if length == 0 {
            return Err(BitmapFrameError::EmptyRange);
        }
        let end = start
            .checked_add(length)
            .ok_or(BitmapFrameError::RangeOverflow)?;
        if self.reserved_ranges[..self.reserved_count]
            .iter()
            .flatten()
            .any(|range| start >= range.start && end <= range.end)
        {
            return Ok(());
        }
        if self.allocations_started {
            return Err(BitmapFrameError::ReservationAfterAllocation);
        }
        if self.reserved_count >= MAX_PHYSICAL_RESERVED_RANGES {
            return Err(BitmapFrameError::ReservationCapacityExceeded);
        }
        Ok(())
    }

    /// Atomically reserve a batch of physical byte ranges. Capacity and
    /// lifecycle checks are completed for the whole batch before the ledger
    /// is changed, preserving the boot handoff's all-or-nothing contract.
    pub fn reserve_ranges(
        &mut self,
        ranges: &[(u64, u64)],
    ) -> Result<(), (usize, BitmapFrameError)> {
        let mut additions = [None; MAX_PHYSICAL_RESERVED_RANGES];
        let mut addition_count = 0usize;

        for (index, (start, length)) in ranges.iter().copied().enumerate() {
            self.check_reserve_range(start, length)
                .map_err(|error| (index, error))?;
            let end = start
                .checked_add(length)
                .ok_or((index, BitmapFrameError::RangeOverflow))?;

            let already_covered = self.reserved_ranges[..self.reserved_count]
                .iter()
                .flatten()
                .any(|range| start >= range.start && end <= range.end)
                || additions[..addition_count]
                    .iter()
                    .flatten()
                    .any(|range: &ReservedPhysicalRange| start >= range.start && end <= range.end);

            if !already_covered {
                if self.reserved_count + addition_count >= MAX_PHYSICAL_RESERVED_RANGES {
                    return Err((index, BitmapFrameError::ReservationCapacityExceeded));
                }
                additions[addition_count] = Some(ReservedPhysicalRange { start, end });
                addition_count += 1;
            }
        }

        for range in additions[..addition_count].iter().flatten() {
            self.reserved_ranges[self.reserved_count] = Some(*range);
            self.reserved_count += 1;
        }
        Ok(())
    }

    /// Permanently exclude every 4 KiB frame intersecting the byte range.
    /// This allocator is monotonic, so reservations are accepted only before
    /// the first frame is handed out.
    pub fn reserve_range(&mut self, start: u64, length: u64) -> Result<(), BitmapFrameError> {
        self.reserve_ranges(&[(start, length)])
            .map_err(|(_, error)| error)
    }

    fn frame_intersects_reservation(&self, frame_start: u64) -> bool {
        let Some(frame_end) = frame_start.checked_add(PAGE_SIZE) else {
            return true;
        };
        self.reserved_ranges[..self.reserved_count]
            .iter()
            .flatten()
            .any(|range| range.start < frame_end && frame_start < range.end)
    }

    pub fn allocate(&mut self) -> Option<Frame> {
        loop {
            if self.cursor == 0 || self.cursor >= self.end {
                self.select_next_region();
            }
            if self.cursor == 0 {
                return None;
            }

            let start = self.cursor;
            self.cursor = self.cursor.checked_add(PAGE_SIZE).unwrap_or(0);
            if self.frame_intersects_reservation(start) {
                continue;
            }

            self.allocations_started = true;
            return Some(Frame { start });
        }
    }

    pub fn remaining_in_current_region(&self) -> u64 {
        self.end.saturating_sub(self.cursor) / PAGE_SIZE
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BitmapFrameError {
    InvalidAllocator,
    InvalidFrame,
    DoubleFree,
    EmptyRange,
    RangeOverflow,
    RangeOutOfBounds,
    FrameAlreadyAllocated,
    ReservedFrame,
    ReservationAfterAllocation,
    ReservationCapacityExceeded,
}

pub struct BitmapFrameAllocator<const MAX_BITMAP_WORDS: usize> {
    /// Set for frames that are either allocated or permanently reserved.
    bitmap: [u64; MAX_BITMAP_WORDS],
    /// Set only for reserved frames, which must never be returned by free.
    reserved: [u64; MAX_BITMAP_WORDS],
    base_address: u64,
    total_frames: usize,
    valid: bool,
}

impl<const MAX_BITMAP_WORDS: usize> BitmapFrameAllocator<MAX_BITMAP_WORDS> {
    pub const fn new(base_address: u64, total_frames: usize) -> Self {
        let capacity = MAX_BITMAP_WORDS.saturating_mul(64);
        let range_valid = match (total_frames as u64).checked_mul(PAGE_SIZE) {
            Some(bytes) => base_address.checked_add(bytes).is_some(),
            None => false,
        };
        let valid = base_address & (PAGE_SIZE - 1) == 0 && total_frames <= capacity && range_valid;
        Self {
            bitmap: [0u64; MAX_BITMAP_WORDS],
            reserved: [0u64; MAX_BITMAP_WORDS],
            base_address,
            total_frames,
            valid,
        }
    }

    pub const fn is_valid(&self) -> bool {
        self.valid
    }

    /// Validate a reservation request without mutating any bitmap state.
    ///
    /// This is the preflight half of reserve_range: the same empty, overflow,
    /// bounds and already-allocated checks, but no bit is set. Use it when a
    /// set of reservations must be validated as a whole before any of them
    /// is committed.
    pub fn check_reserve_range(&self, start: u64, length: u64) -> Result<(), BitmapFrameError> {
        if !self.valid {
            return Err(BitmapFrameError::InvalidAllocator);
        }
        if length == 0 {
            return Err(BitmapFrameError::EmptyRange);
        }

        let end = start
            .checked_add(length)
            .ok_or(BitmapFrameError::RangeOverflow)?;
        let managed_bytes = (self.total_frames as u64)
            .checked_mul(PAGE_SIZE)
            .ok_or(BitmapFrameError::InvalidAllocator)?;
        let managed_end = self
            .base_address
            .checked_add(managed_bytes)
            .ok_or(BitmapFrameError::InvalidAllocator)?;
        if start < self.base_address || end > managed_end {
            return Err(BitmapFrameError::RangeOutOfBounds);
        }

        let first_frame = usize::try_from((start - self.base_address) / PAGE_SIZE)
            .map_err(|_| BitmapFrameError::RangeOutOfBounds)?;
        let relative_end = end - self.base_address;
        let last_frame_exclusive = usize::try_from(relative_end.div_ceil(PAGE_SIZE))
            .map_err(|_| BitmapFrameError::RangeOutOfBounds)?;
        if first_frame >= last_frame_exclusive || last_frame_exclusive > self.total_frames {
            return Err(BitmapFrameError::RangeOutOfBounds);
        }

        for frame_idx in first_frame..last_frame_exclusive {
            let word_idx = frame_idx / 64;
            let mask = 1u64 << (frame_idx % 64);
            let allocated = self.bitmap[word_idx] & mask != 0;
            let reserved = self.reserved[word_idx] & mask != 0;
            if allocated && !reserved {
                return Err(BitmapFrameError::FrameAlreadyAllocated);
            }
        }
        Ok(())
    }

    /// Reserve every 4 KiB frame intersecting the non-empty byte range
    /// [start, start + length).
    ///
    /// Unaligned boundaries are rounded outwards so no partially covered
    /// frame can be handed to the allocator. The range must be fully inside
    /// this allocator's managed physical interval. Repeating an existing
    /// reservation is safe; a range containing a frame already allocated
    /// through this allocator is rejected without changing any bits.
    pub fn reserve_range(&mut self, start: u64, length: u64) -> Result<(), BitmapFrameError> {
        if !self.valid {
            return Err(BitmapFrameError::InvalidAllocator);
        }
        if length == 0 {
            return Err(BitmapFrameError::EmptyRange);
        }

        let end = start
            .checked_add(length)
            .ok_or(BitmapFrameError::RangeOverflow)?;
        let managed_bytes = (self.total_frames as u64)
            .checked_mul(PAGE_SIZE)
            .ok_or(BitmapFrameError::InvalidAllocator)?;
        let managed_end = self
            .base_address
            .checked_add(managed_bytes)
            .ok_or(BitmapFrameError::InvalidAllocator)?;
        if start < self.base_address || end > managed_end {
            return Err(BitmapFrameError::RangeOutOfBounds);
        }

        // Preflight the complete range first: a conflict must not leave a
        // partially reserved prefix behind. check_reserve_range performs the
        // identical empty, overflow, bounds and allocated-frame validation
        // without touching any bitmap state.
        self.check_reserve_range(start, length)?;

        let first_frame = usize::try_from((start - self.base_address) / PAGE_SIZE)
            .map_err(|_| BitmapFrameError::RangeOutOfBounds)?;
        let relative_end = end - self.base_address;
        let last_frame_exclusive = usize::try_from(relative_end.div_ceil(PAGE_SIZE))
            .map_err(|_| BitmapFrameError::RangeOutOfBounds)?;

        for frame_idx in first_frame..last_frame_exclusive {
            let word_idx = frame_idx / 64;
            let mask = 1u64 << (frame_idx % 64);
            self.bitmap[word_idx] |= mask;
            self.reserved[word_idx] |= mask;
        }
        Ok(())
    }

    pub fn allocate_frame(&mut self) -> Option<Frame> {
        if !self.valid {
            return None;
        }
        for word_idx in 0..self.bitmap.len() {
            let available = !self.bitmap[word_idx];
            if available == 0 {
                continue;
            }
            let bit_idx = available.trailing_zeros() as usize;
            let frame_idx = word_idx * 64 + bit_idx;
            if frame_idx >= self.total_frames {
                continue;
            }
            let offset = (frame_idx as u64).checked_mul(PAGE_SIZE)?;
            let start = self.base_address.checked_add(offset)?;
            self.bitmap[word_idx] |= 1u64 << bit_idx;
            return Some(Frame { start });
        }
        None
    }

    pub fn try_free_frame(&mut self, frame: Frame) -> Result<(), BitmapFrameError> {
        if !self.valid {
            return Err(BitmapFrameError::InvalidAllocator);
        }
        if frame.start < self.base_address || frame.start & (PAGE_SIZE - 1) != 0 {
            return Err(BitmapFrameError::InvalidFrame);
        }
        let offset = frame.start - self.base_address;
        let frame_idx =
            usize::try_from(offset / PAGE_SIZE).map_err(|_| BitmapFrameError::InvalidFrame)?;
        if frame_idx >= self.total_frames {
            return Err(BitmapFrameError::InvalidFrame);
        }
        let word_idx = frame_idx / 64;
        let bit_idx = frame_idx % 64;
        let mask = 1u64 << bit_idx;
        if self.reserved[word_idx] & mask != 0 {
            return Err(BitmapFrameError::ReservedFrame);
        }
        if self.bitmap[word_idx] & mask == 0 {
            return Err(BitmapFrameError::DoubleFree);
        }
        self.bitmap[word_idx] &= !mask;
        Ok(())
    }

    /// Compatibility wrapper. Use try_free_frame when the caller must observe
    /// invalid-frame and double-free errors.
    pub fn free_frame(&mut self, frame: Frame) {
        let _ = self.try_free_frame(frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use awe_boot_protocol::{Architecture, MemoryRegion};

    fn boot_info(regions: &[MemoryRegion]) -> BootInfo {
        BootInfo {
            magic: awe_boot_protocol::AWE_BOOT_MAGIC,
            version: awe_boot_protocol::AWE_BOOT_VERSION,
            size: core::mem::size_of::<BootInfo>() as u32,
            architecture: Architecture::X86_64,
            cpu_count: 1,
            memory_regions: regions.as_ptr(),
            memory_region_count: regions.len() as u32,
            framebuffer_address: 0,
            framebuffer_size: 0,
            framebuffer_width: 0,
            framebuffer_height: 0,
            framebuffer_pitch: 0,
            acpi_rsdp: 0,
            device_tree: 0,
            kernel_base: 0,
            kernel_size: 0,
        }
    }

    #[test]
    fn frame_alignment_and_end_are_checked() {
        assert_eq!(frame_at(0x1234).start, 0x1000);
        assert_eq!(checked_frame_end(frame_at(0x1234)), Some(0x2000));
        assert_eq!(checked_frame_end(frame_at(u64::MAX)), None);
    }

    #[test]
    fn allocator_skips_reserved_memory_and_page_zero() {
        let regions = [
            MemoryRegion {
                base: 0,
                length: 0x2000,
                kind: MEMORY_REGION_USABLE,
                reserved: 0,
            },
            MemoryRegion {
                base: 0x2000,
                length: 0x2000,
                kind: MEMORY_REGION_RESERVED,
                reserved: 0,
            },
            MemoryRegion {
                base: 0x4000,
                length: 0x3000,
                kind: MEMORY_REGION_USABLE,
                reserved: 0,
            },
        ];
        let info = boot_info(&regions);
        let mut allocator = unsafe { PhysicalFrameAllocator::try_from_boot_info(&info) }.unwrap();
        assert_eq!(allocator.allocate().unwrap().start, 0x1000);
        assert_eq!(allocator.allocate().unwrap().start, 0x4000);
        assert_eq!(allocator.allocate().unwrap().start, 0x5000);
        assert_eq!(allocator.allocate().unwrap().start, 0x6000);
        assert!(allocator.allocate().is_none());
    }

    #[test]
    fn physical_allocator_skips_every_frame_intersecting_a_reservation() {
        let regions = [MemoryRegion {
            base: 0x1000,
            length: 0x8000,
            kind: MEMORY_REGION_USABLE,
            reserved: 0,
        }];
        let info = boot_info(&regions);
        let mut allocator = unsafe { PhysicalFrameAllocator::try_from_boot_info(&info) }.unwrap();

        // [0x2800, 0x4000) intersects frames 0x2000 and 0x3000.
        allocator.reserve_range(0x2800, 0x1800).unwrap();
        // A fully covered reservation is idempotent.
        allocator.reserve_range(0x3000, 0x1000).unwrap();

        assert_eq!(allocator.allocate().unwrap().start, 0x1000);
        assert_eq!(allocator.allocate().unwrap().start, 0x4000);
        assert_eq!(allocator.allocate().unwrap().start, 0x5000);
    }

    #[test]
    fn physical_allocator_batch_reservation_is_atomic_on_capacity_failure() {
        let mut allocator = PhysicalFrameAllocator::empty();
        let mut ranges = [(0u64, 0u64); MAX_PHYSICAL_RESERVED_RANGES + 1];
        for (index, range) in ranges.iter_mut().enumerate() {
            *range = (0x1000 + (index as u64 * 0x2000), 0x1000);
        }

        assert_eq!(
            allocator.reserve_ranges(&ranges),
            Err((
                MAX_PHYSICAL_RESERVED_RANGES,
                BitmapFrameError::ReservationCapacityExceeded
            ))
        );
        // The failed batch committed nothing.
        assert_eq!(allocator.reserve_range(0x1000, 0x1000), Ok(()));
    }

    #[test]
    fn physical_allocator_rejects_new_reservations_after_allocation() {
        let regions = [MemoryRegion {
            base: 0x1000,
            length: 0x4000,
            kind: MEMORY_REGION_USABLE,
            reserved: 0,
        }];
        let info = boot_info(&regions);
        let mut allocator = unsafe { PhysicalFrameAllocator::try_from_boot_info(&info) }.unwrap();

        assert_eq!(allocator.allocate().unwrap().start, 0x1000);
        assert_eq!(
            allocator.reserve_range(0x2000, 0x1000),
            Err(BitmapFrameError::ReservationAfterAllocation)
        );
        // Rejected reservation did not skip or consume the next frame.
        assert_eq!(allocator.allocate().unwrap().start, 0x2000);
    }

    #[test]
    fn peek_next_frame_previews_without_consuming() {
        let regions = [MemoryRegion {
            base: 0x1000,
            length: 0x4000,
            kind: MEMORY_REGION_USABLE,
            reserved: 0,
        }];
        let info = boot_info(&regions);
        let mut allocator = unsafe { PhysicalFrameAllocator::try_from_boot_info(&info) }.unwrap();
        assert_eq!(allocator.peek_next_frame().unwrap().start, 0x1000);
        assert_eq!(allocator.peek_next_frame().unwrap().start, 0x1000);
        assert_eq!(allocator.allocate().unwrap().start, 0x1000);
        assert_eq!(allocator.peek_next_frame().unwrap().start, 0x2000);
        assert_eq!(allocator.allocate().unwrap().start, 0x2000);
    }

    #[test]
    fn peek_contiguous_frames_skips_reservations_without_allocating() {
        let regions = [MemoryRegion {
            base: 0x1000,
            length: 0x10000,
            kind: MEMORY_REGION_USABLE,
            reserved: 0,
        }];
        let info = boot_info(&regions);
        let mut allocator = unsafe { PhysicalFrameAllocator::try_from_boot_info(&info) }.unwrap();
        allocator.reserve_range(0x2000, 0x3000).unwrap();

        // The requested four-page run must start after the three reserved
        // pages; previewing it twice must not consume the first free frame.
        assert_eq!(allocator.peek_contiguous_frames(4).unwrap().start, 0x5000);
        assert_eq!(allocator.peek_next_frame().unwrap().start, 0x5000);
        assert_eq!(allocator.allocate().unwrap().start, 0x5000);
    }

    #[test]
    fn reservation_intersection_report_is_conservative() {
        let regions = [MemoryRegion {
            base: 0x1000,
            length: 0x8000,
            kind: MEMORY_REGION_USABLE,
            reserved: 0,
        }];
        let info = boot_info(&regions);
        let mut allocator = unsafe { PhysicalFrameAllocator::try_from_boot_info(&info) }.unwrap();
        assert!(!allocator.range_intersects_reservation(0x1000, 0x2000));
        // [0x2000, 0x3800) intersects frames 0x2000 through 0x3000.
        allocator.reserve_range(0x2000, 0x1800).unwrap();
        assert!(allocator.range_intersects_reservation(0x1000, 0x2000));
        assert!(allocator.range_intersects_reservation(0x3000, 0x1000));
        assert!(!allocator.range_intersects_reservation(0x4000, 0x1000));
        // An overflowing request is reported as intersecting, never free.
        assert!(allocator.range_intersects_reservation(u64::MAX, 1));
    }

    #[test]
    fn rejects_overlapping_or_unsorted_memory_map() {
        let regions = [
            MemoryRegion {
                base: 0x1000,
                length: 0x3000,
                kind: MEMORY_REGION_USABLE,
                reserved: 0,
            },
            MemoryRegion {
                base: 0x3000,
                length: 0x1000,
                kind: MEMORY_REGION_RESERVED,
                reserved: 0,
            },
        ];
        let info = boot_info(&regions);
        assert_eq!(
            unsafe { PhysicalFrameAllocator::try_from_boot_info(&info) }.err(),
            Some(FrameAllocatorError::OverlappingOrUnsortedRegions)
        );
    }

    #[test]
    fn rejects_overflowing_memory_region() {
        let regions = [MemoryRegion {
            base: u64::MAX - 0xfff,
            length: 0x2000,
            kind: MEMORY_REGION_USABLE,
            reserved: 0,
        }];
        let info = boot_info(&regions);
        assert_eq!(
            unsafe { PhysicalFrameAllocator::try_from_boot_info(&info) }.err(),
            Some(FrameAllocatorError::RegionAddressOverflow)
        );
    }

    #[test]
    fn rejects_zero_length_and_unknown_kind() {
        let zero = [MemoryRegion {
            base: 0x1000,
            length: 0,
            kind: MEMORY_REGION_USABLE,
            reserved: 0,
        }];
        let zero_info = boot_info(&zero);
        assert_eq!(
            unsafe { PhysicalFrameAllocator::try_from_boot_info(&zero_info) }.err(),
            Some(FrameAllocatorError::EmptyRegion)
        );

        let unknown = [MemoryRegion {
            base: 0x1000,
            length: 0x1000,
            kind: 99,
            reserved: 0,
        }];
        let unknown_info = boot_info(&unknown);
        assert_eq!(
            unsafe { PhysicalFrameAllocator::try_from_boot_info(&unknown_info) }.err(),
            Some(FrameAllocatorError::InvalidRegionKind)
        );
    }

    #[test]
    fn bitmap_allocator_rejects_invalid_ranges_and_double_free() {
        let mut allocator = BitmapFrameAllocator::<4>::new(0x100000, 10);
        assert!(allocator.is_valid());
        let first = allocator.allocate_frame().unwrap();
        let second = allocator.allocate_frame().unwrap();
        assert_eq!(first.start, 0x100000);
        assert_eq!(second.start, 0x101000);
        allocator.try_free_frame(first).unwrap();
        assert_eq!(
            allocator.try_free_frame(first),
            Err(BitmapFrameError::DoubleFree)
        );
        assert_eq!(
            allocator.try_free_frame(Frame {
                start: second.start + 1
            }),
            Err(BitmapFrameError::InvalidFrame)
        );
        assert_eq!(allocator.allocate_frame().unwrap().start, first.start);
    }

    #[test]
    fn bitmap_allocator_rejects_capacity_and_address_overflow() {
        let mut too_many = BitmapFrameAllocator::<1>::new(0x1000, 65);
        assert!(!too_many.is_valid());
        assert!(too_many.allocate_frame().is_none());

        let mut overflow = BitmapFrameAllocator::<1>::new(u64::MAX - 0xfff, 1);
        assert!(!overflow.is_valid());
        assert!(overflow.allocate_frame().is_none());
    }
    #[test]
    fn bitmap_reservations_round_outwards_and_are_idempotent() {
        let mut allocator = BitmapFrameAllocator::<1>::new(0x1000, 8);
        allocator.reserve_range(0x1801, 0x1000).unwrap();
        allocator.reserve_range(0x1801, 0x1000).unwrap();

        assert_eq!(
            allocator.try_free_frame(Frame { start: 0x1000 }),
            Err(BitmapFrameError::ReservedFrame)
        );
        assert_eq!(
            allocator.try_free_frame(Frame { start: 0x2000 }),
            Err(BitmapFrameError::ReservedFrame)
        );
        assert_eq!(allocator.allocate_frame().unwrap().start, 0x3000);
    }

    #[test]
    fn bitmap_reservation_conflict_is_atomic() {
        let mut allocator = BitmapFrameAllocator::<1>::new(0x1000, 8);
        let allocated = allocator.allocate_frame().unwrap();
        assert_eq!(allocated.start, 0x1000);

        assert_eq!(
            allocator.reserve_range(0x1000, 0x2000),
            Err(BitmapFrameError::FrameAlreadyAllocated)
        );
        // The second page was not reserved as a side effect of the conflict.
        assert_eq!(allocator.allocate_frame().unwrap().start, 0x2000);
    }

    #[test]
    fn bitmap_reservation_rejects_empty_overflow_and_out_of_range_inputs() {
        let mut allocator = BitmapFrameAllocator::<1>::new(0x1000, 8);
        assert_eq!(
            allocator.reserve_range(0x1000, 0),
            Err(BitmapFrameError::EmptyRange)
        );
        assert_eq!(
            allocator.reserve_range(u64::MAX - 1, 4),
            Err(BitmapFrameError::RangeOverflow)
        );
        assert_eq!(
            allocator.reserve_range(0x8fff, 2),
            Err(BitmapFrameError::RangeOutOfBounds)
        );
        assert_eq!(allocator.allocate_frame().unwrap().start, 0x1000);
    }
}
