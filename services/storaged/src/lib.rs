//! AWEOS User-Space Storage Service (storaged).
//!
//! This crate currently provides bounded metadata tables and a block cache.
//! It does not yet perform device I/O, durable filesystem writes, or
//! copy-on-write snapshots.

#![no_std]

pub const MAX_VOLUMES: usize = 16;
pub const MAX_MOUNTS: usize = 16;
pub const MAX_SNAPSHOTS: usize = 8;
pub const MAX_PACKAGE_FILES: usize = 32;
pub const BLOCK_SIZE: usize = 512;
pub const CACHE_BLOCKS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeType {
    Ramdisk,
    GptPartition,
    VirtualDisk,
    AweFsVolume,
}

/// Metadata for a package file. This is not the file's durable contents.
#[derive(Debug, Clone, Copy)]
pub struct PackageFileRecord {
    pub file_id: u32,
    pub volume_id: u32,
    pub owner_process_id: u64,
    pub name_hash: u64,
    pub size_bytes: u32,
    pub is_installed_package: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct StorageVolume {
    pub volume_id: u32,
    pub owner_process_id: u64,
    pub volume_type: VolumeType,
    pub block_count: u64,
    pub start_lba: u64,
    pub read_only: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct MountEntry {
    pub mount_id: u32,
    pub volume_id: u32,
    pub owner_process_id: u64,
    pub path_hash: u64,
    pub active: bool,
}

/// Snapshot metadata only; this record does not represent a persisted
/// copy-on-write snapshot or a recoverable block image.
#[derive(Debug, Clone, Copy)]
pub struct SnapshotMetadata {
    pub snapshot_id: u32,
    pub volume_id: u32,
    pub owner_process_id: u64,
    pub timestamp: u64,
    pub block_delta_count: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct CachedBlock {
    pub volume_id: u32,
    pub owner_process_id: u64,
    pub lba: u64,
    pub dirty: bool,
    pub data: [u8; BLOCK_SIZE],
}

/// Bounded metadata/cache manager for the storage service.
///
/// The owner process ID is an ownership-consistency key supplied by the
/// service caller. This crate does not authenticate that identity or prove
/// that it is live; a future trusted service IPC/supervisor boundary must
/// derive it from kernel-owned caller context before exposing these methods
/// to untrusted processes.
#[derive(Debug)]
pub struct StorageDaemon {
    volumes: [Option<StorageVolume>; MAX_VOLUMES],
    mounts: [Option<MountEntry>; MAX_MOUNTS],
    snapshots: [Option<SnapshotMetadata>; MAX_SNAPSHOTS],
    files: [Option<PackageFileRecord>; MAX_PACKAGE_FILES],
    cache: [Option<CachedBlock>; CACHE_BLOCKS],
    volume_counter: u32,
    mount_counter: u32,
    snapshot_counter: u32,
    file_counter: u32,
    pub recovery_requests: usize,
}

impl StorageDaemon {
    pub const fn new() -> Self {
        Self {
            volumes: [None; MAX_VOLUMES],
            mounts: [None; MAX_MOUNTS],
            snapshots: [None; MAX_SNAPSHOTS],
            files: [None; MAX_PACKAGE_FILES],
            cache: [None; CACHE_BLOCKS],
            volume_counter: 1,
            mount_counter: 1,
            snapshot_counter: 1,
            file_counter: 1,
            recovery_requests: 0,
        }
    }

    /// Allocate a monotonic non-zero ID. The maximum ID may be issued once;
    /// zero then becomes a permanent exhausted sentinel and is never reused.
    fn allocate_id(counter: &mut u32, exhausted: &'static str) -> Result<u32, &'static str> {
        let id = *counter;
        if id == 0 {
            return Err(exhausted);
        }
        *counter = id.checked_add(1).unwrap_or(0);
        Ok(id)
    }

    fn validate_volume_owner(
        &self,
        owner_process_id: u64,
        volume_id: u32,
    ) -> Result<&StorageVolume, &'static str> {
        if owner_process_id == 0 {
            return Err("Owner process ID must be non-zero");
        }
        let volume = self
            .volumes
            .iter()
            .flatten()
            .find(|volume| volume.volume_id == volume_id)
            .ok_or("Volume ID not found")?;
        if volume.owner_process_id != owner_process_id {
            return Err("Permission denied: volume belongs to another owner");
        }
        Ok(volume)
    }

    /// Register a bounded volume metadata record for a non-zero owner ID.
    ///
    /// Ownership is an in-service consistency check, not caller authentication.
    pub fn register_volume(
        &mut self,
        owner_process_id: u64,
        vol_type: VolumeType,
        block_count: u64,
        start_lba: u64,
        read_only: bool,
    ) -> Result<u32, &'static str> {
        if owner_process_id == 0 {
            return Err("Owner process ID must be non-zero");
        }
        if block_count == 0 {
            return Err("Volume block count must be non-zero");
        }
        if start_lba.checked_add(block_count).is_none() {
            return Err("Volume LBA range overflows");
        }

        let slot = self
            .volumes
            .iter()
            .position(|entry| entry.is_none())
            .ok_or("Volume table full")?;
        let volume_id = Self::allocate_id(&mut self.volume_counter, "Volume ID space exhausted")?;
        self.volumes[slot] = Some(StorageVolume {
            volume_id,
            owner_process_id,
            volume_type: vol_type,
            block_count,
            start_lba,
            read_only,
        });
        Ok(volume_id)
    }

    /// Record a mount request after verifying the volume owner.
    ///
    /// This is metadata registration, not a VFS mount or filesystem operation.
    pub fn mount_volume(
        &mut self,
        owner_process_id: u64,
        volume_id: u32,
        path_hash: u64,
    ) -> Result<u32, &'static str> {
        self.validate_volume_owner(owner_process_id, volume_id)?;
        let slot = self
            .mounts
            .iter()
            .position(|entry| entry.is_none())
            .ok_or("Mount table full")?;
        let mount_id = Self::allocate_id(&mut self.mount_counter, "Mount ID space exhausted")?;
        self.mounts[slot] = Some(MountEntry {
            mount_id,
            volume_id,
            owner_process_id,
            path_hash,
            active: true,
        });
        Ok(mount_id)
    }

    /// Create a snapshot metadata record for an owned, existing volume.
    ///
    /// This does not copy blocks, persist a snapshot, or provide rollback.
    pub fn create_snapshot(
        &mut self,
        owner_process_id: u64,
        volume_id: u32,
        timestamp: u64,
    ) -> Result<u32, &'static str> {
        self.validate_volume_owner(owner_process_id, volume_id)?;
        let slot = self
            .snapshots
            .iter()
            .position(|entry| entry.is_none())
            .ok_or("Snapshot table full")?;
        let snapshot_id =
            Self::allocate_id(&mut self.snapshot_counter, "Snapshot ID space exhausted")?;
        self.snapshots[slot] = Some(SnapshotMetadata {
            snapshot_id,
            volume_id,
            owner_process_id,
            timestamp,
            block_delta_count: 0,
        });
        Ok(snapshot_id)
    }

    /// Read a cached block only when the supplied owner matches the volume.
    pub fn cache_read(
        &self,
        owner_process_id: u64,
        volume_id: u32,
        lba: u64,
    ) -> Option<&[u8; BLOCK_SIZE]> {
        let volume = self
            .validate_volume_owner(owner_process_id, volume_id)
            .ok()?;
        if lba >= volume.block_count {
            return None;
        }
        self.cache
            .iter()
            .flatten()
            .find(|block| {
                block.volume_id == volume_id
                    && block.owner_process_id == owner_process_id
                    && block.lba == lba
            })
            .map(|block| &block.data)
    }

    /// Update/insert a cache block after owner and volume-relative LBA checks.
    pub fn cache_write(
        &mut self,
        owner_process_id: u64,
        volume_id: u32,
        lba: u64,
        data: &[u8; BLOCK_SIZE],
    ) -> Result<(), &'static str> {
        let block_count = self
            .validate_volume_owner(owner_process_id, volume_id)?
            .block_count;
        if lba >= block_count {
            return Err("Block LBA is outside the volume");
        }

        for block in self.cache.iter_mut().flatten() {
            if block.volume_id == volume_id && block.lba == lba {
                if block.owner_process_id != owner_process_id {
                    return Err("Permission denied: cached block belongs to another owner");
                }
                block.data = *data;
                block.dirty = true;
                return Ok(());
            }
        }

        let slot = self
            .cache
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or("Block cache full")?;
        *slot = Some(CachedBlock {
            volume_id,
            owner_process_id,
            lba,
            dirty: true,
            data: *data,
        });
        Ok(())
    }

    /// Register package-file metadata for an owned, existing volume.
    ///
    /// The package bytes are not written by this metadata operation.
    pub fn store_package_file(
        &mut self,
        owner_process_id: u64,
        volume_id: u32,
        name_hash: u64,
        size_bytes: u32,
    ) -> Result<u32, &'static str> {
        self.validate_volume_owner(owner_process_id, volume_id)?;
        let slot = self
            .files
            .iter()
            .position(|entry| entry.is_none())
            .ok_or("File records table full")?;
        let file_id = Self::allocate_id(&mut self.file_counter, "File ID space exhausted")?;
        self.files[slot] = Some(PackageFileRecord {
            file_id,
            volume_id,
            owner_process_id,
            name_hash,
            size_bytes,
            is_installed_package: true,
        });
        Ok(file_id)
    }

    /// Delete a package metadata record only for its recorded owner.
    pub fn delete_package_file(
        &mut self,
        owner_process_id: u64,
        file_id: u32,
    ) -> Result<(), &'static str> {
        if owner_process_id == 0 {
            return Err("Owner process ID must be non-zero");
        }
        for slot in self.files.iter_mut() {
            if let Some(file) = slot
                && file.file_id == file_id
            {
                if file.owner_process_id != owner_process_id {
                    return Err("Permission denied: package record belongs to another owner");
                }
                *slot = None;
                return Ok(());
            }
        }
        Err("File not found")
    }

    /// Record that an owned volume has a snapshot candidate for future recovery.
    ///
    /// This only records a request; it does not repair data or restore blocks.
    pub fn request_snapshot_recovery(
        &mut self,
        owner_process_id: u64,
        volume_id: u32,
    ) -> Result<bool, &'static str> {
        self.validate_volume_owner(owner_process_id, volume_id)?;
        let has_snapshot = self.snapshots.iter().flatten().any(|snapshot| {
            snapshot.volume_id == volume_id && snapshot.owner_process_id == owner_process_id
        });
        if has_snapshot {
            self.recovery_requests = self.recovery_requests.saturating_add(1);
        }
        Ok(has_snapshot)
    }
}

impl Default for StorageDaemon {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWNER: u64 = 41;
    const OTHER_OWNER: u64 = 42;

    fn create_volume(storaged: &mut StorageDaemon) -> u32 {
        storaged
            .register_volume(OWNER, VolumeType::Ramdisk, 2048, 0, false)
            .unwrap()
    }

    #[test]
    fn test_storaged_lifecycle() {
        let mut storaged = StorageDaemon::new();
        let volume_id = create_volume(&mut storaged);
        assert_eq!(volume_id, 1);

        let mount_id = storaged
            .mount_volume(OWNER, volume_id, 0x1234_5678)
            .unwrap();
        assert_eq!(mount_id, 1);

        let snapshot_id = storaged.create_snapshot(OWNER, volume_id, 1000).unwrap();
        assert_eq!(snapshot_id, 1);

        let block_data = [0xAB; BLOCK_SIZE];
        storaged
            .cache_write(OWNER, volume_id, 42, &block_data)
            .unwrap();
        let cached = storaged
            .cache_read(OWNER, volume_id, 42)
            .expect("Should hit block cache");
        assert_eq!(cached[0], 0xAB);

        let file_id = storaged
            .store_package_file(OWNER, volume_id, 0x112233, 4096)
            .unwrap();
        assert_eq!(file_id, 1);
        assert!(
            storaged
                .request_snapshot_recovery(OWNER, volume_id)
                .unwrap()
        );
        assert_eq!(storaged.recovery_requests, 1);

        storaged.delete_package_file(OWNER, file_id).unwrap();
        assert!(storaged.delete_package_file(OWNER, file_id).is_err());
    }

    #[test]
    fn owner_mismatch_is_rejected_for_volume_children() {
        let mut storaged = StorageDaemon::new();
        let volume_id = create_volume(&mut storaged);
        let data = [0x5A; BLOCK_SIZE];

        assert!(storaged.mount_volume(OTHER_OWNER, volume_id, 0xAA).is_err());
        assert!(
            storaged
                .create_snapshot(OTHER_OWNER, volume_id, 2000)
                .is_err()
        );
        assert!(
            storaged
                .cache_write(OTHER_OWNER, volume_id, 0, &data)
                .is_err()
        );
        assert!(storaged.cache_read(OTHER_OWNER, volume_id, 0).is_none());
        assert!(
            storaged
                .store_package_file(OTHER_OWNER, volume_id, 0xBB, 12)
                .is_err()
        );
        assert!(
            storaged
                .request_snapshot_recovery(OTHER_OWNER, volume_id)
                .is_err()
        );

        let file_id = storaged
            .store_package_file(OWNER, volume_id, 0xCC, 12)
            .unwrap();
        assert!(storaged.delete_package_file(OTHER_OWNER, file_id).is_err());
        assert!(storaged.delete_package_file(OWNER, file_id).is_ok());
    }

    #[test]
    fn zero_owner_empty_volume_and_overflowing_lba_range_are_rejected() {
        let mut storaged = StorageDaemon::new();
        assert!(
            storaged
                .register_volume(0, VolumeType::Ramdisk, 1, 0, false)
                .is_err()
        );
        assert!(
            storaged
                .register_volume(OWNER, VolumeType::Ramdisk, 0, 0, false)
                .is_err()
        );
        assert!(
            storaged
                .register_volume(OWNER, VolumeType::Ramdisk, 2, u64::MAX, false)
                .is_err()
        );
        assert_eq!(storaged.volume_counter, 1);
        assert!(storaged.volumes.iter().all(Option::is_none));
    }

    #[test]
    fn unknown_volumes_cannot_create_mount_snapshot_file_or_cache_entries() {
        let mut storaged = StorageDaemon::new();
        let data = [0x11; BLOCK_SIZE];

        assert!(storaged.mount_volume(OWNER, 99, 0).is_err());
        assert!(storaged.create_snapshot(OWNER, 99, 0).is_err());
        assert!(storaged.store_package_file(OWNER, 99, 0, 0).is_err());
        assert!(storaged.cache_write(OWNER, 99, 0, &data).is_err());
        assert!(storaged.cache_read(OWNER, 99, 0).is_none());
        assert_eq!(storaged.mount_counter, 1);
        assert_eq!(storaged.snapshot_counter, 1);
        assert_eq!(storaged.file_counter, 1);
    }

    #[test]
    fn cache_rejects_lbas_outside_the_volume() {
        let mut storaged = StorageDaemon::new();
        let volume_id = storaged
            .register_volume(OWNER, VolumeType::Ramdisk, 2, 0, false)
            .unwrap();
        let data = [0x22; BLOCK_SIZE];

        assert!(storaged.cache_write(OWNER, volume_id, 2, &data).is_err());
        assert!(storaged.cache_read(OWNER, volume_id, 2).is_none());
    }

    #[test]
    fn all_id_spaces_exhaust_without_wrapping_or_reusing_zero() {
        let mut volumes = StorageDaemon::new();
        volumes.volume_counter = u32::MAX;
        assert_eq!(
            volumes.register_volume(OWNER, VolumeType::Ramdisk, 1, 0, false),
            Ok(u32::MAX)
        );
        assert_eq!(
            volumes.register_volume(OWNER, VolumeType::Ramdisk, 1, 0, false),
            Err("Volume ID space exhausted")
        );

        let mut mounts = StorageDaemon::new();
        let volume_id = create_volume(&mut mounts);
        mounts.mount_counter = u32::MAX;
        assert_eq!(mounts.mount_volume(OWNER, volume_id, 1), Ok(u32::MAX));
        assert_eq!(
            mounts.mount_volume(OWNER, volume_id, 2),
            Err("Mount ID space exhausted")
        );

        let mut snapshots = StorageDaemon::new();
        let volume_id = create_volume(&mut snapshots);
        snapshots.snapshot_counter = u32::MAX;
        assert_eq!(snapshots.create_snapshot(OWNER, volume_id, 1), Ok(u32::MAX));
        assert_eq!(
            snapshots.create_snapshot(OWNER, volume_id, 2),
            Err("Snapshot ID space exhausted")
        );

        let mut files = StorageDaemon::new();
        let volume_id = create_volume(&mut files);
        files.file_counter = u32::MAX;
        assert_eq!(
            files.store_package_file(OWNER, volume_id, 1, 1),
            Ok(u32::MAX)
        );
        assert_eq!(
            files.store_package_file(OWNER, volume_id, 2, 2),
            Err("File ID space exhausted")
        );
    }

    #[test]
    fn full_volume_table_does_not_consume_an_id() {
        let mut storaged = StorageDaemon::new();
        for _ in 0..MAX_VOLUMES {
            create_volume(&mut storaged);
        }
        let next_id = storaged.volume_counter;

        assert_eq!(
            storaged.register_volume(OWNER, VolumeType::Ramdisk, 1, 0, false),
            Err("Volume table full")
        );
        assert_eq!(storaged.volume_counter, next_id);
    }

    #[test]
    fn exhausted_id_sentinel_is_permanent() {
        let mut storaged = StorageDaemon::new();
        storaged.volume_counter = 0;
        assert_eq!(
            storaged.register_volume(OWNER, VolumeType::Ramdisk, 1, 0, false),
            Err("Volume ID space exhausted")
        );
        assert!(storaged.volumes.iter().all(Option::is_none));
    }
}
