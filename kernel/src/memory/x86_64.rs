#![no_std]

//! Minimal x86_64 paging activation for the first real CellKernel runtime.
//!
//! The bootstrap map identity-maps the first 1 GiB with 2 MiB pages. It keeps
//! the loader/kernel/Multiboot data reachable while the permanent VM subsystem
//! is brought online. It is deliberately not the final userspace VM manager.

use core::arch::asm;

use super::paging::{PageFlags, PageTableEntry};

const ENTRIES: usize = 512;
const HUGE_2M: u64 = 1 << 7;

/// Permissions for non-leaf bootstrap page-table entries. User access is
/// deliberately absent until a real per-process address space is installed.
pub const BOOTSTRAP_TABLE_FLAGS: u64 = PageFlags::PRESENT.0 | PageFlags::WRITABLE.0;

/// Permissions for the 2 MiB bootstrap identity-map leaves.
pub const BOOTSTRAP_LEAF_FLAGS: u64 = BOOTSTRAP_TABLE_FLAGS | HUGE_2M;

#[repr(C, align(4096))]
struct Table([u64; ENTRIES]);

impl Table {
    const fn empty() -> Self {
        Self([0; ENTRIES])
    }
}

static mut PML4: Table = Table::empty();
static mut PDPT: Table = Table::empty();
static mut PD: Table = Table::empty();

/// Prepare and activate the bootstrap identity map.
///
/// # Safety
/// Must execute in x86_64 long mode with interrupts disabled. The first 1 GiB
/// of physical memory must remain identity-addressable during bootstrap.
#[inline(never)]
pub unsafe fn activate_bootstrap_identity_map() {
    unsafe {
        let pml4 = core::ptr::addr_of_mut!(PML4);
        let pdpt = core::ptr::addr_of_mut!(PDPT);
        let pd = core::ptr::addr_of_mut!(PD);

        // Non-leaf entries must not carry PS/HUGE; only the PD leafs do.
        (*pml4).0[0] = (pdpt as u64) | BOOTSTRAP_TABLE_FLAGS;
        (*pdpt).0[0] = (pd as u64) | BOOTSTRAP_TABLE_FLAGS;

        // Identity-map 0..1 GiB with 2 MiB leaf entries.
        let mut index = 0usize;
        while index < ENTRIES {
            let physical = (index as u64) * 0x20_0000;
            (*pd).0[index] = physical | BOOTSTRAP_LEAF_FLAGS;
            index += 1;
        }

        asm!("mov cr3, {0}", in(reg) pml4 as u64, options(nostack, preserves_flags));
    }
}

/// Return the physical address encoded in a page-table entry.
pub const fn bootstrap_entry_address(entry: u64) -> u64 {
    PageTableEntry::new(entry & 0x000f_ffff_ffff_f000, PageFlags::empty()).address()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_megabyte_leafs_cover_one_gib() {
        assert_eq!(ENTRIES as u64 * 0x20_0000, 0x4000_0000);
        assert_eq!(TABLE_FLAGS & HUGE_2M, 0);
        assert_eq!(LEAF_FLAGS & PageFlags::PRESENT.0, PageFlags::PRESENT.0);
        assert_eq!(LEAF_FLAGS & HUGE_2M, HUGE_2M);
    }

    #[test]
    fn bootstrap_identity_map_is_supervisor_only() {
        assert!(BOOTSTRAP_TABLE_FLAGS & PageFlags::PRESENT.0 != 0);
        assert!(BOOTSTRAP_TABLE_FLAGS & PageFlags::WRITABLE.0 != 0);
        assert_eq!(BOOTSTRAP_TABLE_FLAGS & PageFlags::USER.0, 0);
        assert!(BOOTSTRAP_LEAF_FLAGS & HUGE_2M != 0);
        assert_eq!(BOOTSTRAP_LEAF_FLAGS & PageFlags::USER.0, 0);
    }

    #[test]
    fn entry_address_masks_flags() {
        assert_eq!(
            bootstrap_entry_address(0x1234_5000 | LEAF_FLAGS),
            0x1234_5000
        );
    }
}
