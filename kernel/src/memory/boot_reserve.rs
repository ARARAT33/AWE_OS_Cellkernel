#![no_std]

//! Boot-owned physical range reservation.
//!
//! The bootloader hands the kernel a validated memory map, but the kernel
//! itself and the boot artifacts it depends on live inside otherwise usable
//! RAM: the kernel image, the BootInfo structure, the memory-map descriptor
//! array, the framebuffer and the ACPI RSDP page. Until those ranges are
//! reserved, the physical allocator can hand out frames that back live
//! boot state.
//!
//! This module computes the boot-owned ranges from a validated BootInfo and
//! reserves them on any allocator implementing FrameReserver, all-or-
//! nothing: every range is preflighted before any bitmap state changes, so
//! a failed reservation leaves the allocator untouched.
//!
//! Explicit scope: the RSDP descriptor carries no length, so exactly the
//! page containing it is reserved. The device-tree pointer also carries no
//! length in the v1 boot protocol and is not reserved here. This module
//! does not build the direct map, does not switch CR3 and does not
//! instantiate the live boot allocator.

use core::mem::{align_of, size_of};

use awe_boot_protocol::validate;

use super::frame::{BitmapFrameAllocator, BitmapFrameError, PAGE_SIZE};
use awe_boot_protocol::{BootInfo, MemoryRegion};

/// An allocator that can permanently reserve a physical byte range.
pub trait FrameReserver {
    /// Validate a reservation request without mutating any state.
    fn check_reserve_range(&self, start: u64, length: u64) -> Result<(), BitmapFrameError>;
    /// Reserve every 4 KiB frame intersecting [start, start + length).
    fn reserve_range(&mut self, start: u64, length: u64) -> Result<(), BitmapFrameError>;
}

impl<const N: usize> FrameReserver for BitmapFrameAllocator<N> {
    fn check_reserve_range(&self, start: u64, length: u64) -> Result<(), BitmapFrameError> {
        BitmapFrameAllocator::<N>::check_reserve_range(self, start, length)
    }

    fn reserve_range(&mut self, start: u64, length: u64) -> Result<(), BitmapFrameError> {
        BitmapFrameAllocator::<N>::reserve_range(self, start, length)
    }
}

/// A boot-owned physical byte range to reserve from the allocator.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OwnedRange {
    pub start: u64,
    pub length: u64,
}

/// Upper bound on the boot-owned ranges derived from a v1 BootInfo:
/// kernel image, BootInfo structure, memory-map array, framebuffer, RSDP page.
pub const MAX_BOOT_OWNED_RANGES: usize = 5;

/// The fixed set of boot-owned ranges derived from a BootInfo.
///
/// Derives equality and formatting so the derivation tests can compare the
/// whole Result directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BootOwnedRanges {
    pub ranges: [Option<OwnedRange>; MAX_BOOT_OWNED_RANGES],
    pub count: usize,
}

impl BootOwnedRanges {
    /// Iterate the derived ranges in deterministic order.
    pub fn iter(&self) -> impl Iterator<Item = OwnedRange> + '_ {
        self.ranges.iter().flatten().copied()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BootReserveError {
    /// The BootInfo failed the boot-protocol validation.
    InvalidBootInfo,
    /// The kernel image range [kernel_base, +kernel_size) overflows.
    KernelImageOverflow,
    /// The BootInfo structure address plus its size overflows.
    BootInfoOverflow,
    /// The memory-map descriptor array extent overflows.
    MemoryMapOverflow,
    /// The memory-map pointer is not aligned for MemoryRegion and cannot be
    /// trusted as a descriptor array.
    MemoryMapMisaligned,
    /// The framebuffer range [address, +size) overflows.
    FramebufferOverflow,
    /// The allocator rejected this range (0-based derivation order).
    Reserve {
        index: usize,
        error: BitmapFrameError,
    },
}

fn checked_range(start: u64, length: u64) -> Option<(u64, u64)> {
    if length == 0 {
        return None;
    }
    let end = start.checked_add(length)?;
    Some((start, end))
}

/// Derive the boot-owned physical ranges from a validated BootInfo.
///
/// Order is deterministic: kernel image, BootInfo structure, memory-map
/// array, framebuffer, RSDP page. Fields this loader did not provide
/// (zero length or zero address) are skipped.
pub fn boot_owned_ranges(info: &BootInfo) -> Result<BootOwnedRanges, BootReserveError> {
    if !validate(info) {
        return Err(BootReserveError::InvalidBootInfo);
    }

    let mut out = BootOwnedRanges {
        ranges: [None; MAX_BOOT_OWNED_RANGES],
        count: 0,
    };

    // Kernel image, as reported by the loader or the kernel linker symbols.
    if info.kernel_size > 0 {
        let (start, length) = checked_range(info.kernel_base, info.kernel_size)
            .ok_or(BootReserveError::KernelImageOverflow)?;
        out.ranges[out.count] = Some(OwnedRange { start, length });
        out.count += 1;
    }

    // The BootInfo structure handed to this kernel instance.
    let info_start = info as *const BootInfo as u64;
    let (start, length) = checked_range(info_start, size_of::<BootInfo>() as u64)
        .ok_or(BootReserveError::BootInfoOverflow)?;
    out.ranges[out.count] = Some(OwnedRange { start, length });
    out.count += 1;

    // Memory-map descriptor array. Misalignment is the same fail-closed
    // condition try_from_boot_info rejects before reading descriptors.
    if info.memory_region_count > 0 && !info.memory_regions.is_null() {
        let address = info.memory_regions as usize;
        if !address.is_multiple_of(align_of::<MemoryRegion>()) {
            return Err(BootReserveError::MemoryMapMisaligned);
        }
        let bytes = (info.memory_region_count as usize)
            .checked_mul(size_of::<MemoryRegion>())
            .ok_or(BootReserveError::MemoryMapOverflow)?;
        let (start, length) = checked_range(address as u64, bytes as u64)
            .ok_or(BootReserveError::MemoryMapOverflow)?;
        out.ranges[out.count] = Some(OwnedRange { start, length });
        out.count += 1;
    }

    // Framebuffer, when the loader configured one.
    if info.framebuffer_address != 0 && info.framebuffer_size > 0 {
        let (start, length) = checked_range(info.framebuffer_address, info.framebuffer_size)
            .ok_or(BootReserveError::FramebufferOverflow)?;
        out.ranges[out.count] = Some(OwnedRange { start, length });
        out.count += 1;
    }

    // ACPI RSDP: only the page containing the signature is known to be
    // boot-owned, so exactly that page is reserved.
    if info.acpi_rsdp != 0 {
        out.ranges[out.count] = Some(OwnedRange {
            start: info.acpi_rsdp & !(PAGE_SIZE - 1),
            length: PAGE_SIZE,
        });
        out.count += 1;
    }

    Ok(out)
}

/// Reserve the boot-owned ranges on the given reserver, all-or-nothing.
///
/// Every range is preflighted before any bitmap state changes. On success
/// the returned value is the number of ranges that were derived and
/// reserved. The RSDP page reservation rounds down to the containing
/// 4 KiB frame through the allocator's outward-rounding contract.
pub fn reserve_boot_owned_ranges<R: FrameReserver>(
    reserver: &mut R,
    info: &BootInfo,
) -> Result<usize, BootReserveError> {
    let owned = boot_owned_ranges(info)?;

    // Preflight every range first: reserve is permanent, so a request that
    // cannot be honored must not leave a partial reservation behind.
    for (index, slot) in owned.ranges.iter().enumerate().take(owned.count) {
        // Slots below count are always filled by boot_owned_ranges.
        let range = slot.expect("boot-owned slot below count is filled");
        reserver
            .check_reserve_range(range.start, range.length)
            .map_err(|error| BootReserveError::Reserve { index, error })?;
    }

    for (index, slot) in owned.ranges.iter().enumerate().take(owned.count) {
        // Slots below count are always filled by boot_owned_ranges.
        let range = slot.expect("boot-owned slot below count is filled");
        reserver
            .reserve_range(range.start, range.length)
            .map_err(|error| BootReserveError::Reserve { index, error })?;
    }

    Ok(owned.count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use awe_boot_protocol::{Architecture, MEMORY_REGION_USABLE, MemoryRegion};

    use crate::memory::frame::Frame;

    fn info_with(regions: &[MemoryRegion], kernel_base: u64, kernel_size: u64) -> BootInfo {
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
            kernel_base,
            kernel_size,
        }
    }

    fn empty_allocator() -> BitmapFrameAllocator<8> {
        BitmapFrameAllocator::<8>::new(0x1000, 32)
    }

    #[test]
    fn derives_kernel_image_map_framebuffer_and_rsdp_ranges_in_order() {
        let regions = [MemoryRegion {
            base: 0x1000,
            length: 0x10000,
            kind: MEMORY_REGION_USABLE,
            reserved: 0,
        }];
        let mut info = info_with(&regions, 0x100_0000, 0x30_0000);
        info.framebuffer_address = 0xfd00_0000;
        info.framebuffer_size = 0x8000;
        info.acpi_rsdp = 0x0009_f123;

        let owned = boot_owned_ranges(&info).unwrap();
        assert_eq!(owned.count, 5);
        assert_eq!(owned.iter().count(), 5);

        let kernel = owned.ranges[0].unwrap();
        assert_eq!(kernel.start, 0x100_0000);
        assert_eq!(kernel.length, 0x30_0000);

        let boot_info_range = owned.ranges[1].unwrap();
        assert_eq!(boot_info_range.start, &info as *const BootInfo as u64);
        assert_eq!(boot_info_range.length, size_of::<BootInfo>() as u64);

        let map = owned.ranges[2].unwrap();
        assert_eq!(map.start, regions.as_ptr() as u64);
        assert_eq!(
            map.length,
            (regions.len() * size_of::<MemoryRegion>()) as u64
        );

        let fb = owned.ranges[3].unwrap();
        assert_eq!(fb.start, 0xfd00_0000);
        assert_eq!(fb.length, 0x8000);

        let rsdp = owned.ranges[4].unwrap();
        assert_eq!(rsdp.start, 0x0009_f000);
        assert_eq!(rsdp.length, PAGE_SIZE);
    }

    #[test]
    fn zero_fields_are_skipped_and_only_boot_info_is_derived() {
        let regions = [MemoryRegion {
            base: 0x1000,
            length: 0x8000,
            kind: MEMORY_REGION_USABLE,
            reserved: 0,
        }];
        let info = info_with(&regions, 0, 0);
        let owned = boot_owned_ranges(&info).unwrap();
        assert_eq!(owned.count, 1);
        assert_eq!(owned.iter().count(), 1);
        let mut allocator = empty_allocator();
        // A host test BootInfo instance lives outside the allocator window,
        // so the all-or-nothing preflight rejects the reservation here. The
        // derivation result itself remains valid.
        assert!(matches!(
            reserve_boot_owned_ranges(&mut allocator, &info),
            Err(BootReserveError::Reserve { index: 0, .. })
        ));
    }

    #[test]
    fn invalid_boot_info_is_rejected_before_any_derivation() {
        let mut info = BootInfo::empty(Architecture::X86_64);
        info.magic = 0;
        assert_eq!(
            boot_owned_ranges(&info),
            Err(BootReserveError::InvalidBootInfo)
        );
    }

    #[test]
    fn kernel_image_overflow_is_rejected() {
        let info = info_with(&[], u64::MAX - 0x800, 0x1000);
        assert_eq!(
            boot_owned_ranges(&info),
            Err(BootReserveError::KernelImageOverflow)
        );
    }

    #[test]
    fn misaligned_memory_map_pointer_is_rejected() {
        let storage = [0u64; 4];
        let misaligned = unsafe { storage.as_ptr().cast::<u8>().add(1).cast::<MemoryRegion>() };
        let mut info = info_with(&[], 0, 0);
        info.memory_regions = misaligned;
        info.memory_region_count = 1;
        assert_eq!(
            boot_owned_ranges(&info),
            Err(BootReserveError::MemoryMapMisaligned)
        );
    }

    #[test]
    fn map_pointer_presence_follows_descriptor_count() {
        let mut info = info_with(&[], 0, 0);
        info.memory_region_count = 1;
        // A descriptor count without a pointer fails boot validation.
        assert_eq!(
            boot_owned_ranges(&info),
            Err(BootReserveError::InvalidBootInfo)
        );
    }

    #[test]
    fn check_reserve_range_never_mutates_state() {
        let mut allocator = empty_allocator();
        // Zero-length, overflow and out-of-bounds are all rejected.
        assert_eq!(
            allocator.check_reserve_range(0x1000, 0),
            Err(BitmapFrameError::EmptyRange)
        );
        assert_eq!(
            allocator.check_reserve_range(u64::MAX - 1, 4),
            Err(BitmapFrameError::RangeOverflow)
        );
        assert_eq!(
            allocator.check_reserve_range(0xfc00_0000, 0x1000),
            Err(BitmapFrameError::RangeOutOfBounds)
        );
        // A valid range is accepted but nothing is reserved by the check.
        assert_eq!(allocator.check_reserve_range(0x1000, 0x2000), Ok(()));
        // The checked range is still free afterwards.
        assert_eq!(allocator.allocate_frame().unwrap().start, 0x1000);
    }

    #[test]
    fn reservation_removes_reserved_frames_from_allocation() {
        let mut allocator = empty_allocator();
        // Burn the two lowest frames so the reservations exercise middle-of-
        // -window frames instead of the first ones.
        assert_eq!(allocator.allocate_frame().unwrap().start, 0x1000);
        assert_eq!(allocator.allocate_frame().unwrap().start, 0x2000);
        // The BootInfo range is not reservable in a host test window; these
        // direct trait calls prove reservable ranges stick.
        allocator.reserve_range(0x3000, 0x2800).unwrap();
        assert_eq!(
            allocator.try_free_frame(Frame { start: 0x3000 }),
            Err(BitmapFrameError::ReservedFrame)
        );
        assert_eq!(
            allocator.try_free_frame(Frame { start: 0x5000 }),
            Err(BitmapFrameError::ReservedFrame)
        );
        // Idempotent re-reservation.
        allocator.reserve_range(0x3000, 0x2800).unwrap();
        // First free frame is above the range.
        assert_eq!(allocator.allocate_frame().unwrap().start, 0x6000);
        // A request covering an allocated frame fails atomically.
        assert_eq!(
            allocator.reserve_range(0x6000, 0x2000),
            Err(BitmapFrameError::FrameAlreadyAllocated)
        );
        // The failed request reserved nothing: 0x7000 is next.
        assert_eq!(allocator.allocate_frame().unwrap().start, 0x7000);
    }
}
