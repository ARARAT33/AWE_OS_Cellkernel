#![no_std]

//! Checked physical user-copy layer.
//!
//! This module closes the remaining logic gap between
//! [`AddressSpace::check_user_access`] and an actual kernel-side copy: it
//! validates the entire user range first, then performs the copy one
//! page-bounded segment at a time through a caller-supplied
//! [`PhysicalAccess`] backend.
//!
//! The layer itself never dereferences memory. Whether the translated
//! physical addresses are really readable or writable is a property of
//! the backend (for example, a verified direct-map window). A backend
//! that hands out unchecked mappings is a caller error, exactly as for
//! `DirectMapWindow` in `physical_tables.rs`.
//!
//! Explicit non-goals:
//! - this module is not wired into the live boot memory map;
//! - it does not activate address spaces and does not touch CR3;
//! - it proves no Ring-3 isolation;
//! - an `Accessor` fault after validation ends can leave the copy
//!   partially applied, so a caller must treat its destination buffer as
//!   unspecified until this layer returns `Ok(())`.

use super::address_space::{ADDRESS_SPACE_PAGE_SIZE, AddressSpace, UserCopyError};

/// Page size, re-exported from the address-space contract layer for a
/// self-contained user-copy API.
pub const USER_COPY_PAGE_SIZE: u64 = ADDRESS_SPACE_PAGE_SIZE;

/// Hard cap on a single checked copy call, independent of the caller's
/// range arithmetic. Bounds any backend's exposure to a runaway request.
pub const MAX_COPY_BYTES: u64 = 1024 * 1024;

/// Errors of the checked user-copy layer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CopyUserError {
    /// The user-side validation failed. No physical access was attempted.
    Access(UserCopyError),
    /// The requested length exceeds [`MAX_COPY_BYTES`].
    TooLarge,
    /// Physical frame-plus-offset arithmetic overflowed. Fails closed.
    PhysOverflow,
    /// The physical backend rejected a segment access.
    Accessor,
}

/// Failure marker returned by a [`PhysicalAccess`] backend.
///
/// Backends are expected to be total over their declared window and to
/// reject everything else, so this marker deliberately carries no detail:
/// any fault means the caller must stop trusting the transferred bytes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AccessorFault;

/// Bounded physical-memory backend for checked user copies.
///
/// Each call must cover only bytes within one translated page segment;
/// the copy layer guarantees that per-call lengths never exceed the
/// remaining bytes of a single 4 KiB page. The backend is responsible for
/// all safety of the physical access itself.
pub trait PhysicalAccess {
    /// Read `out.len()` bytes at physical address `physical`.
    fn read_bytes(&mut self, physical: u64, out: &mut [u8]) -> Result<(), AccessorFault>;

    /// Write `src.len()` bytes at physical address `physical`.
    fn write_bytes(&mut self, physical: u64, src: &[u8]) -> Result<(), AccessorFault>;
}

/// Validate a whole copy range before any physical access.
///
/// This ordering is the security property of the layer: a range that
/// fails mapping, permission or cap checks never reaches the backend, so
/// a failed validation cannot leave half-transferred state behind.
fn validate_copy<const MAX_MAPPINGS: usize>(
    space: &AddressSpace<MAX_MAPPINGS>,
    user_address: u64,
    len: u64,
    write: bool,
) -> Result<(), CopyUserError> {
    if len > MAX_COPY_BYTES {
        return Err(CopyUserError::TooLarge);
    }
    space
        .check_user_access(user_address, len, write)
        .map_err(CopyUserError::Access)
}

/// Translate one byte's virtual address to its physical address through
/// the registered mappings. The caller must have validated the enclosing
/// range; this function still fails closed on any gap.
fn segment_physical<const MAX_MAPPINGS: usize>(
    space: &AddressSpace<MAX_MAPPINGS>,
    user_address: u64,
) -> Result<u64, CopyUserError> {
    let page = user_address & !(USER_COPY_PAGE_SIZE - 1);
    let offset = user_address - page;
    let physical_page = space
        .translate_page(page)
        .ok_or(CopyUserError::Access(UserCopyError::Unmapped))?;
    physical_page
        .checked_add(offset)
        .ok_or(CopyUserError::PhysOverflow)
}

/// Copy bytes from user memory into a kernel buffer.
///
/// The full [user_address, user_address + destination.len()) range is
/// validated for read access before the first physical access. Each
/// page-crossing segment is then translated separately and transferred
/// through `access`.
pub fn copy_user_bytes_in<const MAX_MAPPINGS: usize, A: PhysicalAccess>(
    space: &AddressSpace<MAX_MAPPINGS>,
    user_address: u64,
    destination: &mut [u8],
    access: &mut A,
) -> Result<(), CopyUserError> {
    validate_copy(space, user_address, destination.len() as u64, false)?;
    let len = destination.len();
    let mut done: usize = 0;
    while done < len {
        let virtual_address = user_address
            .checked_add(done as u64)
            .ok_or(CopyUserError::PhysOverflow)?;
        let physical = segment_physical(space, virtual_address)?;
        let in_page = (virtual_address & (USER_COPY_PAGE_SIZE - 1)) as usize;
        let segment = (USER_COPY_PAGE_SIZE as usize - in_page).min(len - done);
        access
            .read_bytes(physical, &mut destination[done..done + segment])
            .map_err(|_| CopyUserError::Accessor)?;
        done += segment;
    }
    Ok(())
}

/// Copy bytes from a kernel buffer into user memory.
///
/// The full [user_address, user_address + source.len()) range is
/// validated for write access before the first physical access. Each
/// page-crossing segment is then translated separately and transferred
/// through `access`.
pub fn copy_user_bytes_out<const MAX_MAPPINGS: usize, A: PhysicalAccess>(
    space: &AddressSpace<MAX_MAPPINGS>,
    user_address: u64,
    source: &[u8],
    access: &mut A,
) -> Result<(), CopyUserError> {
    validate_copy(space, user_address, source.len() as u64, true)?;
    let len = source.len();
    let mut done: usize = 0;
    while done < len {
        let virtual_address = user_address
            .checked_add(done as u64)
            .ok_or(CopyUserError::PhysOverflow)?;
        let physical = segment_physical(space, virtual_address)?;
        let in_page = (virtual_address & (USER_COPY_PAGE_SIZE - 1)) as usize;
        let segment = (USER_COPY_PAGE_SIZE as usize - in_page).min(len - done);
        access
            .write_bytes(physical, &source[done..done + segment])
            .map_err(|_| CopyUserError::Accessor)?;
        done += segment;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::paging::PageFlags;
    use super::*;

    const R: PageFlags = PageFlags::PRESENT.union(PageFlags::USER);
    const RW: PageFlags = R.union(PageFlags::WRITABLE);

    /// Address space with two RW pages at 0x1000 and one RO page at 0x3000;
    /// its physical frames sit inside 0x9000..0xc000 so every translated byte
    /// falls within the FrameMemory window used alongside it.
    fn mixed_space() -> AddressSpace<4> {
        let mut space = AddressSpace::<4>::new(0x5000).unwrap();
        space.map_user(0x1000, 0x9000, 2, RW).unwrap();
        space.map_user(0x3000, 0xb000, 1, R).unwrap();
        space
    }

    /// A backend over three physical pages starting at `base`.
    struct FrameMemory {
        base: u64,
        mem: [u8; 12_288],
    }

    impl FrameMemory {
        fn new(base: u64) -> Self {
            FrameMemory {
                base,
                mem: [0u8; 12_288],
            }
        }

        fn offset(&self, physical: u64, len: usize) -> Result<usize, AccessorFault> {
            let end = physical.checked_add(len as u64).ok_or(AccessorFault)?;
            let limit = self.base.checked_add(12_288).ok_or(AccessorFault)?;
            if physical < self.base || end > limit {
                return Err(AccessorFault);
            }
            Ok((physical - self.base) as usize)
        }
    }

    impl PhysicalAccess for FrameMemory {
        fn read_bytes(&mut self, physical: u64, out: &mut [u8]) -> Result<(), AccessorFault> {
            let start = self.offset(physical, out.len())?;
            out.copy_from_slice(&self.mem[start..start + out.len()]);
            Ok(())
        }

        fn write_bytes(&mut self, physical: u64, src: &[u8]) -> Result<(), AccessorFault> {
            let start = self.offset(physical, src.len())?;
            self.mem[start..start + src.len()].copy_from_slice(src);
            Ok(())
        }
    }

    #[test]
    fn roundtrip_crosses_the_page_boundary() {
        let space = mixed_space();
        let mut access = FrameMemory::new(0x9000);
        let source: [u8; 32] = core::array::from_fn(|i| (i as u8).wrapping_mul(7).wrapping_add(1));
        // 32 bytes at 0x1ff0 span pages 0x1000 and 0x2000.
        copy_user_bytes_out(&space, 0x1ff0, &source, &mut access).unwrap();
        assert_eq!(source[0], access.mem[0xff0]);
        assert_eq!(source[15], access.mem[0xfff]);
        assert_eq!(source[16], access.mem[0x1000]);
        assert_eq!(source[31], access.mem[0x100f]);

        let mut round = [0u8; 32];
        copy_user_bytes_in(&space, 0x1ff0, &mut round, &mut access).unwrap();
        assert_eq!(round, source);
    }

    #[test]
    fn permission_denial_and_unmapped_ranges_never_touch_the_backend() {
        let space = mixed_space();
        let mut access = FrameMemory::new(0x9000);
        let source = [0xaa; 32];

        // 0x2ff0 + 32 crosses into the read-only mapping; write is denied.
        assert_eq!(
            copy_user_bytes_out(&space, 0x2ff0, &source, &mut access),
            Err(CopyUserError::Access(UserCopyError::PermissionDenied))
        );
        assert!(access.mem.iter().all(|b| *b == 0));

        // Reading across the RW -> RO boundary is allowed.
        let mut buf = [0u8; 32];
        assert!(copy_user_bytes_in(&space, 0x2ff0, &mut buf, &mut access).is_ok());

        // 0x3ff0 + 32 starts on the read-only page and crosses into page
        // 0x4000. Reads fail with Unmapped on the second page; writes hit the
        // PermissionDenied of the first page first, because checks walk pages
        // in order.
        assert_eq!(
            copy_user_bytes_in(&space, 0x3ff0, &mut buf, &mut access),
            Err(CopyUserError::Access(UserCopyError::Unmapped))
        );
        assert_eq!(
            copy_user_bytes_out(&space, 0x3ff0, &source, &mut access),
            Err(CopyUserError::Access(UserCopyError::PermissionDenied))
        );
    }

    #[test]
    fn backend_faults_propagate_as_accessor_errors() {
        let mut space = AddressSpace::<2>::new(0x5000).unwrap();
        // 0xd000 lies outside the FrameMemory window 0x9000..0xc000.
        space.map_user(0x3000, 0xd000, 1, R).unwrap();
        let mut access = FrameMemory::new(0x9000);
        let mut buf = [0u8; 8];
        assert_eq!(
            copy_user_bytes_in(&space, 0x3000, &mut buf, &mut access),
            Err(CopyUserError::Accessor)
        );
    }

    #[test]
    fn zero_length_and_oversized_copies_fail_closed() {
        let space = mixed_space();
        let mut access = FrameMemory::new(0x9000);
        let empty: [u8; 0] = [];
        let mut empty_dst: [u8; 0] = [];
        assert_eq!(
            copy_user_bytes_in(&space, 0x1000, &mut empty_dst, &mut access),
            Err(CopyUserError::Access(UserCopyError::InvalidRange))
        );
        assert_eq!(
            copy_user_bytes_out(&space, 0x1000, &empty, &mut access),
            Err(CopyUserError::Access(UserCopyError::InvalidRange))
        );
        // The cap is checked before any mapping lookup.
        assert_eq!(
            validate_copy(&space, 0x1000, MAX_COPY_BYTES + 1, false),
            Err(CopyUserError::TooLarge)
        );
        // MAX_COPY_BYTES itself passes the cap and fails on unmapped range.
        assert_eq!(
            validate_copy(&space, 0x1000, MAX_COPY_BYTES, false),
            Err(CopyUserError::Access(UserCopyError::Unmapped))
        );
    }

    #[test]
    fn reads_are_split_per_page_segment() {
        let space = mixed_space();
        let chunks = core::cell::RefCell::new([0usize; 8]);
        struct Recorder<'a>(&'a core::cell::RefCell<[usize; 8]>);

        impl PhysicalAccess for Recorder<'_> {
            fn read_bytes(&mut self, _physical: u64, out: &mut [u8]) -> Result<(), AccessorFault> {
                let mut cells = self.0.borrow_mut();
                for cell in cells.iter_mut() {
                    if *cell == 0 {
                        *cell = out.len();
                        return Ok(());
                    }
                }
                Ok(())
            }

            fn write_bytes(&mut self, _physical: u64, _src: &[u8]) -> Result<(), AccessorFault> {
                Err(AccessorFault)
            }
        }

        let mut recorder = Recorder(&chunks);
        let mut buf = [0u8; 4112];
        copy_user_bytes_in(&space, 0x1000, &mut buf, &mut recorder).unwrap();
        assert_eq!(
            chunks.borrow().as_slice(),
            &[4096usize, 16, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn user_copy_page_size_matches_the_contract_layer() {
        assert_eq!(
            USER_COPY_PAGE_SIZE,
            super::super::address_space::ADDRESS_SPACE_PAGE_SIZE
        );
        assert_eq!(USER_COPY_PAGE_SIZE, 4096);
    }
}
