#![no_std]

use awe_boot_protocol::{BootInfo, validate};

use crate::boot_phase::{BootPhase, BootProgress};
use crate::memory::PhysicalFrameAllocator;

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KernelBootStatus {
    Ready = 0,
    InvalidBootInfo = 1,
    UnsupportedArchitecture = 2,
    NoCpu = 3,
    NoUsableMemory = 4,
}

pub struct KernelContext {
    progress: BootProgress,
}

impl Default for KernelContext {
    fn default() -> Self {
        Self::new()
    }
}

impl KernelContext {
    pub const fn new() -> Self {
        Self {
            progress: BootProgress::new(),
        }
    }
    pub const fn phase(&self) -> BootPhase {
        self.progress.phase()
    }
    pub fn advance(&mut self) -> bool {
        self.progress.advance()
    }
}

/// Stable entry contract between AWE Loader and CellKernel.
pub fn kernel_entry(info: &BootInfo) -> KernelBootStatus {
    if !validate(info) {
        return KernelBootStatus::InvalidBootInfo;
    }
    if !info.architecture.is_supported() {
        return KernelBootStatus::UnsupportedArchitecture;
    }
    if info.cpu_count == 0 {
        return KernelBootStatus::NoCpu;
    }
    if info.memory_region_count == 0 || info.memory_regions.is_null() {
        return KernelBootStatus::NoUsableMemory;
    }

    let mut frames = unsafe { PhysicalFrameAllocator::from_boot_info(info) };
    if frames.allocate().is_none() {
        return KernelBootStatus::NoUsableMemory;
    }

    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    {
        use crate::arch::x86_64::gdt::init_gdt;
        use crate::arch::x86_64::interrupts::init_pic;
        use crate::arch::x86_64::serial_write_str;
        use crate::memory::allocator::init_kernel_heap;
        use crate::platform::pit::Pit;

        static mut KERNEL_STACK: [u8; 65536] = [0; 65536];

        let stack_top = core::ptr::addr_of_mut!(KERNEL_STACK) as u64 + 65536;

        use crate::arch::x86_64::gdt::{KERNEL_CODE_SELECTOR, USER_CODE_SELECTOR};
        use crate::arch::x86_64::idt::IDT;
        use crate::arch::x86_64::isr_stubs::init_idt_stubs;
        use crate::syscall::{entry_address, init_msr_syscall, set_kernel_stack_top};

        init_gdt(stack_top);
        serial_write_str("AWEOS: GDT & TSS initialized\r\n");

        set_kernel_stack_top(stack_top);
        unsafe {
            init_msr_syscall(entry_address(), KERNEL_CODE_SELECTOR, USER_CODE_SELECTOR);
        }
        serial_write_str("AWEOS: SYSCALL/SYSRET MSRs initialized\r\n");

        let idt_ptr = core::ptr::addr_of_mut!(IDT);
        let tss_ptr = core::ptr::addr_of_mut!(crate::arch::x86_64::gdt::TSS);
        unsafe {
            init_idt_stubs(&mut *idt_ptr, KERNEL_CODE_SELECTOR);
            // Boot-order contract: the critical #NMI/#DF/#MC vectors get
            // their dedicated IST stacks after the handlers are installed
            // and before the IDT is loaded. Failing closed is the only
            // safe option: continuing without IST would let a fault on a
            // broken kernel stack escalate to a triple fault.
            if crate::arch::x86_64::ist::configure_ist_stacks(&mut *tss_ptr, &mut *idt_ptr).is_err()
            {
                serial_write_str("AWEOS: FATAL: critical IST stack setup failed\r\n");
                crate::arch::x86_64::boot::halt();
            }
            (*idt_ptr).load();
        }
        serial_write_str("AWEOS: IDT & critical IST stacks initialized\r\n");

        init_kernel_heap();
        serial_write_str("AWEOS: Kernel Heap initialized\r\n");

        use crate::drivers::pci;
        let mut pci_out = [None; 16];
        let mut enumerator = pci::Enumerator::new(pci::PortConfigSpace);
        if let Ok(_count) = enumerator.scan_bus(0, &mut pci_out) {
            serial_write_str("AWEOS: PCI Bus 0 enumerated\r\n");
        }

        unsafe {
            init_pic();
        }
        if let Some(pit) = Pit::new(1000) {
            unsafe {
                pit.program();
            }
        }
        serial_write_str("AWEOS: Interrupts & PIC/PIT initialized\r\n");

        // No scheduler exists yet: timer ticks terminate in the ISR stubs.
        // This line previously claimed a running scheduler; it does not.
        serial_write_str("AWEOS: timer programmed (no scheduler yet)\r\n");
        serial_write_str("AWEOS: boot protocol validated\r\n");
        serial_write_str("AWEOS: kernel state = RUNNING\r\n");
        serial_write_str("AWEOS: kernel is alive\r\n");
        // The bootstrap identity map is supervisor-only. Ring 3 must remain
        // disabled until a per-process address space maps only the intended
        // user code/data/stack with USER permissions and keeps kernel pages
        // supervisor-only.
        serial_write_str(
            "AWEOS: Ring 3 deferred: isolated user address spaces are not installed\r\n",
        );
    }

    KernelBootStatus::Ready
}

/// Enter a user-mode bootstrap context.
///
/// # Safety
/// The active page tables must map `user_rip` and the stack containing
/// `user_rsp` as user-accessible, while every kernel mapping remains
/// supervisor-only. The caller must also have installed a valid syscall
/// entry and kernel stack. The current boot path deliberately does not call
/// this helper until a per-process address-space builder exists.
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub unsafe fn enter_userspace(user_rip: u64, user_rsp: u64) {
    use crate::arch::x86_64::gdt::{USER_CODE_SELECTOR, USER_DATA_SELECTOR};

    let user_cs = USER_CODE_SELECTOR as u64;
    let user_ss = USER_DATA_SELECTOR as u64;
    // IF=1, IOPL=0. Ring 3 must use syscalls/IPC for privileged operations.
    let rflags = 0x0202u64;

    unsafe {
        core::arch::asm!(
            "push {0}",
            "push {1}",
            "push {2}",
            "push {3}",
            "push {4}",
            "iretq",
            in(reg) user_ss,
            in(reg) user_rsp,
            in(reg) rflags,
            in(reg) user_cs,
            in(reg) user_rip,
            options(noreturn)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use awe_boot_protocol::{Architecture, BootInfo, MemoryRegion};

    #[test]
    fn accepts_valid_x86_64_handoff_with_memory() {
        let regions = [MemoryRegion {
            base: 0x1000,
            length: 0x10000,
            kind: 1,
            reserved: 0,
        }];
        let info = BootInfo {
            magic: awe_boot_protocol::AWE_BOOT_MAGIC,
            version: awe_boot_protocol::AWE_BOOT_VERSION,
            size: core::mem::size_of::<BootInfo>() as u32,
            architecture: Architecture::X86_64,
            cpu_count: 1,
            memory_regions: regions.as_ptr(),
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
        assert_eq!(kernel_entry(&info), KernelBootStatus::Ready);
    }

    #[test]
    fn rejects_invalid_handoff() {
        let mut info = BootInfo::empty(Architecture::X86_64);
        info.magic = 0;
        assert_eq!(kernel_entry(&info), KernelBootStatus::InvalidBootInfo);
    }

    #[test]
    fn rejects_missing_memory_map() {
        let info = BootInfo::empty(Architecture::X86_64);
        assert_eq!(kernel_entry(&info), KernelBootStatus::NoUsableMemory);
    }
}
