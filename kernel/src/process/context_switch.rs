#![no_std]

use super::context::CpuContext;
use crate::memory::address::is_canonical_x86_64_48;

pub type SwitchCompletionCallback = unsafe extern "C" fn(u64, u64);

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct SwitchCompletion {
    /// Zero disables the callback. Otherwise this is a canonical kernel code address.
    pub callback: u64,
    /// Opaque, canonical kernel pointer passed as the callback's first argument.
    pub context: u64,
    /// Opaque value passed as the callback's second argument.
    pub argument: u64,
}

impl SwitchCompletion {
    pub const fn none() -> Self {
        Self {
            callback: 0,
            context: 0,
            argument: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct SwitchFrame {
    pub rbx: u64,
    pub rbp: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rsp: u64,
    pub rip: u64,
    /// Optional callback invoked by the x86_64 backend after loading this
    /// frame's stack and before jumping to its saved RIP.
    pub completion: SwitchCompletion,
}

impl SwitchFrame {
    pub const fn empty() -> Self {
        Self {
            rbx: 0,
            rbp: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
            rsp: 0,
            rip: 0,
            completion: SwitchCompletion::none(),
        }
    }

    pub const fn from_context(ctx: &CpuContext) -> Self {
        Self {
            rbx: ctx.rbx,
            rbp: ctx.rbp,
            r12: ctx.r12,
            r13: ctx.r13,
            r14: ctx.r14,
            r15: ctx.r15,
            rsp: ctx.rsp,
            rip: ctx.rip,
            completion: SwitchCompletion::none(),
        }
    }

    pub fn apply_to_context(&self, ctx: &mut CpuContext) {
        ctx.rbx = self.rbx;
        ctx.rbp = self.rbp;
        ctx.r12 = self.r12;
        ctx.r13 = self.r13;
        ctx.r14 = self.r14;
        ctx.r15 = self.r15;
        ctx.rsp = self.rsp;
        ctx.rip = self.rip;
    }

    /// Attach a kernel-only completion hook to a target frame.
    ///
    /// # Safety
    /// The callback must be a valid non-unwinding System V AMD64 function.
    /// Its context pointer must remain valid and kernel-accessible until the
    /// target stack has been installed and the callback has returned.
    pub unsafe fn with_completion(
        mut self,
        callback: SwitchCompletionCallback,
        context: u64,
        argument: u64,
    ) -> Result<Self, SwitchError> {
        self.completion = SwitchCompletion {
            callback: callback as usize as u64,
            context,
            argument,
        };
        if self.is_well_formed() {
            Ok(self)
        } else {
            Err(SwitchError::InvalidCompletion)
        }
    }

    pub const fn is_well_formed(&self) -> bool {
        let completion_valid = if self.completion.callback == 0 {
            self.completion.context == 0 && self.completion.argument == 0
        } else {
            is_canonical_x86_64_48(self.completion.callback)
                && self.completion.context != 0
                && is_canonical_x86_64_48(self.completion.context)
                // The assembly pushes the target RIP (plus one pad word when
                // the saved RSP is 16-byte aligned) before calling the hook.
                // An ABI function entry must therefore start with
                // RSP % 16 == 8, which the backend guarantees for both the
                // 16n and 16n+8 saved-RSP cases.
                && matches!(self.rsp & 0xF, 0 | 8)
        };

        self.rip != 0
            && self.rsp != 0
            && is_canonical_x86_64_48(self.rip)
            && is_canonical_x86_64_48(self.rsp)
            && completion_valid
    }
}

// The first eight u64s are consumed directly by x86_64_switch.S. Keep the
// completion triplet at offsets 64/72/80 when changing this kernel-private ABI.
const _: [(); 64] = [(); core::mem::offset_of!(SwitchFrame, completion)];
const _: [(); 24] = [(); core::mem::size_of::<SwitchCompletion>()];
const _: [(); 88] = [(); core::mem::size_of::<SwitchFrame>()];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SwitchError {
    InvalidCurrent,
    InvalidNext,
    IdenticalTarget,
    InvalidCompletion,
    UnsupportedArchitecture,
}

pub fn prepare_switch(
    current: &CpuContext,
    next: &CpuContext,
) -> Result<(SwitchFrame, SwitchFrame), SwitchError> {
    let old = SwitchFrame::from_context(current);
    let new = SwitchFrame::from_context(next);
    if !old.is_well_formed() {
        return Err(SwitchError::InvalidCurrent);
    }
    if !new.is_well_formed() {
        return Err(SwitchError::InvalidNext);
    }
    if old.rip == new.rip && old.rsp == new.rsp {
        return Err(SwitchError::IdenticalTarget);
    }
    Ok((old, new))
}

#[inline(never)]
pub unsafe fn context_switch(
    current: *mut SwitchFrame,
    next: *const SwitchFrame,
) -> Result<(), SwitchError> {
    if current.is_null() {
        return Err(SwitchError::InvalidCurrent);
    }
    if next.is_null() {
        return Err(SwitchError::InvalidNext);
    }

    let current_ref = unsafe { &*current };
    let next_ref = unsafe { &*next };
    if !current_ref.is_well_formed() {
        return Err(SwitchError::InvalidCurrent);
    }
    if !next_ref.is_well_formed() {
        return Err(SwitchError::InvalidNext);
    }
    if current_ref.rip == next_ref.rip && current_ref.rsp == next_ref.rsp {
        return Err(SwitchError::IdenticalTarget);
    }

    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: both frames were validated immediately above. The assembly
        // backend owns only the register save/restore and never dereferences
        // user-controlled pointers beyond the two validated frame addresses.
        unsafe {
            super::x86_64_backend::awe_x86_64_switch(current, next);
        }
        Ok(())
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = current;
        let _ = next;
        Err(SwitchError::UnsupportedArchitecture)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(rip: u64, rsp: u64) -> SwitchFrame {
        SwitchFrame {
            rip,
            rsp,
            ..SwitchFrame::default()
        }
    }

    #[test]
    fn rejects_identical_target() {
        let current = CpuContext::kernel_entry(0x1000, 0x2000, 0);
        let next = current;
        assert_eq!(
            prepare_switch(&current, &next),
            Err(SwitchError::IdenticalTarget)
        );
    }

    #[test]
    fn accepts_distinct_valid_frames() {
        let current = frame(0x1000, 0x2000);
        let next = frame(0xffff_8000_0000_3000, 0xffff_8000_0000_4000);
        assert!(current.is_well_formed());
        assert!(next.is_well_formed());
    }

    #[test]
    fn rejects_noncanonical_sign_extension_gaps() {
        assert!(!frame(0x0000_8000_0000_0000, 0x2000).is_well_formed());
        assert!(!frame(0x1000, 0xffff_7fff_ffff_fff0).is_well_formed());
    }

    #[test]
    fn completion_callback_accepts_both_aligned_entry_stacks() {
        unsafe extern "C" fn completion(_: u64, _: u64) {}

        // A 16-byte-aligned saved RSP: the backend pushes one pad word
        // before the callback so the entry still sees RSP % 16 == 8.
        let aligned = frame(0x400000, 0x800000);
        assert!(unsafe { aligned.with_completion(completion, 0x1000, 7) }.is_ok());

        // A 16n+8 saved RSP: no pad is needed.
        let aligned_eight = frame(0x400000, 0x800008);
        assert!(unsafe { aligned_eight.with_completion(completion, 0x1000, 7) }.is_ok());

        let misaligned = frame(0x400000, 0x800004);
        assert_eq!(
            unsafe { misaligned.with_completion(completion, 0x1000, 7) },
            Err(SwitchError::InvalidCompletion)
        );
    }

    #[test]
    fn disabled_completion_rejects_orphan_metadata() {
        let mut target = frame(0x400000, 0x800000);
        target.completion.context = 0x1000;
        assert!(!target.is_well_formed());
    }
}
