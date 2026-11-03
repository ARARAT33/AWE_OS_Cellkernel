#![no_std]

/// CPU state captured by the x86_64 common ISR entry.
///
/// The register/vector/error/RIP/CS/RFLAGS prefix is always present. The
/// trailing RSP/SS words are pushed by hardware only when the interrupt
/// changes privilege level; same-ring handlers must use the accessors below
/// instead of reading those fields as though hardware had populated them.
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
    /// The CPU only pushes RSP/SS when an interrupt changes privilege level.
    /// For a same-ring interrupt, the original RSP is the address immediately
    /// after the three-word hardware return frame (RIP, CS, RFLAGS).
    pub const fn interrupted_from_user(&self) -> bool {
        self.cs & 0x3 != 0
    }

    /// Return the interrupted stack pointer without assuming that the CPU
    /// pushed an RSP field for a same-privilege interrupt.
    ///
    /// IST-backed vectors (NMI, #DF, #MC) switch RSP to the TSS IST stack
    /// before the frame is built, but the CPU still does not push RSP/SS for
    /// a same-privilege interrupt, so the derivation below stays valid: the
    /// same-ring RSP is the address immediately after the three-word hardware
    /// return frame (RIP, CS, RFLAGS) on whichever stack the entry used.
    pub fn interrupted_stack_pointer(&self) -> u64 {
        if self.interrupted_from_user() {
            self.rsp
        } else {
            (self as *const Self as usize + core::mem::offset_of!(Self, rsp)) as u64
        }
    }

    /// The interrupted SS is hardware-saved only after a privilege transition.
    pub const fn interrupted_stack_segment(&self) -> Option<u64> {
        if self.interrupted_from_user() {
            Some(self.ss)
        } else {
            None
        }
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
        for vector in [0, 1, 2, 3, 4, 5, 6, 7, 9, 15, 16, 18, 19, 20, 28, 31, 32, 255] {
            assert!(!is_exception_with_error_code(vector), "vector {vector}");
        }
    }

    #[test]
    fn ring_zero_frame_derives_rsp_from_the_hardware_frame() {
        let mut f = InterruptFrame::empty();
        f.rip = 0x0040_0000;
        f.cs = 0x08;
        f.rflags = 0x202;
        // These fields are not present in a same-ring hardware frame.
        f.rsp = 0x1;
        f.ss = 0xffff;

        let expected_rsp =
            (&f as *const InterruptFrame as usize + core::mem::offset_of!(InterruptFrame, rsp))
                as u64;
        assert_eq!(f.interrupted_stack_pointer(), expected_rsp);
        assert_eq!(f.interrupted_stack_segment(), None);
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
        assert_eq!(f.interrupted_stack_segment(), Some(0x23));
        assert!(f.validate());
    }
}
