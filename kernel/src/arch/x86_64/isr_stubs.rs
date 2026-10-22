#![no_std]
#![allow(bad_asm_style)]

use super::interrupts::pic_send_eoi;
use super::isr::InterruptFrame;
use super::serial_write_str;
use core::arch::global_asm;
use core::sync::atomic::{AtomicU64, Ordering};

static TIMER_IRQ_COUNT: AtomicU64 = AtomicU64::new(0);

global_asm!(
    r#".intel_syntax noprefix
.section .text
.global awe_isr_common
.type awe_isr_common, @function
awe_isr_common:
    push rax
    push rbx
    push rcx
    push rdx
    push rbp
    push rsi
    push rdi
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15

    mov rdi, rsp
    call awe_interrupt_handler

    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rdi
    pop rsi
    pop rbp
    pop rdx
    pop rcx
    pop rbx
    pop rax

    add rsp, 16
    iretq

.macro ISR_NOERR num
.global awe_isr_\num
awe_isr_\num:
    push 0
    push \num
    jmp awe_isr_common
.endm

.macro ISR_ERR num
.global awe_isr_\num
awe_isr_\num:
    push \num
    jmp awe_isr_common
.endm

ISR_NOERR 0
ISR_NOERR 1
ISR_NOERR 2
ISR_NOERR 3
ISR_NOERR 4
ISR_NOERR 5
ISR_NOERR 6
ISR_NOERR 7
ISR_ERR   8
ISR_NOERR 9
ISR_ERR   10
ISR_ERR   11
ISR_ERR   12
ISR_ERR   13
ISR_ERR   14
ISR_NOERR 15
ISR_NOERR 16
ISR_ERR   17
ISR_NOERR 18
ISR_NOERR 19
ISR_NOERR 20
ISR_NOERR 32
ISR_NOERR 33

.att_syntax prefix
"#
);

unsafe extern "C" {
    pub fn awe_isr_0();
    pub fn awe_isr_1();
    pub fn awe_isr_2();
    pub fn awe_isr_3();
    pub fn awe_isr_4();
    pub fn awe_isr_5();
    pub fn awe_isr_6();
    pub fn awe_isr_7();
    pub fn awe_isr_8();
    pub fn awe_isr_9();
    pub fn awe_isr_10();
    pub fn awe_isr_11();
    pub fn awe_isr_12();
    pub fn awe_isr_13();
    pub fn awe_isr_14();
    pub fn awe_isr_15();
    pub fn awe_isr_16();
    pub fn awe_isr_17();
    pub fn awe_isr_18();
    pub fn awe_isr_19();
    pub fn awe_isr_20();
    pub fn awe_isr_32();
    pub fn awe_isr_33();
}

pub fn init_idt_stubs(idt: &mut super::idt::Idt, cs: u16) {
    idt.set_handler(0, awe_isr_0 as *const () as usize as u64, cs);
    idt.set_handler(1, awe_isr_1 as *const () as usize as u64, cs);
    idt.set_handler(2, awe_isr_2 as *const () as usize as u64, cs);
    idt.set_handler(3, awe_isr_3 as *const () as usize as u64, cs);
    idt.set_handler(4, awe_isr_4 as *const () as usize as u64, cs);
    idt.set_handler(5, awe_isr_5 as *const () as usize as u64, cs);
    idt.set_handler(6, awe_isr_6 as *const () as usize as u64, cs);
    idt.set_handler(7, awe_isr_7 as *const () as usize as u64, cs);
    idt.set_handler(8, awe_isr_8 as *const () as usize as u64, cs);
    idt.set_handler(9, awe_isr_9 as *const () as usize as u64, cs);
    idt.set_handler(10, awe_isr_10 as *const () as usize as u64, cs);
    idt.set_handler(11, awe_isr_11 as *const () as usize as u64, cs);
    idt.set_handler(12, awe_isr_12 as *const () as usize as u64, cs);
    idt.set_handler(13, awe_isr_13 as *const () as usize as u64, cs);
    idt.set_handler(14, awe_isr_14 as *const () as usize as u64, cs);
    idt.set_handler(15, awe_isr_15 as *const () as usize as u64, cs);
    idt.set_handler(16, awe_isr_16 as *const () as usize as u64, cs);
    idt.set_handler(17, awe_isr_17 as *const () as usize as u64, cs);
    idt.set_handler(18, awe_isr_18 as *const () as usize as u64, cs);
    idt.set_handler(19, awe_isr_19 as *const () as usize as u64, cs);
    idt.set_handler(20, awe_isr_20 as *const () as usize as u64, cs);
    idt.set_handler(32, awe_isr_32 as *const () as usize as u64, cs);
    idt.set_handler(33, awe_isr_33 as *const () as usize as u64, cs);
}

fn print_u64_hex(mut val: u64) {
    let hex = b"0123456789ABCDEF";
    let mut buf = [b'0'; 18];
    buf[0] = b'0';
    buf[1] = b'x';
    for i in (2..18).rev() {
        buf[i] = hex[(val & 0xF) as usize];
        val >>= 4;
    }
    for b in buf {
        super::serial_write_byte(b);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn awe_interrupt_handler(frame: &mut InterruptFrame) {
    let vector = frame.vector as u8;
    if vector == 32 {
        unsafe {
            pic_send_eoi(0);
        }
        TIMER_IRQ_COUNT.fetch_add(1, Ordering::Relaxed);
        super::timer::interrupt_tick();
    } else if vector == 33 {
        unsafe {
            let _scancode = super::io_in8(0x60);
            pic_send_eoi(1);
        }
    } else {
        serial_write_str("AWEOS EXCEPTION vector=");
        print_u64_hex(frame.vector);
        serial_write_str(" err=");
        print_u64_hex(frame.error_code);
        serial_write_str(" rip=");
        print_u64_hex(frame.rip);
        serial_write_str(" cs=");
        print_u64_hex(frame.cs);
        serial_write_str(" rsp=");
        print_u64_hex(frame.rsp);
        serial_write_str(" ss=");
        print_u64_hex(frame.ss);
        serial_write_str("\r\n");
        loop {
            unsafe {
                core::arch::asm!("hlt");
            }
        }
    }
}

pub fn timer_irq_count() -> u64 {
    TIMER_IRQ_COUNT.load(Ordering::Acquire)
}
