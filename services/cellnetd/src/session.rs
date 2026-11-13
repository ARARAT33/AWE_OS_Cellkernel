//! Session cryptography: handshake key schedule, AEAD envelope sealing and
//! the anti-replay window. All primitives come from `awe-securityd`.

use crate::wire::{
    EnvelopeHeader, FINISHED_PAYLOAD_LEN, MAX_ENVELOPE_PAYLOAD, MessageType,
    decode_envelope_header, encode_envelope_header,
};
use crate::{CellNetError, ENVELOPE_HEADER_LEN, REPLAY_WINDOW, node_id_for_pubkey, scrub};

/// Long-term node identity: an Ed25519 keypair plus the derived node id.
#[derive(Clone)]
pub struct NodeIdentity {
    pub node_id: u64,
    pub public_key: [u8; 32],
    /// 64-byte Ed25519 secret key (seed ‖ public key), as issued by
    /// `awe_securityd::ed25519_keypair_from_seed`.
    secret_key: [u8; 64],
}

impl NodeIdentity {
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        let (public_key, secret_key) = awe_securityd::ed25519_keypair_from_seed(seed);
        Self {
            node_id: node_id_for_pubkey(&public_key),
            public_key,
            secret_key,
        }
    }

    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        awe_securityd::ed25519_sign(&self.secret_key, message)
    }
}

impl Drop for NodeIdentity {
    fn drop(&mut self) {
        scrub(&mut self.secret_key);
    }
}

/// A discovered mesh node (may or may not have an active session).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeRecord {
    pub node_id: u64,
    pub public_key: [u8; 32],
    /// Last transport address this node announced or handshaked from.
    pub address: u64,
    /// Highest announcement sequence accepted from this node.
    pub announce_sequence: u64,
    pub last_seen_tick: u64,
}

/// Sliding-window anti-replay tracker (IPsec-style, 64 entries wide).
#[derive(Clone, Copy, Debug, Default)]
pub struct ReplayWindow {
    max_sequence: u64,
    bitmap: u64,
    initialized: bool,
}

impl ReplayWindow {
    /// Accept and mark `sequence`; returns `false` for duplicates or
    /// sequences older than the window.
    pub fn check_and_mark(&mut self, sequence: u64) -> bool {
        if !self.initialized {
            self.initialized = true;
            self.max_sequence = sequence;
            self.bitmap = 1;
            return true;
        }
        if sequence > self.max_sequence {
            let shift = sequence - self.max_sequence;
            self.bitmap = if shift >= REPLAY_WINDOW {
                1
            } else {
                (self.bitmap << shift) | 1
            };
            self.max_sequence = sequence;
            return true;
        }
        let distance = self.max_sequence - sequence;
        if distance >= REPLAY_WINDOW {
            return false;
        }
        let bit = 1u64 << distance;
        if self.bitmap & bit != 0 {
            return false;
        }
        self.bitmap |= bit;
        true
    }
}

/// Established session keys for one peer.
#[derive(Clone)]
pub struct Session {
    pub session_id: u64,
    tx_key: [u8; 32],
    rx_key: [u8; 32],
    next_tx_sequence: u64,
    rx_window: ReplayWindow,
}

impl Drop for Session {
    fn drop(&mut self) {
        scrub(&mut self.tx_key);
        scrub(&mut self.rx_key);
    }
}

impl core::fmt::Debug for Session {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Session")
            .field("session_id", &self.session_id)
            .field("next_tx_sequence", &self.next_tx_sequence)
            .finish_non_exhaustive()
    }
}

impl Session {
    /// HKDF-SHA256 key schedule.
    ///
    /// - salt: `initiator_nonce ‖ responder_nonce`
    /// - ikm: X25519 shared secret
    /// - info: protocol label ‖ handshake transcript hash
    ///
    /// Expands to `k_i2r ‖ k_r2i ‖ session_id`.
    pub fn derive(
        shared_secret: &[u8; 32],
        transcript: &[u8; 32],
        initiator_nonce: &[u8; 32],
        responder_nonce: &[u8; 32],
        is_initiator: bool,
    ) -> Option<Self> {
        let mut salt = [0u8; 64];
        salt[..32].copy_from_slice(initiator_nonce);
        salt[32..].copy_from_slice(responder_nonce);

        let mut info = [0u8; 22 + 32];
        info[..22].copy_from_slice(b"AWE-CELLNET-SESSION-V1");
        info[22..].copy_from_slice(transcript);

        let mut okm = [0u8; 72];
        if !awe_securityd::hkdf_sha256(&salt, shared_secret, &info, &mut okm) {
            return None;
        }
        let k_i2r: [u8; 32] = okm[..32].try_into().expect("32 bytes");
        let k_r2i: [u8; 32] = okm[32..64].try_into().expect("32 bytes");
        let session_id = u64::from_le_bytes(okm[64..72].try_into().expect("8 bytes"));
        scrub(&mut okm);

        let (tx_key, rx_key) = if is_initiator {
            (k_i2r, k_r2i)
        } else {
            (k_r2i, k_i2r)
        };
        Some(Self {
            session_id,
            tx_key,
            rx_key,
            next_tx_sequence: 1,
            rx_window: ReplayWindow::default(),
        })
    }

    /// Transcript confirmation tag sent as the Finished payload.
    ///
    /// `HMAC-SHA256(tx_key, "AWE-CELLNET-CONFIRM-V1" ‖ transcript)`.
    pub fn confirm_tag(&self, transcript: &[u8; 32]) -> [u8; 32] {
        let mut message = [0u8; 22 + 32];
        message[..22].copy_from_slice(b"AWE-CELLNET-CONFIRM-V1");
        message[22..].copy_from_slice(transcript);
        awe_securityd::hmac_sha256(&self.tx_key, &message)
    }

    /// Verify a Finished payload produced by the peer (its tx key is our rx key).
    pub fn verify_confirm(&self, transcript: &[u8; 32], tag: &[u8; 32]) -> bool {
        let mut message = [0u8; 22 + 32];
        message[..22].copy_from_slice(b"AWE-CELLNET-CONFIRM-V1");
        message[22..].copy_from_slice(transcript);
        let expected = awe_securityd::hmac_sha256(&self.rx_key, &message);
        constant_time_eq(&expected, tag)
    }

    fn nonce(&self, sequence: u64) -> [u8; 12] {
        let mut nonce = [0u8; 12];
        nonce[..4].copy_from_slice(&self.session_id.to_le_bytes()[..4]);
        nonce[4..].copy_from_slice(&sequence.to_le_bytes());
        nonce
    }

    /// Encrypt `payload` into `out`, returning the total wire length.
    pub fn seal(
        &mut self,
        message_type: MessageType,
        payload: &[u8],
        out: &mut [u8],
    ) -> Result<usize, CellNetError> {
        if payload.len() > MAX_ENVELOPE_PAYLOAD
            || out.len() < ENVELOPE_HEADER_LEN + payload.len() + 16
        {
            return Err(CellNetError::Malformed);
        }
        let sequence = self.next_tx_sequence;
        self.next_tx_sequence = self
            .next_tx_sequence
            .checked_add(1)
            .ok_or(CellNetError::Malformed)?;

        let header = EnvelopeHeader {
            message_type,
            session_id: self.session_id,
            sequence,
            payload_len: payload.len(),
        };
        let payload_range = ENVELOPE_HEADER_LEN..ENVELOPE_HEADER_LEN + payload.len();
        let (head, rest) = out.split_at_mut(ENVELOPE_HEADER_LEN);
        encode_envelope_header(&header, head)?;
        rest[..payload.len()].copy_from_slice(payload);
        let tag = awe_securityd::chacha20poly1305_encrypt(
            &self.tx_key,
            &self.nonce(sequence),
            head,
            &mut rest[..payload.len()],
        )
        .ok_or(CellNetError::Malformed)?;
        rest[payload.len()..payload.len() + 16].copy_from_slice(&tag);
        Ok(payload_range.end + 16)
    }

    /// Authenticate and decrypt an envelope in place. Returns the message
    /// type and the decrypted payload range inside `buffer`.
    pub fn open(
        &mut self,
        buffer: &mut [u8],
    ) -> Result<(MessageType, core::ops::Range<usize>), CellNetError> {
        let header = decode_envelope_header(buffer)?;
        if header.session_id != self.session_id {
            return Err(CellNetError::Malformed);
        }
        let payload_range = ENVELOPE_HEADER_LEN..ENVELOPE_HEADER_LEN + header.payload_len;
        let tag: [u8; 16] = buffer[payload_range.end..payload_range.end + 16]
            .try_into()
            .map_err(|_| CellNetError::Malformed)?;
        let (head, rest) = buffer.split_at_mut(ENVELOPE_HEADER_LEN);
        if !awe_securityd::chacha20poly1305_decrypt(
            &self.rx_key,
            &self.nonce(header.sequence),
            &head[..ENVELOPE_HEADER_LEN],
            &mut rest[..header.payload_len],
            &tag,
        ) {
            return Err(CellNetError::DecryptFailed);
        }
        if !self.rx_window.check_and_mark(header.sequence) {
            return Err(CellNetError::ReplayRejected);
        }
        Ok((header.message_type, payload_range))
    }
}

/// Constant-time byte-string equality for authentication tags.
pub fn constant_time_eq(a: &[u8; 32], b: &[u8; 32]) -> bool {
    let mut diff = 0u8;
    for index in 0..32 {
        diff |= a[index] ^ b[index];
    }
    diff == 0
}

/// Build the Finished payload for `session`.
pub fn finished_payload(session: &Session, transcript: &[u8; 32]) -> [u8; FINISHED_PAYLOAD_LEN] {
    session.confirm_tag(transcript)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_session_pair() -> (Session, Session) {
        let shared = [0x5Au8; 32];
        let transcript = [0x11u8; 32];
        let nonce_i = [0x01u8; 32];
        let nonce_r = [0x02u8; 32];
        let initiator =
            Session::derive(&shared, &transcript, &nonce_i, &nonce_r, true).expect("initiator");
        let responder =
            Session::derive(&shared, &transcript, &nonce_i, &nonce_r, false).expect("responder");
        (initiator, responder)
    }

    #[test]
    fn derived_sessions_have_matching_cross_keys() {
        let (mut a, mut b) = test_session_pair();
        assert_eq!(a.session_id, b.session_id);
        let mut wire = [0u8; 256];
        let len = a
            .seal(MessageType::Heartbeat, b"ping-ping", &mut wire)
            .expect("seal");
        let (kind, range) = b.open(&mut wire[..len]).expect("open");
        assert_eq!(kind, MessageType::Heartbeat);
        assert_eq!(&wire[range], b"ping-ping");
    }

    #[test]
    fn confirm_tags_verify_both_directions() {
        let (a, b) = test_session_pair();
        let transcript = [0x11u8; 32];
        assert!(b.verify_confirm(&transcript, &a.confirm_tag(&transcript)));
        assert!(a.verify_confirm(&transcript, &b.confirm_tag(&transcript)));
        assert!(!b.verify_confirm(&transcript, &[0xFF; 32]));
    }

    #[test]
    fn replay_window_rejects_duplicates_and_old_sequences() {
        let mut window = ReplayWindow::default();
        assert!(window.check_and_mark(1));
        assert!(!window.check_and_mark(1));
        assert!(window.check_and_mark(2));
        assert!(window.check_and_mark(3));
        assert!(!window.check_and_mark(2));
        assert!(window.check_and_mark(100));
        assert!(!window.check_and_mark(3));
        assert!(!window.check_and_mark(1));
    }

    #[test]
    fn open_rejects_tampered_ciphertext_and_wrong_session() {
        let (mut a, mut b) = test_session_pair();
        let mut wire = [0u8; 256];
        let len = a
            .seal(MessageType::CellIpc, b"cell-data", &mut wire)
            .expect("seal");
        wire[ENVELOPE_HEADER_LEN] ^= 1;
        assert_eq!(b.open(&mut wire[..len]), Err(CellNetError::DecryptFailed));

        let len = a
            .seal(MessageType::CellIpc, b"cell-data", &mut wire)
            .expect("seal");
        wire[8] ^= 1; // session id
        assert!(b.open(&mut wire[..len]).is_err());
    }

    #[test]
    fn open_rejects_replayed_frame() {
        let (mut a, mut b) = test_session_pair();
        let mut wire = [0u8; 256];
        let len = a
            .seal(MessageType::Heartbeat, b"x", &mut wire)
            .expect("seal");
        let snapshot = wire[..len].to_vec();
        let mut first = snapshot.clone();
        b.open(&mut first).expect("first delivery");
        let mut replay = snapshot;
        assert_eq!(b.open(&mut replay), Err(CellNetError::ReplayRejected));
    }
}
