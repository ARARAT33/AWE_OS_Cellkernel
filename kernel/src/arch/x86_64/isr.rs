#![no_std]

/// CPU state captured by the x86_64 common ISR entry.
///
/// In 64-bit mode, the processor unconditionally saves the interrupted
/// SS:RSP after the return frame, including same-CPL interrupts with no IST
/// stack switch. The structure therefore has one fixed hardware-frame layout
/// for all vectors (with the stub-normalized error-code slot).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct InterruptFrame {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rbp: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
    pub vector: u64,
    pub error_code: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

impl InterruptFrame {
    pub const fn empty() -> Self {
        Self {
            r15: 0,
            r14: 0,
            r13: 0,
            r12: 0,
            r11: 0,
            r10: 0,
            r9: 0,
            r8: 0,
            rdi: 0,
            rsi: 0,
            rbp: 0,
            rdx: 0,
            rcx: 0,
            rbx: 0,
            rax: 0,
            vector: 0,
            error_code: 0,
            rip: 0,
            cs: 0,
            rflags: 0,
            rsp: 0,
            ss: 0,
        }
    }

    pub const fn instruction_pointer_valid(&self) -> bool {
        let upper = self.rip >> 48;
        (upper == 0 || upper == 0xffff) && self.rip != 0
    }

    /// Whether the interrupted code was running outside ring 0.
    ///
    /// This reports the saved CS privilege level; it does not determine
    /// whether the hardware-saved RSP/SS fields are present (they always are
    /// in 64-bit mode).
    pub const fn interrupted_from_user(&self) -> bool {
        self.cs & 0x3 != 0
    }

    /// Return the RSP unconditionally saved by the processor in 64-bit mode.
    ///
    /// Long-mode interrupt/exception frames always contain the interrupted
    /// SS:RSP, even when CPL does not change and the IDT gate has IST=0.
    pub const fn interrupted_stack_pointer(&self) -> u64 {
        self.rsp
    }

    /// Return the SS unconditionally saved by the processor in 64-bit mode.
    pub const fn interrupted_stack_segment(&self) -> u64 {
        self.ss
    }

    pub fn stack_pointer_valid(&self) -> bool {
        let rsp = self.interrupted_stack_pointer();
        let upper = rsp >> 48;
        (upper == 0 || upper == 0xffff) && (rsp & 0x7) == 0
    }

    pub const fn rflags_valid(&self) -> bool {
        // Bit 1 is architecturally fixed to one; VM is not valid for a normal
        // 64-bit kernel return frame.
        self.rflags & 0x2 != 0 && self.rflags & (1 << 17) == 0
    }

    pub fn validate(&self) -> bool {
        self.instruction_pointer_valid() && self.stack_pointer_valid() && self.rflags_valid()
    }
}

/// Architectural x86 exception vectors for which the CPU pushes an
/// error-code word before entering the handler.
pub const fn is_exception_with_error_code(vector: u8) -> bool {
    matches!(vector, 8 | 10 | 11 | 12 | 13 | 14 | 17 | 21 | 29 | 30)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_validation_accepts_canonical_state() {
        let mut f = InterruptFrame::empty();
        f.rip = 0x0000_0000_0040_0000;
        f.rsp = 0x0000_0000_0080_0000;
        f.rflags = 0x202;
        assert!(f.validate());
    }

    #[test]
    fn frame_validation_rejects_bad_state() {
        let mut f = InterruptFrame::empty();
        f.rip = 0x0001_0000_0000_0000;
        f.rsp = 0x1001;
        f.rflags = 0;
        assert!(!f.validate());
    }

    #[test]
    fn error_code_vectors_are_explicit() {
        for vector in [8, 10, 11, 12, 13, 14, 17, 21, 29, 30] {
            assert!(is_exception_with_error_code(vector), "vector {vector}");
        }
        for vector in [
            0, 1, 2, 3, 4, 5, 6, 7, 9, 15, 16, 18, 19, 20, 28, 31, 32, 255,
        ] {
            assert!(!is_exception_with_error_code(vector), "vector {vector}");
        }
    }

    #[test]
    fn ring_zero_frame_uses_hardware_saved_rsp_and_ss() {
        let mut f = InterruptFrame::empty();
        f.vector = 32; // Same-CPL external interrupt, no IST slot.
        f.rip = 0x0040_0000;
        f.cs = 0x08;
        f.rflags = 0x202;
        f.rsp = 0x0080_0000;
        f.ss = 0x10;

        assert_eq!(f.interrupted_stack_pointer(), 0x0080_0000);
        assert_eq!(f.interrupted_stack_segment(), 0x10);
        assert!(f.validate());
    }

    #[test]
    fn ist_same_ring_frame_uses_hardware_saved_rsp_and_ss() {
        let mut f = InterruptFrame::empty();
        f.vector = 8; // Double Fault uses IST2.
        f.rip = 0x0040_0000;
        f.cs = 0x08; // Same privilege level: CPL0.
        f.rflags = 0x202;
        f.rsp = 0x0080_0000;
        f.ss = 0x10;

        assert!(!f.interrupted_from_user());
        assert_eq!(f.interrupted_stack_pointer(), 0x0080_0000);
        assert_eq!(f.interrupted_stack_segment(), 0x10);
        assert!(f.validate());
    }

    #[test]
    fn ring_three_frame_uses_hardware_saved_rsp_and_ss() {
        let mut f = InterruptFrame::empty();
        f.rip = 0x0040_0000;
        f.cs = 0x1b;
        f.rflags = 0x202;
        f.rsp = 0x0080_0000;
        f.ss = 0x23;

        assert_eq!(f.interrupted_stack_pointer(), 0x0080_0000);
        assert_eq!(f.interrupted_stack_segment(), 0x23);
        assert!(f.validate());
    }
}
