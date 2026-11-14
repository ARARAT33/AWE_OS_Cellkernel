//! Fixed-capacity in-memory block device used by storage integration tests.
//!
//! The caller owns the backing memory. Construction stays allocation-free and
//! does not return a 256 KiB object by value or create a large stack frame.

use super::{BLOCK_SIZE, BlockDevice, StorageError};

pub const RAMDISK_BLOCKS: usize = 64;
pub const RAMDISK_BYTES: usize = RAMDISK_BLOCKS * BLOCK_SIZE;

pub struct RamBlockDevice<'a> {
    data: &'a mut [u8; RAMDISK_BYTES],
    read_only: bool,
    dirty: bool,
}

impl<'a> RamBlockDevice<'a> {
    /// Create a zeroed ramdisk over caller-owned, exactly sized storage.
    ///
    /// The backing array must remain alive and exclusively borrowed for the
    /// lifetime of this device.
    pub fn new(data: &'a mut [u8; RAMDISK_BYTES], read_only: bool) -> Self {
        data.fill(0);
        Self {
            data,
            read_only,
            dirty: false,
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    fn block_offset(block: u64) -> Result<usize, StorageError> {
        if block >= RAMDISK_BLOCKS as u64 {
            return Err(StorageError::InvalidBlock);
        }
        (block as usize)
            .checked_mul(BLOCK_SIZE)
            .ok_or(StorageError::InvalidBlock)
    }
}

impl BlockDevice for RamBlockDevice<'_> {
    fn block_count(&self) -> u64 {
        RAMDISK_BLOCKS as u64
    }

    fn read_block(&mut self, block: u64, out: &mut [u8]) -> Result<(), StorageError> {
        if out.len() < BLOCK_SIZE {
            return Err(StorageError::BufferTooSmall);
        }
        let offset = Self::block_offset(block)?;
        out[..BLOCK_SIZE].copy_from_slice(&self.data[offset..offset + BLOCK_SIZE]);
        Ok(())
    }

    fn write_block(&mut self, block: u64, data: &[u8]) -> Result<(), StorageError> {
        if self.read_only {
            return Err(StorageError::ReadOnly);
        }
        if data.len() < BLOCK_SIZE {
            return Err(StorageError::BufferTooSmall);
        }
        let offset = Self::block_offset(block)?;
        self.data[offset..offset + BLOCK_SIZE].copy_from_slice(&data[..BLOCK_SIZE]);
        self.dirty = true;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), StorageError> {
        self.dirty = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

    #[test]
    fn round_trip_and_flush_are_deterministic() {
        let mut backing = std::vec![0xFFu8; RAMDISK_BYTES];
        let storage: &mut [u8; RAMDISK_BYTES] = backing
            .as_mut_slice()
            .try_into()
            .expect("exact ramdisk size");
        let mut disk = RamBlockDevice::new(storage, false);
        let mut initial = [0u8; BLOCK_SIZE];
        disk.read_block(3, &mut initial).expect("initial read");
        assert_eq!(initial, [0; BLOCK_SIZE]);

        let mut input = [0u8; BLOCK_SIZE];
        input[0] = 0xA5;
        input[BLOCK_SIZE - 1] = 0x5A;
        disk.write_block(3, &input).expect("write");
        assert!(disk.is_dirty());

        let mut output = [0u8; BLOCK_SIZE];
        disk.read_block(3, &mut output).expect("read");
        assert_eq!(output, input);
        disk.flush().expect("flush");
        assert!(!disk.is_dirty());
    }

    #[test]
    fn enforces_bounds_and_read_only_mode() {
        let mut backing = std::vec![0u8; RAMDISK_BYTES];
        let storage: &mut [u8; RAMDISK_BYTES] = backing
            .as_mut_slice()
            .try_into()
            .expect("exact ramdisk size");
        let mut disk = RamBlockDevice::new(storage, true);
        let block = [0u8; BLOCK_SIZE];
        assert_eq!(disk.write_block(0, &block), Err(StorageError::ReadOnly));
        assert_eq!(
            disk.read_block(RAMDISK_BLOCKS as u64, &mut [0; BLOCK_SIZE]),
            Err(StorageError::InvalidBlock)
        );
        assert_eq!(
            disk.read_block(0, &mut [0; BLOCK_SIZE - 1]),
            Err(StorageError::BufferTooSmall)
        );
    }
}
