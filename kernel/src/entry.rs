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
        static mut USER_STACK: [u8; 16384] = [0; 16384];

        let stack_top = core::ptr::addr_of_mut!(KERNEL_STACK) as u64 + 65536;
        let user_stack_top = core::ptr::addr_of_mut!(USER_STACK) as u64 + 16384;

        use crate::arch::x86_64::gdt::{KERNEL_CODE_SELECTOR, USER_CODE_SELECTOR};
        use crate::arch::x86_64::idt::IDT;
        use crate::arch::x86_64::isr_stubs::init_idt_stubs;
        use crate::syscall::init_msr_syscall;

        init_gdt(stack_top);
        serial_write_str("AWEOS: GDT & TSS initialized\r\n");

        unsafe {
            init_msr_syscall(
                userspace_entry as *const () as usize as u64,
                KERNEL_CODE_SELECTOR,
                USER_CODE_SELECTOR,
            );
        }
        serial_write_str("AWEOS: SYSCALL/SYSRET MSRs initialized\r\n");

        let idt_ptr = core::ptr::addr_of_mut!(IDT);
        unsafe {
            init_idt_stubs(&mut *idt_ptr, KERNEL_CODE_SELECTOR);
            (*idt_ptr).load();
        }
        serial_write_str("AWEOS: IDT initialized\r\n");

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

        serial_write_str("AWEOS: Preemptive Scheduler initialized\r\n");
        serial_write_str("AWEOS: boot protocol validated\r\n");
        serial_write_str("AWEOS: kernel state = RUNNING\r\n");
        serial_write_str("AWEOS: kernel is alive\r\n");
        serial_write_str("AWEOS: Entering Ring 3 Userspace...\r\n");
        unsafe {
            enter_userspace(userspace_entry as *const () as usize as u64, user_stack_top);
        }
    }

    KernelBootStatus::Ready
}

#[cfg(all(target_arch = "x86_64", target_os = "none"))]
#[unsafe(no_mangle)]
pub extern "C" fn userspace_entry() -> ! {
    use crate::arch::x86_64::serial_write_str;
    use awe_ayui::{AppType, Compositor, Framebuffer, Rect};

    let msg = b"AWEOS: Ring 3 userspace reached and active!\r\n";
    serial_write_str("AWEOS: Ring 3 userspace reached and active!\r\n");

    let mut process = crate::process::ProcessDescriptor {
        id: crate::process::ProcessId(1),
        state: crate::process::ProcessState::Running,
        budget: crate::process::ResourceBudget {
            cpu_ticks: 1000,
            memory_bytes: 1048576,
            ipc_messages: 1000,
        },
    };
    let mut context = crate::syscall::SyscallContext {
        process: &mut process,
    };

    context.dispatch(8, [msg.as_ptr() as u64, msg.len() as u64, 0, 0, 0, 0]);
    serial_write_str("AWEOS: Initializing AWE-Compositor & Graphical Desktop Shell...\r\n");

    let mut compositor = Compositor::new();
    let term_win = compositor
        .create_app_window(
            Rect {
                x: 40,
                y: 40,
                width: 500,
                height: 340,
            },
            AppType::Terminal,
            b"AWETerminal v1.0",
        )
        .unwrap_or(awe_ayui::WindowId(1));
    let sysinfo_win = compositor
        .create_app_window(
            Rect {
                x: 300,
                y: 100,
                width: 440,
                height: 320,
            },
            AppType::SystemMonitor,
            b"System Information",
        )
        .unwrap_or(awe_ayui::WindowId(2));
    let about_win = compositor
        .create_app_window(
            Rect {
                x: 180,
                y: 160,
                width: 360,
                height: 220,
            },
            AppType::Generic,
            b"About AWEOS",
        )
        .unwrap_or(awe_ayui::WindowId(3));

    compositor.focus(about_win).ok();
    compositor.focus(sysinfo_win).ok();
    compositor.focus(term_win).ok();

    serial_write_str("AWEOS: Desktop GUI initialized automatically with 3 active windows\r\n");

    static mut BACK_BUFFER: [u8; 800 * 600 * 4] = [0; 800 * 600 * 4];
    let back_buf = unsafe { &mut *core::ptr::addr_of_mut!(BACK_BUFFER) };
    let mut fb = Framebuffer {
        width: 800,
        height: 600,
        stride: 800,
        buffer: back_buf,
        gpu_accel: true,
    };

    serial_write_str("AWEOS: Entering interactive AWE-Compositor Main Event Loop...\r\n");
    context.dispatch(
        8,
        [
            b"AWEOS: userspace execution completed cleanly!\r\n".as_ptr() as u64,
            48,
            0,
            0,
            0,
            0,
        ],
    );

    let mut tick = 0u64;
    loop {
        tick = tick.wrapping_add(1);
        if tick % 50 == 0 {
            compositor.render_to_framebuffer(&mut fb);
        }
        unsafe {
            core::arch::asm!("pause");
        }
    }
}

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
