#![no_std]

use crate::memory::address::is_canonical_x86_64_48;

use core::mem::size_of;

#[repr(C, packed)]
pub struct GdtEntry {
    limit_low: u16,
    base_low: u16,
    base_middle: u8,
    access: u8,
    granularity: u8,
    base_high: u8,
}

impl GdtEntry {
    pub const fn null() -> Self {
        Self {
            limit_low: 0,
            base_low: 0,
            base_middle: 0,
            access: 0,
            granularity: 0,
            base_high: 0,
        }
    }

    pub const fn kernel_code() -> Self {
        Self {
            limit_low: 0xffff,
            base_low: 0,
            base_middle: 0,
            access: 0x9a,      // Present, Ring 0, Executable, Readable
            granularity: 0xaf, // Long mode, 4KiB granularity
            base_high: 0,
        }
    }

    pub const fn kernel_data() -> Self {
        Self {
            limit_low: 0xffff,
            base_low: 0,
            base_middle: 0,
            access: 0x92,      // Present, Ring 0, Writable
            granularity: 0xcf, // 32-bit/4KiB granularity
            base_high: 0,
        }
    }

    pub const fn user_data() -> Self {
        Self {
            limit_low: 0xffff,
            base_low: 0,
            base_middle: 0,
            access: 0xf2, // Present, Ring 3, Writable
            granularity: 0xcf,
            base_high: 0,
        }
    }

    pub const fn user_code() -> Self {
        Self {
            limit_low: 0xffff,
            base_low: 0,
            base_middle: 0,
            access: 0xfa,      // Present, Ring 3, Executable, Readable
            granularity: 0xaf, // Long mode
            base_high: 0,
        }
    }
}

#[repr(C, packed)]
pub struct TssEntry {
    length: u16,
    base_low: u16,
    base_middle: u8,
    flags1: u8,
    flags2: u8,
    base_high: u8,
    base_upper: u32,
    reserved: u32,
}

#[repr(C, packed)]
pub struct Tss64 {
    reserved1: u32,
    pub rsp0: u64,
    pub rsp1: u64,
    pub rsp2: u64,
    reserved2: u64,
    pub ist1: u64,
    pub ist2: u64,
    pub ist3: u64,
    pub ist4: u64,
    pub ist5: u64,
    pub ist6: u64,
    pub ist7: u64,
    reserved3: u64,
    reserved4: u16,
    pub iomap_base: u16,
}

impl Default for Tss64 {
    fn default() -> Self {
        Self::new()
    }
}

impl Tss64 {
    pub const fn new() -> Self {
        Self {
            reserved1: 0,
            rsp0: 0,
            rsp1: 0,
            rsp2: 0,
            reserved2: 0,
            ist1: 0,
            ist2: 0,
            ist3: 0,
            ist4: 0,
            ist5: 0,
            ist6: 0,
            ist7: 0,
            reserved3: 0,
            reserved4: 0,
            iomap_base: size_of::<Tss64>() as u16,
        }
    }

    /// Assign a stack top to an Interrupt Stack Table slot.
    ///
    /// The slot index is the architectural IST number (1..=7), matching the
    /// low three bits of an IDT entry's options word. The stack top must be
    /// non-zero and 16-byte aligned because the CPU loads it directly into
    /// RSP on entry.
    pub fn set_ist(&mut self, index: u8, stack_top: u64) -> Result<(), &'static str> {
        if !(1..=7).contains(&index) {
            return Err("IST index must be in 1..=7");
        }
        if stack_top == 0 {
            return Err("IST stack top must be non-zero");
        }
        if stack_top & 0xF != 0 {
            return Err("IST stack top must be 16-byte aligned");
        }
        if !is_canonical_x86_64_48(stack_top) {
            return Err("IST stack top must be canonical");
        }
        match index {
            1 => self.ist1 = stack_top,
            2 => self.ist2 = stack_top,
            3 => self.ist3 = stack_top,
            4 => self.ist4 = stack_top,
            5 => self.ist5 = stack_top,
            6 => self.ist6 = stack_top,
            7 => self.ist7 = stack_top,
            _ => return Err("unreachable IST index"),
        }
        Ok(())
    }

    /// Read back an IST slot's stack top. Unassigned or invalid slots read 0.
    pub fn ist(&self, index: u8) -> u64 {
        match index {
            1 => self.ist1,
            2 => self.ist2,
            3 => self.ist3,
            4 => self.ist4,
            5 => self.ist5,
            6 => self.ist6,
            7 => self.ist7,
            _ => 0,
        }
    }
}

pub const KERNEL_CODE_SELECTOR: u16 = 0x08;
pub const KERNEL_DATA_SELECTOR: u16 = 0x10;
pub const USER_DATA_SELECTOR: u16 = 0x1B; // 0x18 | 3
pub const USER_CODE_SELECTOR: u16 = 0x23; // 0x20 | 3
pub const TSS_SELECTOR: u16 = 0x28;

#[repr(C, align(16))]
pub struct Gdt {
    null: GdtEntry,
    kernel_code: GdtEntry,
    kernel_data: GdtEntry,
    user_data: GdtEntry,
    user_code: GdtEntry,
    tss_low: TssEntry,
    tss_high: u32,
    tss_reserved: u32,
}

pub static mut TSS: Tss64 = Tss64::new();
pub static mut GDT: Gdt = Gdt::new();

impl Default for Gdt {
    fn default() -> Self {
        Self::new()
    }
}

impl Gdt {
    pub const fn new() -> Self {
        Self {
            null: GdtEntry::null(),
            kernel_code: GdtEntry::kernel_code(),
            kernel_data: GdtEntry::kernel_data(),
            user_data: GdtEntry::user_data(),
            user_code: GdtEntry::user_code(),
            tss_low: TssEntry {
                length: 0,
                base_low: 0,
                base_middle: 0,
                flags1: 0,
                flags2: 0,
                base_high: 0,
                base_upper: 0,
                reserved: 0,
            },
            tss_high: 0,
            tss_reserved: 0,
        }
    }

    pub fn set_tss_ptr(&mut self, tss: *const Tss64) {
        let addr = tss as u64;
        let len = (size_of::<Tss64>() - 1) as u16;

        self.tss_low = TssEntry {
            length: len,
            base_low: addr as u16,
            base_middle: (addr >> 16) as u8,
            flags1: 0x89, // Present, 64-bit TSS
            flags2: 0x00,
            base_high: (addr >> 24) as u8,
            base_upper: (addr >> 32) as u32,
            reserved: 0,
        };
    }

    pub unsafe fn load_ptr(&self) {
        let descriptor = GdtDescriptor {
            limit: (size_of::<Self>() - 1) as u16,
            base: self as *const _ as u64,
        };
        unsafe {
            core::arch::asm!("lgdt [{}]", in(reg) &descriptor, options(readonly, nostack, preserves_flags));
            core::arch::asm!(
                "mov ax, {0:x}",
                "mov ds, ax",
                "mov es, ax",
                "mov fs, ax",
                "mov gs, ax",
                "mov ss, ax",
                in(reg) KERNEL_DATA_SELECTOR,
                options(nostack, preserves_flags)
            );
            core::arch::asm!("ltr {0:x}", in(reg) TSS_SELECTOR, options(nostack, preserves_flags));
        }
    }
}

#[repr(C, packed)]
struct GdtDescriptor {
    limit: u16,
    base: u64,
}

pub fn init_gdt(kernel_stack_top: u64) {
    unsafe {
        let tss_ptr = core::ptr::addr_of_mut!(TSS);
        let gdt_ptr = core::ptr::addr_of_mut!(GDT);
        (*tss_ptr).rsp0 = kernel_stack_top;
        (*gdt_ptr).set_tss_ptr(tss_ptr);
        (*gdt_ptr).load_ptr();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_ist_assigns_valid_stack_tops() {
        let mut tss = Tss64::new();
        assert!(tss.set_ist(1, 0x0000_0000_0090_0000).is_ok());
        assert!(tss.set_ist(7, 0x0000_0000_00A0_0000).is_ok());
        assert_eq!(tss.ist(1), 0x0000_0000_0090_0000);
        assert_eq!(tss.ist(7), 0x0000_0000_00A0_0000);
    }

    #[test]
    fn set_ist_rejects_invalid_slots_and_tops() {
        let mut tss = Tss64::new();
        // Slot 0 and slots above the architectural 7 are not IST entries.
        assert!(tss.set_ist(0, 0x9000).is_err());
        assert!(tss.set_ist(8, 0x9000).is_err());
        // The CPU would load the value straight into RSP: zero or misaligned
        // tops must be refused instead of silently corrupting entry.
        assert!(tss.set_ist(1, 0).is_err());
        assert!(tss.set_ist(1, 0x9001).is_err());
        assert!(tss.set_ist(1, 0x0000_8000_0000_0000).is_err());
        assert!(tss.set_ist(1, 0xffff_7fff_ffff_fff0).is_err());
        assert_eq!(tss.ist(1), 0);
    }

    #[test]
    fn unassigned_ist_slots_read_zero() {
        let tss = Tss64::new();
        for index in 1..=7 {
            assert_eq!(tss.ist(index), 0);
        }
        assert_eq!(tss.ist(0), 0);
        assert_eq!(tss.ist(8), 0);
    }
}
