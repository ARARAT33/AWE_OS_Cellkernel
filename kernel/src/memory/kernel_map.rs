#![no_std]

//! Higher-half kernel virtual layout contract for CellKernel on x86_64.
//!
//! This module defines, as single typed constants, where the kernel half of a
//! per-process address space will place its mappings once activation lands:
//!
//! - a bounded direct map of all boot-discoverable RAM,
//! - kernel text/data (executed, so it cannot overlap the writable direct map),
//! - the kernel heap,
//! - per-CPU kernel-stack space.
//!
//! It is a contract layer, not an active mapper: nothing here installs page
//! tables, writes CR3, or assumes the bootstrap identity map. Its job is to
//! fail at test/compile time if the planned ranges are misaligned, non-
//! canonical, overlapping, or exceed the direct-map window, so later
//! activation work cannot silently corrupt the user half (PML4 indexes
//! 0..255 stay user-only in the existing builder).

use super::address::{VirtualAddress, is_canonical_x86_64_48};

/// 2 MiB alignment used for every region boundary. Regions are planned at
/// large-page granularity so future huge-page direct-map mappings inherit
/// the layout without breaking it.
pub const REGION_GRANULARITY: u64 = 0x20_0000;

/// The first byte of the kernel half (PML4 index 256).
pub const KERNEL_HALF_BASE: u64 = 0xffff_8000_0000_0000;

/// Base of the bounded physical direct map.
pub const DIRECT_MAP_BASE: u64 = 0xffff_8000_0000_0000;

/// Maximum physical address the direct map is planned to cover (64 TiB).
/// The direct map is bounded by design; boot validation must reject memory
/// maps that exceed it rather than growing the region.
pub const DIRECT_MAP_MAX_PHYSICAL: u64 = 0x0000_4000_0000_0000;

/// Base of kernel text and read-only data.
pub const KERNEL_TEXT_BASE: u64 = 0xffff_c100_0000_0000;

/// Size reserved for kernel text plus read-only data (2 GiB).
pub const KERNEL_TEXT_SIZE: u64 = 0x8000_0000;

/// Base of the kernel heap.
pub const KERNEL_HEAP_BASE: u64 = 0xffff_c180_0000_0000;

/// Size reserved for the kernel heap (16 GiB).
pub const KERNEL_HEAP_SIZE: u64 = 0x4_0000_0000;

/// Base of per-CPU kernel-stack space.
pub const KERNEL_PERCPU_BASE: u64 = 0xffff_c1c0_0000_0000;

/// Size reserved for per-CPU space (4 GiB).
pub const KERNEL_PERCPU_SIZE: u64 = 0x1_0000_0000;

/// The distinct kernel virtual regions, in strict ascending order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KernelRegionKind {
    /// Bounded writable direct map of RAM.
    DirectMap,
    /// Kernel code and read-only data; never writable at runtime.
    KernelText,
    /// Kernel heap.
    KernelHeap,
    /// Per-CPU kernel stacks.
    KernelPerCpu,
}

/// One planned kernel virtual region.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct KernelRegion {
    pub kind: KernelRegionKind,
    pub base: u64,
    pub length: u64,
}

const REGION_COUNT: usize = 4;

const fn region_table() -> [KernelRegion; REGION_COUNT] {
    [
        KernelRegion {
            kind: KernelRegionKind::DirectMap,
            base: DIRECT_MAP_BASE,
            length: DIRECT_MAP_MAX_PHYSICAL,
        },
        KernelRegion {
            kind: KernelRegionKind::KernelText,
            base: KERNEL_TEXT_BASE,
            length: KERNEL_TEXT_SIZE,
        },
        KernelRegion {
            kind: KernelRegionKind::KernelHeap,
            base: KERNEL_HEAP_BASE,
            length: KERNEL_HEAP_SIZE,
        },
        KernelRegion {
            kind: KernelRegionKind::KernelPerCpu,
            base: KERNEL_PERCPU_BASE,
            length: KERNEL_PERCPU_SIZE,
        },
    ]
}

/// Errors that invalidate the planned higher-half layout.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LayoutError {
    /// A region violates 2 MiB boundary alignment or has zero length.
    Misaligned,
    /// A region is not inside the canonical kernel half.
    NotKernelHalf,
    /// A region is not canonical under 48-bit four-level paging.
    NonCanonical,
    /// A region range overflowed its end address.
    Overflow,
    /// Two planned regions overlap.
    Overlap,
    /// Regions are not in strict ascending order.
    Unordered,
}

/// End address of a planned region, or overflow error.
pub const fn region_end(region: &KernelRegion) -> Result<u64, LayoutError> {
    match region.base.checked_add(region.length) {
        Some(end) => Ok(end),
        None => Err(LayoutError::Overflow),
    }
}

/// Validate one planned region against the hard rules.
pub const fn validate_region(region: &KernelRegion) -> Result<(), LayoutError> {
    if region.base & (REGION_GRANULARITY - 1) != 0
        || region.length == 0
        || region.length & (REGION_GRANULARITY - 1) != 0
    {
        return Err(LayoutError::Misaligned);
    }
    if region.base < KERNEL_HALF_BASE {
        return Err(LayoutError::NotKernelHalf);
    }
    let end = match region_end(region) {
        Ok(end) => end,
        Err(e) => return Err(e),
    };
    if !is_canonical_x86_64_48(region.base) || !is_canonical_x86_64_48(end - 1) {
        return Err(LayoutError::NonCanonical);
    }
    Ok(())
}

/// Const validation of the whole higher-half plan. The compile-time
/// assertion below and host tests share this exact validation.
pub const fn validate_layout() -> Result<(), LayoutError> {
    let regions = region_table();
    let mut i = 0;
    while i < REGION_COUNT {
        match validate_region(&regions[i]) {
            Ok(()) => {}
            Err(e) => return Err(e),
        }
        let end = match region_end(&regions[i]) {
            Ok(end) => end,
            Err(e) => return Err(e),
        };
        if i + 1 < REGION_COUNT {
            let next = &regions[i + 1];
            if end > next.base {
                return Err(LayoutError::Overlap);
            }
            if next.base <= regions[i].base {
                return Err(LayoutError::Unordered);
            }
        }
        i += 1;
    }
    Ok(())
}

/// Reject an invalid shipped layout during compilation, before any mapper can
/// consume these constants.
const _: () = match validate_layout() {
    Ok(()) => (),
    Err(_) => panic!("invalid CellKernel x86_64 higher-half layout"),
};

/// Translate a physical address into its higher-half direct-map alias.
/// `None` means the physical address exceeds the bounded direct-map window
/// and must never be aliased by an unbounded translation.
pub const fn direct_map_address(physical: u64) -> Option<VirtualAddress> {
    if physical >= DIRECT_MAP_MAX_PHYSICAL {
        return None;
    }
    Some(VirtualAddress(DIRECT_MAP_BASE + physical))
}

/// Which planned kernel region contains a kernel-half virtual address.
pub fn kernel_region_of(virtual_address: u64) -> Option<KernelRegionKind> {
    let a = virtual_address;
    if a < KERNEL_HALF_BASE {
        return None;
    }
    let regions = region_table();
    let mut i = 0;
    while i < REGION_COUNT {
        let r = regions[i];
        let end = match r.base.checked_add(r.length) {
            Some(end) => end,
            None => {
                i += 1;
                continue;
            }
        };
        if a >= r.base && a < end {
            return Some(r.kind);
        }
        i += 1;
    }
    None
}

/// The PML4 index a virtual address would install into (0..512).
/// Kernel-half addresses must all land in indexes 256..511.
pub const fn pml4_index(virtual_address: u64) -> usize {
    ((virtual_address >> 39) & 0x1ff) as usize
}

/// True if the address is inside the canonical kernel half.
pub const fn is_kernel_half(virtual_address: u64) -> bool {
    virtual_address >= KERNEL_HALF_BASE && is_canonical_x86_64_48(virtual_address)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_half_check_rejects_noncanonical_addresses() {
        assert!(is_kernel_half(KERNEL_HALF_BASE));
        assert!(is_kernel_half(u64::MAX));
        assert!(!is_kernel_half(0xffff_0000_0000_0000));
        assert!(!is_kernel_half(0x0000_8000_0000_0000));
        assert!(!is_kernel_half(0x0000_7fff_ffff_ffff));
    }

    #[test]
    fn shipped_layout_is_valid() {
        assert_eq!(validate_layout(), Ok(()));
    }

    #[test]
    fn all_kernel_regions_leave_the_user_half_alone() {
        let regions = region_table();
        let mut i = 0;
        while i < regions.len() {
            let idx = pml4_index(regions[i].base);
            assert!(
                idx >= 256,
                "region {:?} at PML4 index {} would fall into the user half",
                regions[i].kind,
                idx
            );
            i += 1;
        }
    }

    #[test]
    fn direct_map_translation_is_bounded() {
        assert_eq!(
            direct_map_address(0x1000).unwrap().0,
            DIRECT_MAP_BASE + 0x1000
        );
        assert!(direct_map_address(DIRECT_MAP_MAX_PHYSICAL - 0x1000).is_some());
        assert!(direct_map_address(DIRECT_MAP_MAX_PHYSICAL).is_none());
        assert!(direct_map_address(0xffff_0000_0000_0000).is_none());
    }

    #[test]
    fn direct_map_does_not_crowd_the_text_region() {
        let direct_end = DIRECT_MAP_BASE + DIRECT_MAP_MAX_PHYSICAL;
        assert!(direct_end <= KERNEL_TEXT_BASE);
    }

    #[test]
    fn region_lookup_uses_half_open_boundaries() {
        let regions = region_table();
        let mut i = 0;
        while i < regions.len() {
            let region = regions[i];
            let end = region_end(&region).unwrap();

            assert_eq!(kernel_region_of(region.base), Some(region.kind));
            assert_eq!(kernel_region_of(end - 1), Some(region.kind));

            if i + 1 < regions.len() {
                let next = regions[i + 1];
                if end == next.base {
                    assert_eq!(kernel_region_of(end), Some(next.kind));
                } else {
                    assert_eq!(kernel_region_of(end), None);
                }
            } else {
                assert_eq!(kernel_region_of(end), None);
            }

            i += 1;
        }
    }

    #[test]
    fn kernel_regions_use_distinct_kinds() {
        let text = kernel_region_of(KERNEL_TEXT_BASE + 0x1000).unwrap();
        let heap = kernel_region_of(KERNEL_HEAP_BASE + 0x1000).unwrap();
        let percpu = kernel_region_of(KERNEL_PERCPU_BASE + 0x1000).unwrap();
        let direct = kernel_region_of(DIRECT_MAP_BASE + 0x1000).unwrap();
        assert_ne!(text, heap);
        assert_ne!(heap, percpu);
        assert_ne!(direct, text);
        assert_eq!(kernel_region_of(0x1000), None);
        assert_eq!(
            kernel_region_of(KERNEL_PERCPU_BASE + KERNEL_PERCPU_SIZE),
            None
        );
    }

    #[test]
    fn misaligned_region_is_rejected() {
        let region = KernelRegion {
            kind: KernelRegionKind::KernelText,
            base: KERNEL_TEXT_BASE + 0x1000,
            length: REGION_GRANULARITY,
        };
        assert_eq!(validate_region(&region), Err(LayoutError::Misaligned));
    }

    #[test]
    fn user_half_region_is_rejected() {
        let region = KernelRegion {
            kind: KernelRegionKind::KernelText,
            base: 0x0000_1000_0000_0000,
            length: REGION_GRANULARITY,
        };
        assert_eq!(validate_region(&region), Err(LayoutError::NotKernelHalf));
    }

    #[test]
    fn zero_length_region_is_rejected() {
        let region = KernelRegion {
            kind: KernelRegionKind::KernelHeap,
            base: KERNEL_HEAP_BASE,
            length: 0,
        };
        assert_eq!(validate_region(&region), Err(LayoutError::Misaligned));
    }

    #[test]
    fn oversized_direct_map_only_the_pairwise_check_catches_it() {
        // An oversized direct map is aligned and kernel-half valid, so only
        // the pairwise check in validate_layout can reject it against the
        // kernel-text region. Prove the pairwise arithmetic directly.
        let oversized_direct = KernelRegion {
            kind: KernelRegionKind::DirectMap,
            base: DIRECT_MAP_BASE,
            length: KERNEL_TEXT_BASE - DIRECT_MAP_BASE + REGION_GRANULARITY,
        };
        assert_eq!(validate_region(&oversized_direct), Ok(()));
        let direct_end = region_end(&oversized_direct).unwrap();
        assert!(direct_end > KERNEL_TEXT_BASE);
        // The shipped layout must never exhibit that overlap.
        assert_eq!(validate_layout(), Ok(()));
    }
}
