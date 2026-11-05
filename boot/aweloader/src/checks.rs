#![no_std]

use awe_boot_protocol::{BootInfo, MAX_MEMORY_REGIONS, validate};

pub fn boot_info_valid(info: &BootInfo) -> bool {
    validate(info)
        && info.kernel_base.checked_add(info.kernel_size).is_some()
        && info.memory_region_count <= MAX_MEMORY_REGIONS
}

pub fn address_range_valid(base: u64, size: u64) -> bool {
    size != 0 && base.checked_add(size).is_some()
}
