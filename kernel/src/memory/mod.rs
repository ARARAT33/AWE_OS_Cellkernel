#![no_std]

pub mod address;
pub mod allocator;
pub mod boot_mapper;
pub mod frame;
pub mod mapper;
pub mod paging;
pub mod physical;
pub mod region;
#[cfg(target_arch = "x86_64")]
pub mod x86_64;

pub use address::{PhysicalAddress, VirtualAddress};
pub use allocator::BumpAllocator;
pub use boot_mapper::{BootMapError, BootMapper};
pub use frame::{
    BitmapFrameAllocator, BitmapFrameError, Frame, FrameAllocatorError, PAGE_SIZE,
    PhysicalFrameAllocator, checked_frame_end, frame_at, frame_end,
};
pub use mapper::{MapError, PageMapper};
pub use paging::{
    PageFlags, PageTable, PageTableEntry, indices as page_indices, page_offset, valid_mapping,
};
pub use physical::{FrameBitmap, PhysicalMemoryError};
pub use region::{Region, RegionKind};
#[cfg(target_arch = "x86_64")]
pub use x86_64::activate_bootstrap_identity_map;
