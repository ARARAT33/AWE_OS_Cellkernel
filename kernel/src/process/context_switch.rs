#![no_std]

use super::context::CpuContext;

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
}

impl SwitchFrame {
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

    pub const fn is_well_formed(&self) -> bool {
        self.rip != 0 && self.rsp != 0 && self.rip >> 48 == 0 && self.rsp >> 48 == 0
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SwitchError {
    InvalidCurrent,
    InvalidNext,
    IdenticalTarget,
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
        return Ok(());
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
        let next = frame(0x3000, 0x4000);
        assert!(current.is_well_formed());
        assert!(next.is_well_formed());
    }
}
