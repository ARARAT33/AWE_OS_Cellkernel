#![no_std]

use super::idt::Idt;

pub const TIMER_VECTOR: u8 = 32;

/// Remaps the 8259 PIC so IRQs 0..15 land on IDT vectors 32..47.
pub unsafe fn init_pic() {
    unsafe {
        use super::{io_in8, io_out8};

        // Save masks
        let mask1 = io_in8(0x21);
        let mask2 = io_in8(0xA1);

        // ICW1: Start initialization in cascade mode
        io_out8(0x20, 0x11);
        io_out8(0xA0, 0x11);

        // ICW2: Vector offsets (32 for master, 40 for slave)
        io_out8(0x21, 0x20);
        io_out8(0xA1, 0x28);

        // ICW3: Cascade setup
        io_out8(0x21, 0x04);
        io_out8(0xA1, 0x02);

        // ICW4: 8086 mode
        io_out8(0x21, 0x01);
        io_out8(0xA1, 0x01);

        // Restore masks (or unmask IRQ0 timer & IRQ1 keyboard)
        io_out8(0x21, mask1 & !0x01); // unmask IRQ0 (timer)
        io_out8(0xA1, mask2);
    }
}

pub unsafe fn pic_send_eoi(irq: u8) {
    unsafe {
        use super::io_out8;
        if irq >= 8 {
            io_out8(0xA0, 0x20);
        }
        io_out8(0x20, 0x20);
    }
}

/// Installs the minimal early interrupt surface. The timer handler remains an
/// explicit ABI boundary so scheduler code can be attached without changing
/// IDT construction.
pub fn install_early_interrupts(idt: &mut Idt, code_selector: u16, timer_handler: u64) -> bool {
    idt.set_handler(TIMER_VECTOR, timer_handler, code_selector)
}

#[cfg(test)]
mod tests {
    use super::*;
    extern "C" fn timer() {}

    #[test]
    fn timer_vector_is_installed() {
        let mut idt = Idt::new();
        assert!(install_early_interrupts(
            &mut idt,
            0x08,
            timer as *const () as usize as u64
        ));
        assert!(idt.is_present(TIMER_VECTOR));
        assert!(!idt.is_present(33));
    }

    #[test]
    fn early_interrupt_installation_rejects_noncanonical_handler() {
        let mut idt = Idt::new();
        assert!(!install_early_interrupts(&mut idt, 0x08, 0x0000_8000_0000_0000));
        assert!(!idt.is_present(TIMER_VECTOR));
    }
}
