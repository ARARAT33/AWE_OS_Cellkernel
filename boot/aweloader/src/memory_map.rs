#![no_std]

use awe_boot_protocol::{
    MEMORY_REGION_MMIO, MEMORY_REGION_RECLAIMABLE, MEMORY_REGION_RESERVED,
    MEMORY_REGION_USABLE, MemoryRegion,
};

pub const USABLE: u32 = MEMORY_REGION_USABLE;
pub const RESERVED: u32 = MEMORY_REGION_RESERVED;
pub const RECLAIMABLE: u32 = MEMORY_REGION_RECLAIMABLE;
pub const MMIO: u32 = MEMORY_REGION_MMIO;

pub fn normalize(base: u64, length: u64, kind: u32) -> Option<MemoryRegion> {
    if length == 0 || !(USABLE..=MMIO).contains(&kind) {
        return None;
    }
    base.checked_add(length)?;
    Some(MemoryRegion {
        base,
        length,
        kind,
        reserved: 0,
    })
}

pub fn overlaps(a: &MemoryRegion, b: &MemoryRegion) -> bool {
    let ae = match a.base.checked_add(a.length) { Some(v) => v, None => return true };
    let be = match b.base.checked_add(b.length) { Some(v) => v, None => return true };
    a.base < be && b.base < ae
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_accepts_known_nonempty_ranges() {
        let region = normalize(0x1000, 0x3000, USABLE).unwrap();
        assert_eq!(region.base, 0x1000);
        assert_eq!(region.length, 0x3000);
        assert_eq!(region.kind, USABLE);
    }

    #[test]
    fn normalize_rejects_empty_overflowing_and_unknown_ranges() {
        assert!(normalize(0x1000, 0, USABLE).is_none());
        assert!(normalize(u64::MAX - 0xfff, 0x2000, USABLE).is_none());
        assert!(normalize(0x1000, 0x1000, 99).is_none());
    }

    #[test]
    fn overlap_check_detects_intersections_and_adjacent_ranges() {
        let a = normalize(0x1000, 0x2000, USABLE).unwrap();
        let overlap = normalize(0x2000, 0x1000, RESERVED).unwrap();
        let adjacent = normalize(0x3000, 0x1000, RESERVED).unwrap();
        assert!(overlaps(&a, &overlap));
        assert!(!overlaps(&a, &adjacent));
    }
}
