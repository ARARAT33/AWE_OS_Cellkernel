#![no_std]

use core::mem::size_of;
use crate::memory::address::is_canonical_x86_64_48;

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct IdtEntry {
    offset_low: u16,
    selector: u16,
    options: u16,
    offset_mid: u16,
    offset_high: u32,
    reserved: u32,
}

impl IdtEntry {
    pub const fn missing() -> Self {
        Self {
            offset_low: 0,
            selector: 0,
            options: 0,
            offset_mid: 0,
            offset_high: 0,
            reserved: 0,
        }
    }
    pub const fn new(handler: u64, selector: u16) -> Self {
        Self {
            offset_low: handler as u16,
            selector,
            options: 0x8e00,
            offset_mid: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            reserved: 0,
        }
    }
    pub const fn is_present(&self) -> bool {
        self.options & 0x8000 != 0
    }
    /// Architectural IST slot (1..=7) encoded in bits 0..2 of the options
    /// word, or 0 when the handler runs on the interrupted stack.
    pub const fn ist_index(&self) -> u8 {
        (self.options & 0x0007) as u8
    }
}

#[repr(C, align(16))]
pub struct Idt {
    entries: [IdtEntry; 256],
}

impl Default for Idt {
    fn default() -> Self {
        Self::new()
    }
}

pub static mut IDT: Idt = Idt::new();

impl Idt {
    pub const fn new() -> Self {
        Self {
            entries: [IdtEntry::missing(); 256],
        }
    }
    pub fn set_handler(&mut self, vector: u8, handler: u64, selector: u16) {
        self.entries[vector as usize] = IdtEntry::new(handler, selector);
    }
    pub fn is_present(&self, vector: u8) -> bool {
        self.entries[vector as usize].is_present()
    }
    /// The IST slot of an installed vector, or None for uninstalled vectors
    /// and handlers that run on the interrupted stack.
    pub fn ist_index(&self, vector: u8) -> Option<u8> {
        let entry = self.entries[vector as usize];
        if !entry.is_present() {
            return None;
        }
        let ist = entry.ist_index();
        if ist == 0 { None } else { Some(ist) }
    }
    /// Point an already-installed vector at an IST slot (1..=7) so the CPU
    /// switches to the matching TSS stack on entry. Returns false without
    /// modifying the entry when the vector has no present handler or the
    /// slot index is out of range.
    pub fn set_ist(&mut self, vector: u8, ist: u8) -> bool {
        if !(1..=7).contains(&ist) {
            return false;
        }
        let entry = &mut self.entries[vector as usize];
        if !entry.is_present() {
            return false;
        }
        entry.options = (entry.options & !0x0007) | ist as u16;
        true
    }

    pub fn install_validated(&mut self, vector: u8, handler: u64, selector: u16) -> bool {
        if handler == 0 || selector == 0 {
            return false;
        }
        if !is_canonical_x86_64_48(handler) {
            return false;
        }
        self.set_handler(vector, handler, selector);
        true
    }

    pub fn timer_installed(&self) -> bool {
        self.is_present(32)
    }

    pub unsafe fn load(&'static self) {
        unsafe {
            let descriptor = Idtr {
                limit: (size_of::<Self>() - 1) as u16,
                base: self as *const _ as u64,
            };
            core::arch::asm!("lidt [{}]", in(reg) &descriptor, options(readonly, nostack, preserves_flags));
        }
    }
}

#[repr(C, packed)]
struct Idtr {
    limit: u16,
    base: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    extern "C" fn test_handler() {}
    #[test]
    fn interrupt_gate_encodes_handler_address() {
        let entry = IdtEntry::new(test_handler as *const () as usize as u64, 0x08);
        assert!(entry.is_present());
    }
    #[test]
    fn idt_starts_empty() {
        let idt = Idt::new();
        assert!(!idt.is_present(0));
        assert!(!idt.is_present(255));
    }
    #[test]
    fn validated_install_rejects_invalid_handler() {
        let mut idt = Idt::new();
        assert!(!idt.install_validated(32, 0, 0x08));
        assert!(!idt.timer_installed());
    }
    #[test]
    fn validated_install_accepts_canonical_handler() {
        let mut idt = Idt::new();
        assert!(idt.install_validated(32, 0x0000_0000_0040_0000, 0x08));
        assert!(idt.timer_installed());
    }
    #[test]
    fn validated_install_rejects_noncanonical_sign_extension_gaps() {
        let mut idt = Idt::new();
        assert!(!idt.install_validated(32, 0x0000_8000_0000_0000, 0x08));
        assert!(!idt.install_validated(32, 0xffff_7fff_ffff_fff0, 0x08));
        assert!(!idt.timer_installed());
    }

    #[test]
    fn validated_install_accepts_high_canonical_handler() {
        let mut idt = Idt::new();
        assert!(idt.install_validated(32, 0xffff_8000_0000_1000, 0x08));
        assert!(idt.timer_installed());
    }

    #[test]
    fn plain_gates_do_not_use_an_ist_slot() {
        let mut idt = Idt::new();
        idt.set_handler(32, test_handler as *const () as usize as u64, 0x08);
        assert_eq!(idt.ist_index(32), None);
    }
    #[test]
    fn set_ist_marks_only_the_requested_vector() {
        let mut idt = Idt::new();
        idt.set_handler(8, test_handler as *const () as usize as u64, 0x08);
        assert!(idt.set_ist(8, 2));
        assert_eq!(idt.ist_index(8), Some(2));
        assert_eq!(idt.entries[8].ist_index(), 2);
        // The gate stays present: IST bits must not clobber the P/type bits.
        assert!(idt.is_present(8));
        // Other vectors are untouched.
        assert_eq!(idt.ist_index(2), None);
    }
    #[test]
    fn set_ist_rejects_missing_handlers_and_bad_slots() {
        let mut idt = Idt::new();
        // Vector without an installed handler must fail closed.
        assert!(!idt.set_ist(2, 1));
        idt.set_handler(2, test_handler as *const () as usize as u64, 0x08);
        assert!(!idt.set_ist(2, 0));
        assert!(!idt.set_ist(2, 8));
        assert_eq!(idt.ist_index(2), None);
        assert!(idt.set_ist(2, 7));
    }
    #[test]
    fn uninstalled_vectors_report_no_ist_slot() {
        let idt = Idt::new();
        for vector in 0..=255u8 {
            assert_eq!(idt.ist_index(vector), None);
        }
    }
}
