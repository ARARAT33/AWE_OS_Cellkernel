pub const PAGE_SIZE: usize = 4096;

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct PhysAddr(pub u64);

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct VirtAddr(pub u64);

impl VirtAddr {
    /// 48-bit x86_64 canonical address check for four-level paging.
    #[inline]
    pub const fn is_canonical(self) -> bool {
        let top = self.0 >> 48;
        if self.0 & (1u64 << 47) == 0 {
            top == 0
        } else {
            top == 0xffff
        }
    }

    #[inline]
    pub const fn offset(self) -> usize { (self.0 as usize) & (PAGE_SIZE - 1) }
}

#[repr(u8)]
#[derive(Clone, Copy)]
pub enum PageFlags {
    ReadOnly = 1,
    Writable = 2,
    Executable = 4,
    User = 8,
    Global = 16,
}

/// A tiny bump allocator for the earliest boot phase.
/// It is deliberately temporary: the full physical-frame allocator replaces it later.
pub struct BootAllocator {
    next: usize,
    end: usize,
}

impl BootAllocator {
    pub const fn new(start: usize, end: usize) -> Self { Self { next: start, end } }

    pub fn alloc(&mut self, size: usize, align: usize) -> Option<usize> {
        let mask = align.saturating_sub(1);
        let aligned = (self.next + mask) & !mask;
        let new_end = aligned.checked_add(size)?;
        if new_end > self.end { return None; }
        self.next = new_end;
        Some(aligned)
    }
}
