#![no_std]

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CapabilityId(pub u64);

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rights(pub u64);
impl Rights {
    pub const NONE: Self = Self(0);
    pub const READ: Self = Self(1 << 0);
    pub const WRITE: Self = Self(1 << 1);
    pub const EXECUTE: Self = Self(1 << 2);
    pub const MAP: Self = Self(1 << 3);
    pub const DEVICE: Self = Self(1 << 4);
    pub const NETWORK: Self = Self(1 << 5);
    pub const ADMIN: Self = Self(1 << 63);
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
    pub const fn intersect(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Capability {
    pub id: CapabilityId,
    pub rights: Rights,
}
impl Capability {
    pub const fn permits(&self, required: Rights) -> bool {
        self.rights.contains(required)
    }
    pub const fn derive(&self, id: CapabilityId, requested: Rights) -> Self {
        Self {
            id,
            rights: self.rights.intersect(requested),
        }
    }
    pub const fn revoked(id: CapabilityId) -> Self {
        Self {
            id,
            rights: Rights::NONE,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CapabilityError {
    Full,
    NotFound,
    Revoked,
    InvalidId,
    IdExhausted,
}

pub struct CapabilityTable<const N: usize> {
    entries: [Option<Capability>; N],
    next_id: u64,
}
impl<const N: usize> Default for CapabilityTable<N> {
    fn default() -> Self {
        Self::new()
    }
}
impl<const N: usize> CapabilityTable<N> {
    pub const fn new() -> Self {
        Self {
            entries: [None; N],
            next_id: 1,
        }
    }
    pub const fn len(&self) -> usize {
        let mut count = 0;
        let mut i = 0;
        while i < N {
            if self.entries[i].is_some() {
                count += 1;
            }
            i += 1;
        }
        count
    }
    pub const fn is_full(&self) -> bool {
        self.len() == N
    }
    pub fn insert(&mut self, rights: Rights) -> Result<CapabilityId, CapabilityError> {
        if N == 0 {
            return Err(CapabilityError::Full);
        }
        let mut i = 0;
        while i < N {
            if self.entries[i].is_none() {
                // Zero is an exhausted sentinel. Never wrap an issued ID
                // back to one: stale handles must not become valid again.
                if self.next_id == 0 {
                    return Err(CapabilityError::IdExhausted);
                }
                let raw_id = self.next_id;
                self.next_id = raw_id.checked_add(1).unwrap_or(0);
                let id = CapabilityId(raw_id);
                self.entries[i] = Some(Capability { id, rights });
                return Ok(id);
            }
            i += 1;
        }
        Err(CapabilityError::Full)
    }
    pub fn get(&self, id: CapabilityId) -> Result<Capability, CapabilityError> {
        let mut i = 0;
        while i < N {
            if let Some(cap) = self.entries[i]
                && cap.id == id
            {
                if cap.rights.is_empty() {
                    return Err(CapabilityError::Revoked);
                }
                return Ok(cap);
            }
            i += 1;
        }
        Err(CapabilityError::NotFound)
    }
    pub fn revoke(&mut self, id: CapabilityId) -> Result<(), CapabilityError> {
        let mut i = 0;
        while i < N {
            if let Some(cap) = self.entries[i]
                && cap.id == id
            {
                self.entries[i] = Some(Capability::revoked(id));
                return Ok(());
            }
            i += 1;
        }
        Err(CapabilityError::NotFound)
    }
    pub fn remove(&mut self, id: CapabilityId) -> Result<(), CapabilityError> {
        let mut i = 0;
        while i < N {
            if let Some(cap) = self.entries[i]
                && cap.id == id
            {
                self.entries[i] = None;
                return Ok(());
            }
            i += 1;
        }
        Err(CapabilityError::NotFound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn derivation_cannot_escalate() {
        let root = Capability {
            id: CapabilityId(1),
            rights: Rights::READ.union(Rights::WRITE),
        };
        let child = root.derive(CapabilityId(2), Rights::WRITE.union(Rights::NETWORK));
        assert!(child.permits(Rights::WRITE));
        assert!(!child.permits(Rights::NETWORK));
        assert!(!child.permits(Rights::READ));
    }
    #[test]
    fn table_allocates_revokes_and_reuses_slots() {
        let mut table: CapabilityTable<1> = CapabilityTable::new();
        let id = table.insert(Rights::READ).unwrap();
        assert_eq!(table.get(id).unwrap().rights, Rights::READ);
        assert_eq!(table.insert(Rights::WRITE), Err(CapabilityError::Full));
        assert!(table.revoke(id).is_ok());
        assert_eq!(table.get(id), Err(CapabilityError::Revoked));
        assert!(table.remove(id).is_ok());
        assert!(table.insert(Rights::WRITE).is_ok());
    }
    #[test]
    fn capability_ids_exhaust_without_wrapping_or_reusing_handles() {
        let mut table: CapabilityTable<3> = CapabilityTable::new();
        let first = table.insert(Rights::READ).unwrap();

        table.next_id = u64::MAX;
        let last = table.insert(Rights::WRITE).unwrap();
        assert_eq!(last, CapabilityId(u64::MAX));
        assert_eq!(
            table.insert(Rights::ADMIN),
            Err(CapabilityError::IdExhausted)
        );

        assert_eq!(table.get(first).unwrap().rights, Rights::READ);
        assert_eq!(table.get(last).unwrap().rights, Rights::WRITE);
    }

    #[test]
    fn zero_capacity_fails_closed() {
        let mut table: CapabilityTable<0> = CapabilityTable::new();
        assert_eq!(table.insert(Rights::READ), Err(CapabilityError::Full));
        assert!(table.is_full());
    }
    #[test]
    fn revoked_capability_has_no_authority() {
        let cap = Capability::revoked(CapabilityId(3));
        assert!(cap.rights.is_empty());
        assert!(!cap.permits(Rights::READ));
    }
}
