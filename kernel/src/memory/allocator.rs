#![no_std]
use core::alloc::{GlobalAlloc, Layout};
use core::ptr::null_mut;
use core::sync::atomic::{AtomicUsize, Ordering};
/// Lock-free monotonic bump allocator for the earliest kernel heap phase.
pub struct BumpAllocator {
    base: AtomicUsize,
    start: AtomicUsize,
    end: AtomicUsize,
}
impl Default for BumpAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl BumpAllocator {
    pub const fn new() -> Self {
        Self {
            base: AtomicUsize::new(0),
            start: AtomicUsize::new(0),
            end: AtomicUsize::new(0),
        }
    }
    /// # Safety
    /// The region must be exclusively reserved for this allocator and remain valid for its lifetime.
    pub unsafe fn init(&self, start: usize, size: usize) {
        self.base.store(start, Ordering::Release);
        self.start.store(start, Ordering::Release);
        self.end
            .store(start.saturating_add(size), Ordering::Release)
    }
    pub fn bounds(&self) -> (usize, usize) {
        (
            self.start.load(Ordering::Acquire),
            self.end.load(Ordering::Acquire),
        )
    }
    /// Total initialized capacity, independent of allocations already consumed.
    pub fn capacity(&self) -> usize {
        let base = self.base.load(Ordering::Acquire);
        let end = self.end.load(Ordering::Acquire);
        end.saturating_sub(base)
    }
    pub fn remaining(&self) -> usize {
        let (start, end) = self.bounds();
        end.saturating_sub(start)
    }
    fn alloc_inner(&self, layout: Layout) -> *mut u8 {
        if layout.size() == 0 || !layout.align().is_power_of_two() {
            return null_mut();
        }
        let align = layout.align();
        let size = layout.size();
        loop {
            let current = self.start.load(Ordering::Acquire);
            let aligned = match current.checked_add(align - 1) {
                Some(v) => v & !(align - 1),
                None => return null_mut(),
            };
            let next = match aligned.checked_add(size) {
                Some(v) => v,
                None => return null_mut(),
            };
            if next > self.end.load(Ordering::Acquire) {
                return null_mut();
            }
            if self
                .start
                .compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return aligned as *mut u8;
            }
        }
    }
}
unsafe impl GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.alloc_inner(layout)
    }
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}
}

#[cfg(all(not(test), target_os = "none"))]
#[global_allocator]
pub static HEAP_ALLOCATOR: BumpAllocator = BumpAllocator::new();

#[cfg(all(not(test), target_os = "none"))]
static mut HEAP_MEM: [u8; 1024 * 1024] = [0; 1024 * 1024];

pub fn init_kernel_heap() {
    #[cfg(all(not(test), target_os = "none"))]
    unsafe {
        HEAP_ALLOCATOR.init(core::ptr::addr_of_mut!(HEAP_MEM) as usize, 1024 * 1024);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn allocator_respects_alignment_and_bounds() {
        let allocator = BumpAllocator::new();
        #[repr(align(16))]
        struct AlignedStorage([u8; 136]);
        let mut storage = AlignedStorage([0u8; 136]);
        // Start eight bytes into an aligned allocation so the expected
        // alignment padding is deterministic on every test runner.
        let start = storage.0.as_mut_ptr().wrapping_add(8);
        unsafe { allocator.init(start as usize, 128) }
        let a = allocator.alloc_inner(Layout::from_size_align(1, 1).unwrap());
        let b = allocator.alloc_inner(Layout::from_size_align(16, 16).unwrap());
        assert!(!a.is_null());
        assert!(!b.is_null());
        assert_eq!((b as usize) % 16, 0);
        assert_eq!(allocator.capacity(), 128);
        assert_eq!(allocator.remaining(), 104)
    }
    #[test]
    fn zero_sized_allocations_are_rejected() {
        let allocator = BumpAllocator::new();
        let layout = Layout::from_size_align(0, 8).unwrap();
        assert!(allocator.alloc_inner(layout).is_null())
    }
}
