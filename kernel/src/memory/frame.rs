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

pub const fn frame_at(address: u64) -> Frame {
    Frame {
        start: address & !(PAGE_SIZE - 1),
    }
}

pub const fn frame_end(frame: Frame) -> Option<u64> {
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
}

impl PhysicalFrameAllocator {
    pub const fn empty() -> Self {
        Self {
            regions: core::ptr::null(),
            region_count: 0,
            region_index: 0,
            cursor: 0,
            end: 0,
        }
    }

    /// # Safety
    /// The memory-region array must be readable for memory_region_count
    /// descriptors, aligned for MemoryRegion, and remain valid and immutable
    /// for this allocator's lifetime. The BootInfo object must also be valid.
    pub unsafe fn from_boot_info(info: &BootInfo) -> Result<Self, FrameAllocatorError> {
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
        if address % align_of::<MemoryRegion>() != 0 {
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
        };
        allocator.select_next_region();
        Ok(allocator)
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

    pub fn allocate(&mut self) -> Option<Frame> {
        if self.cursor == 0 || self.cursor >= self.end {
            self.select_next_region();
        }
        if self.cursor == 0 {
            return None;
        }
        let frame = Frame { start: self.cursor };
        self.cursor = self.cursor.checked_add(PAGE_SIZE).unwrap_or(0);
        Some(frame)
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
}

pub struct BitmapFrameAllocator<const MAX_BITMAP_WORDS: usize> {
    bitmap: [u64; MAX_BITMAP_WORDS],
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
            base_address,
            total_frames,
            valid,
        }
    }

    pub const fn is_valid(&self) -> bool {
        self.valid
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

    pub fn free_frame(&mut self, frame: Frame) -> Result<(), BitmapFrameError> {
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
        if self.bitmap[word_idx] & mask == 0 {
            return Err(BitmapFrameError::DoubleFree);
        }
        self.bitmap[word_idx] &= !mask;
        Ok(())
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
        assert_eq!(frame_end(frame_at(0x1234)), Some(0x2000));
        assert_eq!(frame_end(frame_at(u64::MAX)), None);
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
        let mut allocator = unsafe { PhysicalFrameAllocator::from_boot_info(&info) }.unwrap();
        assert_eq!(allocator.allocate().unwrap().start, 0x1000);
        assert_eq!(allocator.allocate().unwrap().start, 0x4000);
        assert_eq!(allocator.allocate().unwrap().start, 0x5000);
        assert_eq!(allocator.allocate().unwrap().start, 0x6000);
        assert!(allocator.allocate().is_none());
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
            unsafe { PhysicalFrameAllocator::from_boot_info(&info) }.err(),
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
            unsafe { PhysicalFrameAllocator::from_boot_info(&info) }.err(),
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
            unsafe { PhysicalFrameAllocator::from_boot_info(&zero_info) }.err(),
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
            unsafe { PhysicalFrameAllocator::from_boot_info(&unknown_info) }.err(),
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
        allocator.free_frame(first).unwrap();
        assert_eq!(
            allocator.free_frame(first),
            Err(BitmapFrameError::DoubleFree)
        );
        assert_eq!(
            allocator.free_frame(Frame {
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
}
