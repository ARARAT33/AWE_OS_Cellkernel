#![no_std]

pub mod address;
pub mod address_space;
pub mod allocator;
pub mod boot_mapper;
pub mod boot_reserve;
pub mod frame;
pub mod mapper;
pub mod page_table_build;
pub mod paging;
pub mod physical;
pub mod physical_tables;
pub mod region;
pub mod table_arena;
pub mod table_frames;
pub mod user_copy;
#[cfg(target_arch = "x86_64")]
pub mod x86_64;

pub use address::{PhysicalAddress, VirtualAddress};
pub use address_space::{AddressSpace, UserCopyError, UserMapping, checked_user_range};
pub use allocator::BumpAllocator;
pub use boot_mapper::{BootMapError, BootMapper};
pub use boot_reserve::{
    BootOwnedRanges, BootReserveError, FrameReserver, MAX_BOOT_OWNED_RANGES, OwnedRange,
    boot_owned_ranges, reserve_boot_owned_ranges,
};
pub use frame::{
    BitmapFrameAllocator, BitmapFrameError, Frame, FrameAllocatorError, PAGE_SIZE,
    PhysicalFrameAllocator, checked_frame_end, frame_at, frame_end,
};
pub use mapper::{MapError, PageMapper};
pub use page_table_build::{PageBuildError, TableMemory, UserPageTableBuilder};
pub use paging::{
    PageFlags, PageTable, PageTableEntry, indices as page_indices, page_offset, valid_mapping,
};
pub use physical::{FrameBitmap, PhysicalMemoryError};
pub use physical_tables::{
    DEFAULT_PHYSICAL_TABLE_POOL_CAPACITY, DirectMapError, DirectMapWindow, FramePoolSource,
    PhysicalTablePool,
};
pub use region::{Region, RegionKind};
pub use table_arena::{BOOT_TABLE_ARENA_FRAMES, TableArena, TableArenaError, reserve_table_arena};
pub use table_frames::{
    BuiltTables, DEFAULT_OWNED_CAPACITY, FrameReleaseError, OwnedTableFrames, TableFramePool,
    build_address_space,
};
pub use user_copy::{
    AccessorFault, CopyUserError, MAX_COPY_BYTES, PhysicalAccess, USER_COPY_PAGE_SIZE,
    copy_user_bytes_in, copy_user_bytes_out,
};
#[cfg(target_arch = "x86_64")]
pub use x86_64::activate_bootstrap_identity_map;
