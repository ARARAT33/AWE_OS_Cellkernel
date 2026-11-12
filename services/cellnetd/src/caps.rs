//! Caller capability tokens for remote cell IPC.
//!
//! Closes the trust boundary documented at PR #68 review time: the
//! advertised `required_caps` mask of a published cell is no longer pure
//! metadata. A caller (a local cell on the sending node, or a remote
//! node's cell on the receiving side) must present a single-use,
//! Ed25519-signed `CapabilityGrant` issued by the node's capability
//! authority. The grant binds the caller cell to a capability set that
//! must cover the destination cell's advertised requirements, inside a
//! validity window, with a per-grantee anti-replay sequence table.
//!
//! No new cryptography is introduced: signing and verification delegate
//! to `awe-securityd`'s Ed25519 primitives. Everything is fixed capacity
//! and allocation-free, matching the CellKernel service style.

use crate::CellNetError;

/// Wire length of the signed capability payload.
pub const CAP_SIGNED_LEN: usize = 49;
/// Length of the Ed25519 signature.
pub const CAP_SIG_LEN: usize = 64;
/// Total wire length of one capability token.
pub const CAP_TOKEN_LEN: usize = CAP_SIGNED_LEN + CAP_SIG_LEN;
/// Grant wire-format version. Decoders reject anything else.
pub const CAP_GRANT_VERSION: u8 = 1;
/// Bounded anti-replay table of consumed `(grantee, sequence)` pairs.
pub const MAX_USED_GRANTS: usize = 64;

/// A single-use, signed authorization for one cell to call with a
/// capability set.
///
/// The signed payload binds: format version, grantee node id, caller cell
/// id, capability mask, validity window and the anti-replay sequence.
/// A grant is valid only while `not_before <= now <= not_after`, only for
/// `grantee_node` acting as `cell_id`, and only when the destination
/// cell's advertised `required_caps` are a subset of `caps`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapabilityGrant {
    pub version: u8,
    pub grantee_node: u64,
    pub cell_id: u64,
    pub caps: u64,
    pub not_before: u64,
    pub not_after: u64,
    /// Anti-replay sequence; each sequence value is consumable once per
    /// grantee node for the whole replay-table lifetime.
    pub sequence: u64,
    pub signature: [u8; 64],
}

impl CapabilityGrant {
    pub const EMPTY: Self = Self {
        version: 0,
        grantee_node: 0,
        cell_id: 0,
        caps: 0,
        not_before: 0,
        not_after: 0,
        sequence: 0,
        signature: [0u8; 64],
    };

    /// Serialize the signed payload (everything except the signature).
    pub fn encode_signed(&self, out: &mut [u8; CAP_SIGNED_LEN]) {
        out[0] = self.version;
        out[1..9].copy_from_slice(&self.grantee_node.to_le_bytes());
        out[9..17].copy_from_slice(&self.cell_id.to_le_bytes());
        out[17..25].copy_from_slice(&self.caps.to_le_bytes());
        out[25..33].copy_from_slice(&self.not_before.to_le_bytes());
        out[33..41].copy_from_slice(&self.not_after.to_le_bytes());
        out[41..49].copy_from_slice(&self.sequence.to_le_bytes());
    }

    /// Serialize the full wire token: signed payload followed by signature.
    pub fn encode_token(&self, out: &mut [u8; CAP_TOKEN_LEN]) {
        let mut signed = [0u8; CAP_SIGNED_LEN];
        self.encode_signed(&mut signed);
        out[..CAP_SIGNED_LEN].copy_from_slice(&signed);
        out[CAP_SIGNED_LEN..].copy_from_slice(&self.signature);
    }

    /// Parse a wire token. Structural validity (version and window order)
    /// is checked by `verify`, so callers cannot skip it.
    pub fn decode_token(input: &[u8; CAP_TOKEN_LEN]) -> Self {
        let mut grant = Self::EMPTY;
        grant.version = input[0];
        grant.grantee_node = u64::from_le_bytes(input[1..9].try_into().expect("8 bytes"));
        grant.cell_id = u64::from_le_bytes(input[9..17].try_into().expect("8 bytes"));
        grant.caps = u64::from_le_bytes(input[17..25].try_into().expect("8 bytes"));
        grant.not_before = u64::from_le_bytes(input[25..33].try_into().expect("8 bytes"));
        grant.not_after = u64::from_le_bytes(input[33..41].try_into().expect("8 bytes"));
        grant.sequence = u64::from_le_bytes(input[41..49].try_into().expect("8 bytes"));
        grant.signature.copy_from_slice(&input[CAP_SIGNED_LEN..]);
        grant
    }

    /// Sign this grant with the capability issuer's Ed25519 secret key and
    /// stamp the current wire-format version.
    pub fn sign_with(&mut self, issuer_secret: &[u8; 64]) {
        self.version = CAP_GRANT_VERSION;
        let mut payload = [0u8; CAP_SIGNED_LEN];
        self.encode_signed(&mut payload);
        self.signature = awe_securityd::ed25519_sign(issuer_secret, &payload);
    }

    /// Structural checks only; cryptographic and authorization checks live
    /// in `CapabilityAuthority::verify`.
    pub fn is_well_formed(&self) -> bool {
        self.version == CAP_GRANT_VERSION && self.not_before <= self.not_after
    }
}

/// Verifies capability grants against one issuer key, with a bounded
/// single-use (anti-replay) table keyed by `(grantee node, sequence)`.
///
/// The table is fail-closed: when it is full, verification is refused
/// rather than evicting an entry that could still be replayed.
pub struct CapabilityAuthority {
    issuer_key: [u8; 32],
    used: [Option<(u64, u64)>; MAX_USED_GRANTS],
}

impl CapabilityAuthority {
    pub const fn new(issuer_key: [u8; 32]) -> Self {
        Self {
            issuer_key,
            used: [None; MAX_USED_GRANTS],
        }
    }

    pub fn issuer_key(&self) -> [u8; 32] {
        self.issuer_key
    }

    /// Verify and consume a grant.
    ///
    /// `grantee_node` is the node presenting the grant (the local node id
    /// on the send path, the authenticated peer on the receive path) and
    /// `cell_id` is the caller cell the grant must name. `required_caps`
    /// is the destination cell's advertised requirement mask.
    pub fn verify(
        &mut self,
        grant: &CapabilityGrant,
        now: u64,
        grantee_node: u64,
        cell_id: u64,
        required_caps: u64,
    ) -> Result<(), CellNetError> {
        if !grant.is_well_formed() {
            return Err(CellNetError::CapabilityDenied);
        }
        if grant.grantee_node != grantee_node || grant.cell_id != cell_id {
            return Err(CellNetError::CapabilityDenied);
        }
        if required_caps & !grant.caps != 0 {
            return Err(CellNetError::CapabilityDenied);
        }
        if now < grant.not_before || now > grant.not_after {
            return Err(CellNetError::CapabilityExpired);
        }
        let mut payload = [0u8; CAP_SIGNED_LEN];
        grant.encode_signed(&mut payload);
        if !awe_securityd::ed25519_verify(&self.issuer_key, &payload, &grant.signature) {
            return Err(CellNetError::BadSignature);
        }
        if self
            .used
            .iter()
            .flatten()
            .any(|&(node, sequence)| node == grantee_node && sequence == grant.sequence)
        {
            return Err(CellNetError::CapabilityReplayRejected);
        }
        let slot = self
            .used
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(CellNetError::CapabilityTableFull)?;
        *slot = Some((grantee_node, grant.sequence));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issuer() -> ([u8; 32], [u8; 64]) {
        awe_securityd::ed25519_keypair_from_seed(&[0x5E; 32])
    }

    fn signed_grant(sequence: u64, caps: u64) -> (CapabilityGrant, [u8; 32]) {
        let (public, secret) = issuer();
        let mut grant = CapabilityGrant {
            version: 0,
            grantee_node: 0xAAAA,
            cell_id: 0x1001,
            caps,
            not_before: 100,
            not_after: 200,
            sequence,
            signature: [0u8; 64],
        };
        grant.sign_with(&secret);
        (grant, public)
    }

    #[test]
    fn signed_grant_verifies_and_is_single_use() {
        let (grant, public) = signed_grant(1, 0xFF);
        let mut authority = CapabilityAuthority::new(public);
        authority
            .verify(&grant, 150, 0xAAAA, 0x1001, 0x0F)
            .expect("valid grant");
        assert_eq!(
            authority.verify(&grant, 150, 0xAAAA, 0x1001, 0x0F),
            Err(CellNetError::CapabilityReplayRejected)
        );
    }

    #[test]
    fn wrong_cell_or_node_is_denied() {
        let (grant, public) = signed_grant(1, 0xFF);
        let mut authority = CapabilityAuthority::new(public);
        assert_eq!(
            authority.verify(&grant, 150, 0xAAAA, 0x2002, 0),
            Err(CellNetError::CapabilityDenied)
        );
        assert_eq!(
            authority.verify(&grant, 150, 0xBBBB, 0x1001, 0),
            Err(CellNetError::CapabilityDenied)
        );
    }

    #[test]
    fn insufficient_caps_are_denied() {
        let (grant, public) = signed_grant(1, 0x01);
        let mut authority = CapabilityAuthority::new(public);
        assert_eq!(
            authority.verify(&grant, 150, 0xAAAA, 0x1001, 0x03),
            Err(CellNetError::CapabilityDenied)
        );
    }

    #[test]
    fn expired_and_future_grants_are_rejected() {
        let (grant, public) = signed_grant(1, 0xFF);
        let mut authority = CapabilityAuthority::new(public);
        assert_eq!(
            authority.verify(&grant, 201, 0xAAAA, 0x1001, 0),
            Err(CellNetError::CapabilityExpired)
        );
        assert_eq!(
            authority.verify(&grant, 99, 0xAAAA, 0x1001, 0),
            Err(CellNetError::CapabilityExpired)
        );
    }

    #[test]
    fn tampered_or_foreign_signed_grants_fail() {
        let (mut grant, public) = signed_grant(1, 0xFF);
        let mut authority = CapabilityAuthority::new(public);
        grant.caps |= 0x8000; // signed payload no longer matches
        assert_eq!(
            authority.verify(&grant, 150, 0xAAAA, 0x1001, 0x8000),
            Err(CellNetError::BadSignature)
        );

        // Signature from a different issuer key.
        let (_other_public, other_secret) = awe_securityd::ed25519_keypair_from_seed(&[0x6F; 32]);
        let mut forged = CapabilityGrant {
            version: 0,
            grantee_node: 0xAAAA,
            cell_id: 0x1001,
            caps: 0xFF,
            not_before: 100,
            not_after: 200,
            sequence: 2,
            signature: [0u8; 64],
        };
        forged.sign_with(&other_secret);
        assert_eq!(
            authority.verify(&forged, 150, 0xAAAA, 0x1001, 0xFF),
            Err(CellNetError::BadSignature)
        );
    }

    #[test]
    fn wrong_version_is_denied() {
        let (mut grant, public) = signed_grant(1, 0xFF);
        grant.version = 2;
        let mut authority = CapabilityAuthority::new(public);
        assert_eq!(
            authority.verify(&grant, 150, 0xAAAA, 0x1001, 0),
            Err(CellNetError::CapabilityDenied)
        );
    }

    #[test]
    fn replay_table_is_bounded_and_fails_closed() {
        let (public, _) = issuer();
        let mut authority = CapabilityAuthority::new(public);
        for sequence in 0..MAX_USED_GRANTS as u64 {
            let (grant, _) = signed_grant(sequence, 0xFF);
            authority
                .verify(&grant, 150, 0xAAAA, 0x1001, 0)
                .expect("slot");
        }
        let (grant, _) = signed_grant(MAX_USED_GRANTS as u64, 0xFF);
        assert_eq!(
            authority.verify(&grant, 150, 0xAAAA, 0x1001, 0),
            Err(CellNetError::CapabilityTableFull)
        );
    }

    #[test]
    fn token_roundtrip_preserves_all_fields() {
        let (grant, public) = signed_grant(9, 0xABCD);
        let mut wire = [0u8; CAP_TOKEN_LEN];
        grant.encode_token(&mut wire);
        let decoded = CapabilityGrant::decode_token(&wire);
        assert_eq!(decoded, grant);
        let mut authority = CapabilityAuthority::new(public);
        authority
            .verify(&decoded, 150, 0xAAAA, 0x1001, 0xABCD)
            .expect("decoded grant verifies");
    }
}
