//! Remote cell directory: the distributed counterpart of the kernel's
//! local `CellManager` registry.
//!
//! The directory only holds *published* cells of remote nodes. A node
//! advertises exactly the cells its publish policy permits; nothing else is
//! visible or addressable from the mesh.

use crate::{CellNetError, MAX_REMOTE_CELLS};

/// A published remote-cell descriptor.
///
/// `interface_hash` identifies the versioned interface contract
/// (`SHA-256(name)[..8]`), so callers bind to a semantic interface rather
/// than a transport address.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CellOffer {
    pub cell_id: u64,
    /// Kind discriminant mirroring `aweos_kernel::cell::CellKind`.
    pub kind: u8,
    /// Informational lifecycle state at announcement time.
    pub state: u8,
    pub abi_major: u16,
    pub abi_minor: u16,
    /// Kernel capabilities a caller must present to use this cell.
    pub required_caps: u64,
    /// First 8 bytes of the SHA-256 of the versioned interface name.
    pub interface_hash: u64,
    /// Advertised entry lifetime in ticks.
    pub ttl_ticks: u64,
}

impl CellOffer {
    pub const EMPTY: Self = Self {
        cell_id: 0,
        kind: 0,
        state: 0,
        abi_major: 0,
        abi_minor: 0,
        required_caps: 0,
        interface_hash: 0,
        ttl_ticks: 0,
    };

    pub fn is_valid(&self) -> bool {
        self.cell_id != 0
            && self.kind < crate::cell_kind::COUNT
            && self.interface_hash != 0
            && self.ttl_ticks > 0
    }
}

/// Health of a directory entry, derived from the owning peer's session
/// state and heartbeat freshness.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteHealth {
    Reachable,
    Suspect,
    Unreachable,
}

/// A remote cell learned through signed announcements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RemoteCell {
    pub node_id: u64,
    pub offer: CellOffer,
    /// Announcement sequence that produced this entry (anti-staleness).
    pub sequence: u64,
    pub last_seen_tick: u64,
    pub expires_tick: u64,
    pub health: RemoteHealth,
}

/// Fixed-capacity directory of remote cells.
pub struct Directory {
    entries: [Option<RemoteCell>; MAX_REMOTE_CELLS],
}

impl Default for Directory {
    fn default() -> Self {
        Self::new()
    }
}

impl Directory {
    pub const fn new() -> Self {
        Self {
            entries: [None; MAX_REMOTE_CELLS],
        }
    }

    fn find_slot(&self, node_id: u64, cell_id: u64) -> Option<usize> {
        self.entries.iter().position(|entry| {
            entry.is_some_and(|e| e.node_id == node_id && e.offer.cell_id == cell_id)
        })
    }

    /// Insert or update an entry. Stale updates (older announcement
    /// sequence) are ignored.
    pub fn upsert(&mut self, record: RemoteCell) -> Result<(), CellNetError> {
        if let Some(index) = self.find_slot(record.node_id, record.offer.cell_id) {
            let current = self.entries[index].expect("slot");
            if record.sequence < current.sequence {
                return Ok(());
            }
            self.entries[index] = Some(record);
            return Ok(());
        }
        let Some(index) = self.entries.iter().position(Option::is_none) else {
            return Err(CellNetError::DirectoryFull);
        };
        self.entries[index] = Some(record);
        Ok(())
    }

    pub fn remove(&mut self, node_id: u64, cell_id: u64) -> bool {
        if let Some(index) = self.find_slot(node_id, cell_id) {
            self.entries[index] = None;
            return true;
        }
        false
    }

    /// Drop every entry owned by `node_id` (used by authoritative
    /// DirectorySync snapshots and session teardown).
    pub fn remove_node(&mut self, node_id: u64) {
        for entry in self.entries.iter_mut() {
            if entry.is_some_and(|e| e.node_id == node_id) {
                *entry = None;
            }
        }
    }

    pub fn set_node_health(&mut self, node_id: u64, health: RemoteHealth) {
        for entry in self.entries.iter_mut() {
            if let Some(record) = entry
                && record.node_id == node_id
            {
                record.health = health;
            }
        }
    }

    /// Remove entries whose TTL has elapsed. Returns the number removed.
    pub fn sweep_expired(&mut self, now: u64) -> usize {
        let mut removed = 0;
        for entry in self.entries.iter_mut() {
            if entry.is_some_and(|e| now >= e.expires_tick) {
                *entry = None;
                removed += 1;
            }
        }
        removed
    }

    pub fn get(&self, node_id: u64, cell_id: u64) -> Option<RemoteCell> {
        self.find_slot(node_id, cell_id)
            .and_then(|index| self.entries[index])
    }

    /// Find the first reachable cell offering `interface_hash`.
    pub fn find_by_interface(&self, interface_hash: u64) -> Option<RemoteCell> {
        self.entries.iter().flatten().copied().find(|entry| {
            entry.offer.interface_hash == interface_hash && entry.health == RemoteHealth::Reachable
        })
    }

    pub fn iter(&self) -> impl Iterator<Item = &RemoteCell> {
        self.entries.iter().flatten()
    }

    pub fn len(&self) -> usize {
        self.entries.iter().flatten().count()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.iter().all(Option::is_none)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer(cell: u64, interface: u64) -> CellOffer {
        CellOffer {
            interface_hash: interface,
            ttl_ticks: 100,
            ..CellOffer {
                cell_id: cell,
                kind: crate::cell_kind::SERVICE,
                ..CellOffer::EMPTY
            }
        }
    }

    fn record(node: u64, cell: u64, seq: u64, interface: u64) -> RemoteCell {
        RemoteCell {
            node_id: node,
            offer: offer(cell, interface),
            sequence: seq,
            last_seen_tick: 0,
            expires_tick: 100,
            health: RemoteHealth::Reachable,
        }
    }

    #[test]
    fn upsert_rejects_stale_sequence() {
        let mut dir = Directory::new();
        dir.upsert(record(1, 10, 5, 0xAAAA)).expect("insert");
        dir.upsert(record(1, 10, 4, 0xBBBB)).expect("stale ok");
        assert_eq!(dir.get(1, 10).expect("entry").offer.interface_hash, 0xAAAA);
        dir.upsert(record(1, 10, 6, 0xBBBB)).expect("fresh");
        assert_eq!(dir.get(1, 10).expect("entry").offer.interface_hash, 0xBBBB);
    }

    #[test]
    fn snapshot_replaces_node_entries() {
        let mut dir = Directory::new();
        dir.upsert(record(1, 10, 1, 0x1)).expect("a");
        dir.upsert(record(2, 20, 1, 0x2)).expect("b");
        dir.remove_node(1);
        assert!(dir.get(1, 10).is_none());
        assert!(dir.get(2, 20).is_some());
    }

    #[test]
    fn expiry_sweep_and_interface_lookup() {
        let mut dir = Directory::new();
        dir.upsert(record(1, 10, 1, 0xC0FFEE)).expect("a");
        assert_eq!(
            dir.find_by_interface(0xC0FFEE)
                .expect("found")
                .offer
                .cell_id,
            10
        );
        assert_eq!(dir.sweep_expired(101), 1);
        assert!(dir.find_by_interface(0xC0FFEE).is_none());
    }
}
