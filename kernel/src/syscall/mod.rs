#![no_std]

pub mod abi;
pub mod dispatch;

pub use abi::{
    ERR_BUSY, ERR_INVALID_ARGUMENT, ERR_NO_MEMORY, ERR_NOT_FOUND, ERR_OK, ERR_PERMISSION, Syscall,
    SyscallResult,
};
pub use dispatch::SyscallContext;

pub const IA32_EFER_MSR: u32 = 0xC000_0080;
pub const IA32_STAR_MSR: u32 = 0xC000_0081;
pub const IA32_LSTAR_MSR: u32 = 0xC000_0082;
pub const IA32_FMASK_MSR: u32 = 0xC000_0084;

/// Reserved only for the early Ring 3-to-kernel round-trip smoke check.
/// It is not part of the public userspace syscall ABI.
pub const BOOTSTRAP_HELLO_SYSCALL: u64 = 0xA0E0;

const fn sysret_selector_base(user_code_selector: u16) -> u16 {
    user_code_selector.saturating_sub(16)
}

#[cfg(all(target_arch = "x86_64", target_os = "none"))]
core::arch::global_asm!(include_str!("entry.S"));

#[cfg(all(target_arch = "x86_64", target_os = "none"))]
#[unsafe(no_mangle)]
pub static mut AWE_SYSCALL_USER_RSP: u64 = 0;

#[cfg(all(target_arch = "x86_64", target_os = "none"))]
#[unsafe(no_mangle)]
pub static mut AWE_SYSCALL_KERNEL_STACK_TOP: u64 = 0;

#[cfg(all(target_arch = "x86_64", target_os = "none"))]
unsafe extern "C" {
    fn awe_syscall_entry();
}

/// Return the address programmed into LSTAR for the bootstrap syscall entry.
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub fn entry_address() -> u64 {
    awe_syscall_entry as *const () as usize as u64
}

/// Configure the kernel stack used by the early single-CPU syscall entry.
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub fn set_kernel_stack_top(stack_top: u64) {
    // SAFETY: called once during single-CPU boot before userspace is entered.
    unsafe {
        AWE_SYSCALL_KERNEL_STACK_TOP = stack_top;
    }
}

/// Minimal bootstrap handler. A real syscall dispatcher with validated user
/// buffers and capability checks must replace this handshake before general
/// userspace applications are enabled.
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
#[unsafe(no_mangle)]
pub extern "C" fn awe_syscall_dispatch_entry(number: u64) -> u64 {
    use crate::arch::x86_64::serial_write_str;

    if number == BOOTSTRAP_HELLO_SYSCALL {
        serial_write_str("AWEOS: Ring 3 userspace reached and active!\r\n");
        serial_write_str("AWEOS: userspace execution completed cleanly!\r\n");
        0
    } else {
        serial_write_str("AWEOS: unsupported bootstrap syscall\r\n");
        ERR_INVALID_ARGUMENT
    }
}

/// Configures x86_64 Model Specific Registers for SYSCALL/SYSRET fast system call entry.
pub unsafe fn init_msr_syscall(syscall_handler_address: u64, kernel_cs: u16, user_cs: u16) {
    let _ = (syscall_handler_address, kernel_cs, user_cs);
    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    unsafe {
        use crate::arch::x86_64::{read_msr, write_msr};
        // Enable SCE (System Call Extension) in EFER.
        let efer = read_msr(IA32_EFER_MSR);
        write_msr(IA32_EFER_MSR, efer | 1);

        // SYSCALL enters with CS=kernel_cs and SS=kernel_cs+8. For SYSRET,
        // STAR stores a base selector 16 below user CS: SYSRET then derives
        // user CS=base+16 and user SS=base+8. This GDT uses CS=0x23, SS=0x1b.
        let sysret_base = sysret_selector_base(user_cs);
        let star = ((sysret_base as u64) << 48) | ((kernel_cs as u64) << 32);
        write_msr(IA32_STAR_MSR, star);

        // LSTAR: trusted kernel syscall entry, not the user entry function.
        write_msr(IA32_LSTAR_MSR, syscall_handler_address);

        // Mask IF while the single-CPU syscall entry is using its kernel stack.
        write_msr(IA32_FMASK_MSR, 0x200);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sysret_selector_base_matches_current_gdt_layout() {
        assert_eq!(sysret_selector_base(0x23), 0x13);
        assert_eq!(sysret_selector_base(0x10), 0);
    }
}
