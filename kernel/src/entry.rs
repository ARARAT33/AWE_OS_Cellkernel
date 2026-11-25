#![no_std]

use awe_boot_protocol::{BootInfo, validate};

use crate::boot_phase::{BootPhase, BootProgress};
use crate::memory::{PhysicalFrameAllocator, reserve_boot_owned_ranges};

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KernelBootStatus {
    Ready = 0,
    InvalidBootInfo = 1,
    UnsupportedArchitecture = 2,
    NoCpu = 3,
    NoUsableMemory = 4,
    InvalidMemoryMap = 5,
    BootReservationFailed = 6,
    /// A usable RAM range of the boot memory map does not fit inside the
    /// bounded direct-map window. Boot fails closed before any allocator or
    /// page-table state is created; see `memory::boot_direct_map`.
    DirectMapWindowExceeded = 7,
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

    // No-allocation direct-map gate: every usable RAM range must fit the
    // 64 TiB bounded direct-map window before any allocator or page-table
    // state exists. A memory map beyond the window fails closed.
    if crate::memory::validate_boot_direct_map(info).is_err() {
        return KernelBootStatus::DirectMapWindowExceeded;
    }

    let mut frames = match unsafe { PhysicalFrameAllocator::try_from_boot_info(info) } {
        Ok(allocator) => allocator,
        Err(_) => return KernelBootStatus::InvalidMemoryMap,
    };
    let _reserved_range_count = match reserve_boot_owned_ranges(&mut frames, info) {
        Ok(count) => count,
        Err(_) => return KernelBootStatus::BootReservationFailed,
    };

    // QEMU evidence probe (kernel-selftest builds only): reserve a small
    // table-frame arena through the boot reservation ledger and prove that
    // a real PhysicalTablePool allocates, zeroes, maps, releases and
    // reclaims page-table frames over it. Production builds do not run it.
    #[cfg(all(
        feature = "kernel-selftest",
        target_arch = "x86_64",
        target_os = "none"
    ))]
    crate::memory::table_arena::boot_table_pool_probe(&mut frames);

    // QEMU evidence probe (kernel-selftest builds only): through the same
    // proven arena path, build a real supervisor-only kernel-half root via
    // the merged KernelPageTableBuilder and validate it by readback. The
    // root is never published and never written to CR3.
    #[cfg(all(
        feature = "kernel-selftest",
        target_arch = "x86_64",
        target_os = "none"
    ))]
    crate::memory::table_arena::boot_kernel_root_probe(&mut frames);

    if frames.allocate().is_none() {
        return KernelBootStatus::NoUsableMemory;
    }

    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    {
        use crate::arch::x86_64::gdt::init_gdt;
        use crate::arch::x86_64::interrupts::init_pic;
        use crate::arch::x86_64::{serial_write_str, serial_write_u64};
        use crate::memory::allocator::init_kernel_heap;
        use crate::platform::pit::Pit;

        serial_write_str("AWEOS: boot-owned physical ranges reserved = ");
        serial_write_u64(_reserved_range_count as u64);
        serial_write_str("\r\n");

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
            if !init_idt_stubs(&mut *idt_ptr, KERNEL_CODE_SELECTOR) {
                serial_write_str("AWEOS: FATAL: invalid IDT handler address\r\n");
                crate::arch::x86_64::boot::halt();
            }
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

        // Bring up the preemptive kernel-thread scheduler. These are ring-0
        // threads sharing the kernel address space; per-process address
        // spaces and ring-3 execution remain separate readiness gates.
        use crate::scheduler::preemptive;
        preemptive::register_boot_context();
        let heartbeat = preemptive::spawn(heartbeat_service_thread);
        let diagnostics = preemptive::spawn(diagnostics_service_thread);
        if heartbeat.is_some() && diagnostics.is_some() {
            preemptive::start(preemptive::DEFAULT_QUANTUM_TICKS);
            serial_write_str(
                "AWEOS: preemptive scheduler armed: 3 kernel threads, quantum 50ms\r\n",
            );
        } else {
            serial_write_str("AWEOS: WARN: scheduler not armed: thread table full\r\n");
        }
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

        // QEMU exception-injection validation (cargo feature
        // "kernel-selftest"): deliberately inject #DE/#UD/#PF/#DF and wait
        // for an externally injected NMI to prove the real IDT handlers,
        // the long-mode frame model and the dedicated IST stacks recover
        // execution. Runs only in validation builds; production images
        // contain no self-test code path.
        #[cfg(feature = "kernel-selftest")]
        {
            crate::arch::x86_64::selftest::run();
        }
    }

    KernelBootStatus::Ready
}

/// Heartbeat kernel service thread: proves that a ring-0 thread other than
/// the boot context runs, survives preemption and observes scheduler time.
/// Prints once per second (1000 PIT ticks).
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
extern "C" fn heartbeat_service_thread() -> ! {
    use crate::arch::x86_64::{serial_write_str, serial_write_u64};
    use crate::scheduler::preemptive;
    let mut beat = 0u64;
    loop {
        beat += 1;
        // Mask interrupts while printing so concurrent thread output cannot
        // interleave inside a single line.
        preemptive::interrupt_free(|| {
            serial_write_str("AWEOS: [cell:heartbeat] beat ");
            serial_write_u64(beat);
            serial_write_str(" @ tick ");
            serial_write_u64(preemptive::ticks());
            serial_write_str("\r\n");
        });
        preemptive::sleep_ticks(1000);
    }
}

/// Diagnostics kernel service thread: reports preemption counters every two
/// seconds so QEMU smoke runs can verify real context switches.
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
extern "C" fn diagnostics_service_thread() -> ! {
    use crate::arch::x86_64::{serial_write_str, serial_write_u64};
    use crate::scheduler::preemptive;
    loop {
        preemptive::sleep_ticks(2000);
        preemptive::interrupt_free(|| {
            serial_write_str("AWEOS: [cell:diagnostics] switches=");
            serial_write_u64(preemptive::switch_count());
            serial_write_str(" dispatches(boot/heartbeat/diagnostics)=");
            serial_write_u64(preemptive::dispatch_count(0).unwrap_or(0));
            serial_write_str("/");
            serial_write_u64(preemptive::dispatch_count(1).unwrap_or(0));
            serial_write_str("/");
            serial_write_u64(preemptive::dispatch_count(2).unwrap_or(0));
            serial_write_str("\r\n");
        });
    }
}

/// Enter a user-mode bootstrap context.
///
/// # Safety
/// The active page tables must map user_rip and the stack containing
/// user_rsp as user-accessible, while every kernel mapping remains
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

    #[test]
    fn rejects_memory_map_beyond_the_direct_map_window() {
        let regions = [MemoryRegion {
            base: super::super::memory::GATE_DIRECT_MAP_LIMIT,
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
        assert_eq!(
            kernel_entry(&info),
            KernelBootStatus::DirectMapWindowExceeded
        );
    }

    #[test]
    fn rejects_overlapping_memory_map() {
        let regions = [
            MemoryRegion {
                base: 0x1000,
                length: 0x3000,
                kind: 1,
                reserved: 0,
            },
            MemoryRegion {
                base: 0x3000,
                length: 0x1000,
                kind: 2,
                reserved: 0,
            },
        ];
        let info = BootInfo {
            magic: awe_boot_protocol::AWE_BOOT_MAGIC,
            version: awe_boot_protocol::AWE_BOOT_VERSION,
            size: core::mem::size_of::<BootInfo>() as u32,
            architecture: Architecture::X86_64,
            cpu_count: 1,
            memory_regions: regions.as_ptr(),
            memory_region_count: 2,
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
        assert_eq!(kernel_entry(&info), KernelBootStatus::InvalidMemoryMap);
    }

    #[test]
    fn rejects_overflowing_memory_map_region() {
        let regions = [MemoryRegion {
            base: u64::MAX - 0xfff,
            length: 0x2000,
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
        assert_eq!(
            kernel_entry(&info),
            KernelBootStatus::DirectMapWindowExceeded
        );
    }

    #[test]
    fn rejects_misaligned_memory_map_pointer_before_reading_it() {
        let storage = [0u64; 4];
        let misaligned = unsafe { storage.as_ptr().cast::<u8>().add(1).cast::<MemoryRegion>() };
        let info = BootInfo {
            magic: awe_boot_protocol::AWE_BOOT_MAGIC,
            version: awe_boot_protocol::AWE_BOOT_VERSION,
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
            kernel_entry(&info),
            KernelBootStatus::DirectMapWindowExceeded
        );
    }
}
