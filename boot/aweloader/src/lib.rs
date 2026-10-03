#![no_std]

use awe_boot_protocol::{Architecture, BootInfo, validate};

pub struct LoaderState {
    pub info: BootInfo,
}

impl LoaderState {
    pub const fn new(architecture: Architecture) -> Self {
        Self {
            info: BootInfo::empty(architecture),
        }
    }

    pub fn ready(&self) -> bool {
        validate(&self.info)
    }

    pub fn set_cpu_count(&mut self, count: u32) -> &mut Self {
        self.info.cpu_count = count.max(1);
        self
    }

    pub fn setup_framebuffer(
        &mut self,
        address: u64,
        size: u64,
        width: u32,
        height: u32,
        pitch: u32,
    ) -> &mut Self {
        self.info.framebuffer_address = address;
        self.info.framebuffer_size = size;
        self.info.framebuffer_width = width;
        self.info.framebuffer_height = height;
        self.info.framebuffer_pitch = pitch;
        self
    }

    pub fn set_acpi_rsdp(&mut self, rsdp: u64) -> &mut Self {
        self.info.acpi_rsdp = rsdp;
        self
    }

    pub fn set_kernel_payload(&mut self, base: u64, size: u64) -> &mut Self {
        self.info.kernel_base = base;
        self.info.kernel_size = size;
        self
    }

    pub fn set_device_tree(&mut self, dtb: u64) -> &mut Self {
        self.info.device_tree = dtb;
        self
    }
}

/// Architecture-neutral handoff. Platform entry code must populate BootInfo
/// before transferring control to the kernel.
///
/// # Safety
/// `kernel_entry` must point to a valid AWEOS kernel entry and `info` must be
/// valid for the lifetime required by the kernel.
pub unsafe fn handoff(kernel_entry: extern "C" fn(*const BootInfo) -> !, info: &BootInfo) -> ! {
    kernel_entry(info as *const BootInfo)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_loader_state_builder_and_validation() {
        let mut loader = LoaderState::new(Architecture::X86_64);
        assert!(loader.ready());

        loader
            .set_cpu_count(8)
            .setup_framebuffer(0xE000_0000, 800 * 600 * 4, 800, 600, 3200)
            .set_acpi_rsdp(0x000F_0000)
            .set_kernel_payload(0x0010_0000, 2 * 1024 * 1024);

        assert_eq!(loader.info.cpu_count, 8);
        assert_eq!(loader.info.framebuffer_address, 0xE000_0000);
        assert_eq!(loader.info.framebuffer_width, 800);
        assert_eq!(loader.info.framebuffer_height, 600);
        assert_eq!(loader.info.acpi_rsdp, 0x000F_0000);
        assert_eq!(loader.info.kernel_base, 0x0010_0000);
        assert!(loader.ready());
    }
}
