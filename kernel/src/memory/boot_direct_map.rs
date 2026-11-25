#![no_std]

//! Boot-time direct-map gate for the validated higher-half layout.
//!
//! This module answers one boot question with no allocation and no page
//! tables: does every usable RAM range of the bootloader memory map fit
//! inside the bounded direct-map window (`DIRECT_MAP_MAX_PHYSICAL`, the
//! 64 TiB physical limit behind the higher-half direct map)?
//!
//! The check runs before the physical allocator is constructed, before any
//! table arena is reserved, and before any page-table root is built. A
//! memory map that exceeds the window must fail closed with a distinct
//! boot status instead of silently growing the window or mapping past it.
//!
//! Scope: only the regions the kernel may actually frame-allocate from
//! (`MEMORY_REGION_USABLE`) are gated here. Reserved, reclaimable and
//! MMIO descriptors are not mapped through the direct map and are
//! validated elsewhere. The check never allocates, never dereferences
//! region *contents* (only the descriptor array), and never mutates state.

use core::mem::{align_of, size_of};

use awe_boot_protocol::{BootInfo, MAX_MEMORY_REGIONS, MEMORY_REGION_USABLE, MemoryRegion};

use super::kernel_page_table_build::{DIRECT_MAP_MAX_PHYSICAL, DirectMapFitError};

/// Fail-closed errors for the boot direct-map gate.
///
/// The offending region index is carried by the caller-facing
/// `Result<(usize, _)>` wrapper so boot diagnostics can name the exact
/// descriptor that failed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DirectMapGateError {
    /// The BootInfo carries no memory map at all.
    MissingMemoryMap,
    /// The descriptor array pointer is misaligned for `MemoryRegion`.
    MisalignedMemoryMap,
    /// The descriptor array itself would overflow the address space.
    MapAddressOverflow,
    /// A usable region is empty; it cannot back a direct-map mapping.
    EmptyUsableRegion,
    /// A usable region's base + length overflows.
    RegionOverflow,
    /// A usable range extends past the bounded direct-map window; boot must
    /// fail instead of mapping past it. Carries the overflowing end.
    BeyondDirectMapWindow {
        /// The computed exclusive byte end of the offending range.
        end: u64,
    },
}

/// Outcome of a passing direct-map gate check.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DirectMapBootReport {
    /// Number of `MEMORY_REGION_USABLE` regions that were checked.
    pub usable_ranges: usize,
    /// The highest exclusive physical end observed across usable ranges.
    pub max_physical_end: u64,
}

/// The limiting physical end used by the gate, re-exported for boot
/// diagnostics and tests.
pub const GATE_DIRECT_MAP_LIMIT: u64 = DIRECT_MAP_MAX_PHYSICAL;

/// Validate every usable RAM range of a BootInfo against the bounded
/// direct-map window, with no allocation.
///
/// The first offending usable region is returned by index together with
/// its cause. Regions of other kinds are skipped.
///
/// # Safety (of the caller's BootInfo)
/// `info.memory_regions` must point to a readable, `MemoryRegion`-aligned
/// array of at least `info.memory_region_count` valid descriptors, and the
/// array must remain unchanged for the duration of this call. The pointer
/// is dereferenced only after the pointer, alignment and byte-extent
/// checks below pass, mirroring the contract of
/// `PhysicalFrameAllocator::try_from_boot_info`.
pub fn validate_boot_direct_map(
    info: &BootInfo,
) -> Result<DirectMapBootReport, (usize, DirectMapGateError)> {
    if info.memory_region_count == 0 || info.memory_regions.is_null() {
        return Err((0, DirectMapGateError::MissingMemoryMap));
    }
    if info.memory_region_count > MAX_MEMORY_REGIONS as u32 {
        return Err((
            usize::from(info.memory_region_count),
            DirectMapGateError::MapAddressOverflow,
        ));
    }
    let regions = info.memory_regions;
    let address = regions as usize;
    if !address.is_multiple_of(align_of::<MemoryRegion>()) {
        return Err((0, DirectMapGateError::MisalignedMemoryMap));
    }
    let count = info.memory_region_count as usize;
    let bytes = count
        .checked_mul(size_of::<MemoryRegion>())
        .ok_or((count, DirectMapGateError::MapAddressOverflow))?;
    address
        .checked_add(bytes)
        .ok_or((count, DirectMapGateError::MapAddressOverflow))?;

    let mut report = DirectMapBootReport {
        usable_ranges: 0,
        max_physical_end: 0,
    };
    for index in 0..count {
        // SAFETY: the pointer/alignment/extent checks above guarantee that
        // descriptor `index` is a valid, aligned `MemoryRegion`.
        let region: MemoryRegion = unsafe { regions.add(index).read() };
        if region.kind != MEMORY_REGION_USABLE {
            continue;
        }
        match fits_direct_map_window(region.base, region.length) {
            Ok(()) => {}
            Err(DirectMapFitError::EmptyRange) => {
                return Err((index, DirectMapGateError::EmptyUsableRegion));
            }
            Err(DirectMapFitError::Overflow) => {
                return Err((index, DirectMapGateError::RegionOverflow));
            }
            Err(DirectMapFitError::BeyondWindow { end }) => {
                return Err((index, DirectMapGateError::BeyondDirectMapWindow { end }));
            }
        }
        let end = region
            .base
            .checked_add(region.length)
            .ok_or((index, DirectMapGateError::RegionOverflow))?;
        if end > report.max_physical_end {
            report.max_physical_end = end;
        }
        report.usable_ranges += 1;
    }
    Ok(report)
}

// A compile-time guard that the gate really uses the shipped 64 TiB window
// bound, so a layout change cannot silently pass it.
const _: () = assert!(DIRECT_MAP_MAX_PHYSICAL <= 64 * 1024 * 1024 * 1024 * 1024);

#[cfg(test)]
mod tests {
    use super::*;
    use awe_boot_protocol::{
        AWE_BOOT_MAGIC, AWE_BOOT_VERSION, Architecture, MEMORY_REGION_MMIO,
        MEMORY_REGION_RESERVED, MemoryRegion,
    };

    fn region(base: u64, length: u64, kind: u8) -> MemoryRegion {
        MemoryRegion {
            base,
            length,
            kind,
            reserved: 0,
        }
    }

    fn boot_info(regions: &[MemoryRegion]) -> BootInfo {
        BootInfo {
            magic: AWE_BOOT_MAGIC,
            version: AWE_BOOT_VERSION,
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
    fn in_window_usable_ranges_pass_with_a_report() {
        let regions = [
            region(0, 0x30_0000, MEMORY_REGION_USABLE),
            region(0x50_0000, 0x31_0000, MEMORY_REGION_USABLE),
            region(0x90_0000, 0x10_0000, MEMORY_REGION_RESERVED),
        ];
        let report = validate_boot_direct_map(&boot_info(&regions)).expect("in-window map");
        assert_eq!(report.usable_ranges, 2);
        assert_eq!(report.max_physical_end, 0x50_0000 + 0x31_0000);
    }

    #[test]
    fn usable_range_exactly_at_the_window_boundary_passes() {
        let regions = [region(
            DIRECT_MAP_MAX_PHYSICAL - 0x2000,
            0x2000,
            MEMORY_REGION_USABLE,
        )];
        let report = validate_boot_direct_map(&boot_info(&regions)).expect("exact boundary fits");
        assert_eq!(report.max_physical_end, DIRECT_MAP_MAX_PHYSICAL);
    }

    #[test]
    fn usable_range_straddling_the_window_boundary_fails_named() {
        let regions = [
            region(0, 0x30_0000, MEMORY_REGION_USABLE),
            region(
                DIRECT_MAP_MAX_PHYSICAL - 0x1000,
                0x3000,
                MEMORY_REGION_USABLE,
            ),
        ];
        let (index, error) =
            validate_boot_direct_map(&boot_info(&regions)).expect_err("straddles window");
        assert_eq!(index, 1);
        assert_eq!(
            error,
            DirectMapGateError::BeyondDirectMapWindow {
                end: DIRECT_MAP_MAX_PHYSICAL + 0x2000
            }
        );
    }

    #[test]
    fn non_usable_ranges_beyond_the_window_are_not_gated() {
        let regions = [
            region(0, 0x30_0000, MEMORY_REGION_USABLE),
            region(
                DIRECT_MAP_MAX_PHYSICAL + 0x1000,
                0x10_0000,
                MEMORY_REGION_MMIO,
            ),
        ];
        let report = validate_boot_direct_map(&boot_info(&regions)).expect("mmio not gated");
        assert_eq!(report.usable_ranges, 1);
    }

    #[test]
    fn empty_usable_region_fails_named() {
        let regions = [
            region(0, 0x30_0000, MEMORY_REGION_USABLE),
            region(0x50_0000, 0, MEMORY_REGION_USABLE),
        ];
        let (index, error) = validate_boot_direct_map(&boot_info(&regions)).expect_err("empty");
        assert_eq!(index, 1);
        assert_eq!(error, DirectMapGateError::EmptyUsableRegion);
    }

    #[test]
    fn usable_region_overflow_fails_named() {
        let regions = [region(u64::MAX - 0xfff, 0x2000, MEMORY_REGION_USABLE)];
        let (index, error) = validate_boot_direct_map(&boot_info(&regions)).expect_err("overflow");
        assert_eq!(index, 0);
        assert_eq!(error, DirectMapGateError::RegionOverflow);
    }

    #[test]
    fn missing_memory_map_fails() {
        let info = BootInfo {
            magic: AWE_BOOT_MAGIC,
            version: AWE_BOOT_VERSION,
            size: core::mem::size_of::<BootInfo>() as u32,
            architecture: Architecture::X86_64,
            cpu_count: 1,
            memory_regions: core::ptr::null(),
            memory_region_count: 0,
            framebuffer_address: 0,
            framebuffer_size: 0,
            framebuffer_width: 0,
            framebuffer_height: 0,
            framebuffer_pitch: 0,
            acpi_rsdp: 0,
            device_tree: 0,
            kernel_base: 0,
            kernel_size: 0,
        };
        assert_eq!(
            validate_boot_direct_map(&info),
            Err((0, DirectMapGateError::MissingMemoryMap))
        );
    }

    #[test]
    fn misaligned_region_pointer_fails_before_any_read() {
        let storage = [0u64; 4];
        let misaligned = unsafe { storage.as_ptr().cast::<u8>().add(1).cast::<MemoryRegion>() };
        let info = BootInfo {
            magic: AWE_BOOT_MAGIC,
            version: AWE_BOOT_VERSION,
            size: core::mem::size_of::<BootInfo>() as u32,
            architecture: Architecture::X86_64,
            cpu_count: 1,
            memory_regions: misaligned,
            memory_region_count: 1,
            framebuffer_address: 0,
            framebuffer_size: 0,
            framebuffer_width: 0,
            framebuffer_height: 0,
            framebuffer_pitch: 0,
            acpi_rsdp: 0,
            device_tree: 0,
            kernel_base: 0,
            kernel_size: 0,
        };
        assert_eq!(
            validate_boot_direct_map(&info),
            Err((0, DirectMapGateError::MisalignedMemoryMap))
        );
    }
}
