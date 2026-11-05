#![no_std]

use awe_boot_protocol::{Architecture, BootInfo, MemoryRegion, AWE_BOOT_MAGIC, AWE_BOOT_VERSION};

#[repr(C)]
pub struct EfiTableHeader {
    pub signature: u64,
    pub revision: u32,
    pub header_size: u32,
    pub crc32: u32,
    pub reserved: u32,
}

#[repr(C)]
pub struct EfiSystemTable {
    pub hdr: EfiTableHeader,
    pub firmware_vendor: *const u16,
    pub firmware_revision: u32,
    pub console_in_handle: usize,
    pub con_in: usize,
    pub console_out_handle: usize,
    pub con_out: usize,
    pub standard_error_handle: usize,
    pub std_err: usize,
    pub runtime_services: usize,
    pub boot_services: *const EfiBootServices,
    pub number_of_table_entries: usize,
    pub configuration_table: usize,
}

#[repr(C)]
pub struct EfiBootServices {
    pub hdr: EfiTableHeader,
    pub raise_tpl: usize,
    pub restore_tpl: usize,
    pub allocate_pages: usize,
    pub free_pages: usize,
    pub get_memory_map: usize,
    pub allocate_pool: usize,
    pub free_pool: usize,
    pub create_event: usize,
    pub set_timer: usize,
    pub wait_for_event: usize,
    pub signal_event: usize,
    pub close_event: usize,
    pub check_event: usize,
    pub install_protocol_interface: usize,
    pub reinstall_protocol_interface: usize,
    pub uninstall_protocol_interface: usize,
    pub handle_protocol: usize,
    pub reserved: usize,
    pub register_protocol_notify: usize,
    pub locate_handle: usize,
    pub locate_device_path: usize,
    pub install_configuration_table: usize,
    pub load_image: usize,
    pub start_image: usize,
    pub exit: usize,
    pub unload_image: usize,
    pub exit_boot_services: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct UefiMemoryDescriptor {
    pub typ: u32,
    pub pad: u32,
    pub physical_start: u64,
    pub virtual_start: u64,
    pub number_of_pages: u64,
    pub attribute: u64,
}

pub const EFI_CONVENTIONAL_MEMORY: u32 = 7;

/// Converts a UEFI memory descriptor into the AWEOS normalized memory-region form.
pub fn descriptor_to_region(d: &UefiMemoryDescriptor) -> Option<MemoryRegion> {
    if d.typ != EFI_CONVENTIONAL_MEMORY || d.number_of_pages == 0 {
        return None;
    }
    let length = d.number_of_pages.checked_mul(4096)?;
    d.physical_start.checked_add(length)?;
    Some(MemoryRegion {
        base: d.physical_start,
        length,
        kind: 1,
        reserved: 0,
    })
}

pub const fn initial_boot_info() -> BootInfo {
    BootInfo {
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
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(physical_start: u64, number_of_pages: u64) -> UefiMemoryDescriptor {
        UefiMemoryDescriptor {
            typ: EFI_CONVENTIONAL_MEMORY,
            pad: 0,
            physical_start,
            virtual_start: 0,
            number_of_pages,
            attribute: 0,
        }
    }

    #[test]
    fn conventional_memory_descriptor_is_normalized() {
        let region = descriptor_to_region(&descriptor(0x1000, 4)).unwrap();
        assert_eq!(region.base, 0x1000);
        assert_eq!(region.length, 0x4000);
        assert_eq!(region.kind, 1);
    }

    #[test]
    fn zero_page_and_overflowing_descriptors_are_rejected() {
        assert!(descriptor_to_region(&descriptor(0x1000, 0)).is_none());
        assert!(descriptor_to_region(&descriptor(u64::MAX - 0xfff, 2)).is_none());
    }
}
