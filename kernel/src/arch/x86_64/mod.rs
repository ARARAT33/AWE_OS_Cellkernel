#![no_std]

pub mod boot;
pub mod entry;
pub mod gdt;
pub mod idt;
pub mod interrupts;
pub mod isr;
pub mod isr_stubs;
pub mod ist;
pub mod timer;

pub const PAGE_TABLE_ENTRIES: usize = 512;
pub const PAGE_SIZE: u64 = 4096;

#[inline(always)]
pub unsafe fn read_cr3() -> u64 {
    unsafe {
        let value: u64;
        core::arch::asm!("mov {}, cr3", out(reg) value, options(nomem, nostack, preserves_flags));
        value
    }
}

#[inline(always)]
pub unsafe fn io_out32(port: u16, value: u32) {
    unsafe {
        core::arch::asm!("out dx, eax", in("dx") port, in("eax") value, options(nomem, nostack, preserves_flags));
    }
}

#[inline(always)]
pub unsafe fn io_in32(port: u16) -> u32 {
    unsafe {
        let value: u32;
        core::arch::asm!("in eax, dx", in("dx") port, out("eax") value, options(nomem, nostack, preserves_flags));
        value
    }
}

pub fn serial_write_byte(byte: u8) {
    unsafe {
        while (io_in8(0x3FD) & 0x20) == 0 {}
        io_out8(0x3F8, byte);
    }
}

pub fn serial_write_str(s: &str) {
    for b in s.bytes() {
        serial_write_byte(b);
    }
}

#[inline(always)]
pub unsafe fn write_cr3(value: u64) {
    unsafe {
        core::arch::asm!("mov cr3, {}", in(reg) value, options(nostack, preserves_flags));
    }
}

#[inline(always)]
pub unsafe fn read_msr(msr: u32) -> u64 {
    unsafe {
        let low: u32;
        let high: u32;
        core::arch::asm!("rdmsr", in("ecx") msr, out("eax") low, out("edx") high, options(nomem, nostack, preserves_flags));
        ((high as u64) << 32) | (low as u64)
    }
}

#[inline(always)]
pub unsafe fn write_msr(msr: u32, value: u64) {
    unsafe {
        let low = value as u32;
        let high = (value >> 32) as u32;
        core::arch::asm!("wrmsr", in("ecx") msr, in("eax") low, in("edx") high, options(nomem, nostack, preserves_flags));
    }
}

#[inline(always)]
pub unsafe fn read_rflags() -> u64 {
    unsafe {
        let value: u64;
        core::arch::asm!("pushfq; pop {}", out(reg) value, options(nomem, preserves_flags));
        value
    }
}

#[inline(always)]
pub unsafe fn io_out8(port: u16, value: u8) {
    unsafe {
        core::arch::asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags));
    }
}

#[inline(always)]
pub unsafe fn io_in8(port: u16) -> u8 {
    unsafe {
        let value: u8;
        core::arch::asm!("in al, dx", in("dx") port, out("al") value, options(nomem, nostack, preserves_flags));
        value
    }
}
