//! AWE storage core contracts.
//! Hardware drivers implement these bounded interfaces; policy and filesystem
//! layers remain independent of the transport.

#![allow(dead_code)]

pub mod gpt;
pub mod journal;
pub mod ramdisk;
pub mod vfs;

pub use gpt::{
    GptError, GptHeader, GptPartition, crc32, parse_header, parse_partition,
    validate_partition_array_crc,
};
pub use journal::{JournalError, JournalState, JournalTxn, RecoveryDecision, decide_recovery};
pub use ramdisk::{RAMDISK_BLOCKS, RAMDISK_BYTES, RamBlockDevice};
pub use vfs::{AweFs, FileName, FsError, Inode, JournalRecord, NodeKind, RecoveryAction, Vfs};

pub const BLOCK_SIZE: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageError {
    InvalidBlock,
    BufferTooSmall,
    ReadOnly,
    Io,
    Unsupported,
    InvalidMetadata,
    TooLarge,
}
impl From<GptError> for StorageError {
    fn from(error: GptError) -> Self {
        match error {
            GptError::BufferTooSmall => Self::BufferTooSmall,
            GptError::InvalidHeaderSize
            | GptError::InvalidLbaRange
            | GptError::InvalidEntrySize
            | GptError::TooManyPartitions
            | GptError::InvalidPartitionRange
            | GptError::HeaderCrcMismatch
            | GptError::PartitionArrayCrcMismatch
            | GptError::BadSignature
            | GptError::UnsupportedRevision => Self::InvalidMetadata,
        }
    }
}

pub trait BlockDevice {
    fn block_size(&self) -> usize {
        BLOCK_SIZE
    }
    fn block_count(&self) -> u64;
    fn read_block(&mut self, block: u64, out: &mut [u8]) -> Result<(), StorageError>;
    fn write_block(&mut self, block: u64, data: &[u8]) -> Result<(), StorageError>;
    fn flush(&mut self) -> Result<(), StorageError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeviceGeometry {
    pub block_size: u32,
    pub blocks: u64,
    pub read_only: bool,
}
impl DeviceGeometry {
    pub const fn new(block_size: u32, blocks: u64, read_only: bool) -> Self {
        Self {
            block_size,
            blocks,
            read_only,
        }
    }
    pub const fn bytes(self) -> Option<u64> {
        self.blocks.checked_mul(self.block_size as u64)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GptScanSummary {
    pub header: GptHeader,
    pub partitions: u16,
}

pub fn scan_gpt<D: BlockDevice>(device: &mut D) -> Result<GptScanSummary, StorageError> {
    if device.block_size() < gpt::GPT_SECTOR_SIZE
        || !device.block_size().is_multiple_of(gpt::GPT_SECTOR_SIZE)
    {
        return Err(StorageError::Unsupported);
    }
    let sectors_per_block = device.block_size() / gpt::GPT_SECTOR_SIZE;
    let disk_last_lba = device
        .block_count()
        .checked_mul(sectors_per_block as u64)
        .and_then(|s| s.checked_sub(1))
        .ok_or(StorageError::InvalidBlock)?;
    let mut block = [0u8; BLOCK_SIZE];
    device.read_block(0, &mut block)?;
    let header = gpt::parse_header(&block[gpt::GPT_SECTOR_SIZE..], disk_last_lba)?;
    let entry_bytes = (header.partition_count as usize)
        .checked_mul(header.partition_entry_size as usize)
        .ok_or(StorageError::TooLarge)?;
    if entry_bytes > 16 * 1024 {
        return Err(StorageError::TooLarge);
    }
    let mut entries = [0u8; 16 * 1024];
    let first_block = header.partition_entry_lba / sectors_per_block as u64;
    let first_sector_in_block =
        (header.partition_entry_lba % sectors_per_block as u64) as usize * gpt::GPT_SECTOR_SIZE;
    let block_count = (first_sector_in_block + entry_bytes).div_ceil(device.block_size());
    for index in 0..block_count {
        device.read_block(first_block + index as u64, &mut block)?;
        let source_start = if index == 0 { first_sector_in_block } else { 0 };
        let destination_start = if index == 0 {
            0
        } else {
            index * device.block_size() - first_sector_in_block
        };
        let copy_len = core::cmp::min(
            device.block_size() - source_start,
            entry_bytes.saturating_sub(destination_start),
        );
        if copy_len == 0 {
            break;
        }
        entries[destination_start..destination_start + copy_len]
            .copy_from_slice(&block[source_start..source_start + copy_len]);
    }
    gpt::validate_partition_array_crc(&entries[..entry_bytes], header.partition_array_crc32)?;
    let mut partitions = 0u16;
    for index in 0..header.partition_count as usize {
        let start = index * header.partition_entry_size as usize;
        let end = start + header.partition_entry_size as usize;
        if gpt::parse_partition(&entries[start..end], &header, disk_last_lba)?.is_some() {
            partitions = partitions.saturating_add(1);
        }
    }
    Ok(GptScanSummary { header, partitions })
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    #[test]
    fn scans_real_gpt_metadata_through_block_device() {
        let mut backing = std::vec![0u8; RAMDISK_BYTES];
        let storage: &mut [u8; RAMDISK_BYTES] = backing
            .as_mut_slice()
            .try_into()
            .expect("exact ramdisk size");
        let mut disk = RamBlockDevice::new(storage, false);
        let mut block = [0u8; BLOCK_SIZE];
        let entry_offset = 1024usize;
        block[entry_offset] = 1;
        block[entry_offset + 16] = 2;
        block[entry_offset + 32..entry_offset + 40].copy_from_slice(&34u64.to_le_bytes());
        block[entry_offset + 40..entry_offset + 48].copy_from_slice(&100u64.to_le_bytes());
        let partition_crc = crc32(&block[entry_offset..entry_offset + 128]);
        let header_offset = 512usize;
        block[header_offset..header_offset + 8].copy_from_slice(&gpt::GPT_SIGNATURE);
        block[header_offset + 8..header_offset + 12]
            .copy_from_slice(&gpt::GPT_REVISION_1_0.to_le_bytes());
        block[header_offset + 12..header_offset + 16]
            .copy_from_slice(&(gpt::GPT_HEADER_MIN_SIZE as u32).to_le_bytes());
        block[header_offset + 24..header_offset + 32].copy_from_slice(&1u64.to_le_bytes());
        block[header_offset + 32..header_offset + 40].copy_from_slice(&511u64.to_le_bytes());
        block[header_offset + 40..header_offset + 48].copy_from_slice(&34u64.to_le_bytes());
        block[header_offset + 48..header_offset + 56].copy_from_slice(&480u64.to_le_bytes());
        block[header_offset + 72..header_offset + 80].copy_from_slice(&2u64.to_le_bytes());
        block[header_offset + 80..header_offset + 84].copy_from_slice(&1u32.to_le_bytes());
        block[header_offset + 84..header_offset + 88]
            .copy_from_slice(&(gpt::GPT_PARTITION_ENTRY_MIN_SIZE as u32).to_le_bytes());
        block[header_offset + 88..header_offset + 92].copy_from_slice(&partition_crc.to_le_bytes());
        let mut header_copy = block;
        header_copy[header_offset + 16..header_offset + 20].fill(0);
        let header_crc =
            crc32(&header_copy[header_offset..header_offset + gpt::GPT_HEADER_MIN_SIZE]);
        block[header_offset + 16..header_offset + 20].copy_from_slice(&header_crc.to_le_bytes());
        disk.write_block(0, &block).expect("write GPT");
        let summary = scan_gpt(&mut disk).expect("scan GPT");
        assert_eq!(summary.partitions, 1);
        assert_eq!(summary.header.partition_count, 1);
    }
}
