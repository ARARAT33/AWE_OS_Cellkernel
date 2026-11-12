//! Fixed-size wire encoding for the cellnet protocol.
//!
//! All integers are little-endian. Signed frames are always verified over
//! the exact received byte range; nothing is re-encoded before signature
//! checks, which avoids canonicalization bugs.

use crate::directory::CellOffer;
use crate::{CellNetError, MAX_OFFERS_PER_ANNOUNCE, WIRE_MAGIC, WIRE_VERSION};

/// Message types on the cellnet wire.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageType {
    /// Plaintext, Ed25519-signed handshake initiation.
    Hello = 1,
    /// Plaintext, Ed25519-signed handshake response echoing the initiator nonce.
    HelloAck = 2,
    /// Encrypted key-confirmation finish message.
    Finished = 3,
    /// Plaintext, Ed25519-signed service announcement (discovery plane).
    Announce = 4,
    /// Encrypted liveness message.
    Heartbeat = 5,
    /// Encrypted remote cell IPC.
    CellIpc = 7,
    /// Encrypted authoritative directory snapshot (anti-entropy).
    DirectorySync = 9,
}

impl MessageType {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Hello),
            2 => Some(Self::HelloAck),
            3 => Some(Self::Finished),
            4 => Some(Self::Announce),
            5 => Some(Self::Heartbeat),
            7 => Some(Self::CellIpc),
            9 => Some(Self::DirectorySync),
            _ => None,
        }
    }
}

/// Signed body length of a Hello frame (signature excluded).
pub const HELLO_SIGNED_LEN: usize = 128;
/// Total wire length of a Hello frame.
pub const HELLO_LEN: usize = HELLO_SIGNED_LEN + 64;
/// Signed body length of a HelloAck frame.
pub const HELLO_ACK_SIGNED_LEN: usize = 152;
/// Total wire length of a HelloAck frame.
pub const HELLO_ACK_LEN: usize = HELLO_ACK_SIGNED_LEN + 64;
/// Signed body length of an Announce frame.
pub const ANNOUNCE_SIGNED_LEN: usize = 72 + MAX_OFFERS_PER_ANNOUNCE * OFFER_WIRE_LEN;
/// Total wire length of an Announce frame.
pub const ANNOUNCE_LEN: usize = ANNOUNCE_SIGNED_LEN + 64;
/// Wire length of one encoded offer.
pub const OFFER_WIRE_LEN: usize = 48;
/// Length of the authenticated envelope header (also the AEAD AAD).
pub const ENVELOPE_HEADER_LEN: usize = 32;
/// Maximum payload carried by an encrypted envelope.
pub const MAX_ENVELOPE_PAYLOAD: usize = 576;
/// Maximum total envelope wire size: header + payload + tag.
pub const MAX_ENVELOPE_WIRE: usize = ENVELOPE_HEADER_LEN + MAX_ENVELOPE_PAYLOAD + 16;
/// Payload length of a Finished message (HMAC-SHA256 transcript confirmation).
pub const FINISHED_PAYLOAD_LEN: usize = 32;
/// Payload length of a Heartbeat message (sender tick).
pub const HEARTBEAT_PAYLOAD_LEN: usize = 8;
/// Header prepended to every remote cell IPC payload.
pub const IPC_HEADER_LEN: usize = 24;

pub fn write_u16(out: &mut [u8], offset: usize, value: u16) {
    out[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

pub fn write_u64(out: &mut [u8], offset: usize, value: u64) {
    out[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

pub fn read_u16(input: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(input[offset..offset + 2].try_into().expect("2 bytes"))
}

pub fn read_u64(input: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(input[offset..offset + 8].try_into().expect("8 bytes"))
}

/// Encode one offer into 48 bytes at `out[offset..offset + 48]`.
pub fn encode_offer(offer: &CellOffer, out: &mut [u8], offset: usize) {
    write_u64(out, offset, offer.cell_id);
    out[offset + 8] = offer.kind;
    out[offset + 9] = offer.state;
    write_u16(out, offset + 10, offer.abi_major);
    write_u16(out, offset + 12, offer.abi_minor);
    write_u16(out, offset + 14, 0);
    write_u64(out, offset + 16, offer.required_caps);
    write_u64(out, offset + 24, offer.interface_hash);
    write_u64(out, offset + 32, offer.ttl_ticks);
    write_u64(out, offset + 40, 0);
}

/// Decode one offer from 48 bytes at `input[offset..offset + 48]`.
pub fn decode_offer(input: &[u8], offset: usize) -> CellOffer {
    CellOffer {
        cell_id: read_u64(input, offset),
        kind: input[offset + 8],
        state: input[offset + 9],
        abi_major: read_u16(input, offset + 10),
        abi_minor: read_u16(input, offset + 12),
        required_caps: read_u64(input, offset + 16),
        interface_hash: read_u64(input, offset + 24),
        ttl_ticks: read_u64(input, offset + 32),
    }
}

fn write_frame_head(out: &mut [u8], message_type: MessageType) {
    out[0..4].copy_from_slice(&WIRE_MAGIC.to_le_bytes());
    write_u16(out, 4, WIRE_VERSION);
    out[6] = message_type as u8;
    out[7] = 0;
}

/// Validate magic, version and minimum framing shared by every message.
pub fn check_frame_head(input: &[u8], min_len: usize) -> Result<MessageType, CellNetError> {
    if input.len() < min_len || input.len() < 8 {
        return Err(CellNetError::Malformed);
    }
    if u32::from_le_bytes(input[0..4].try_into().expect("4 bytes")) != WIRE_MAGIC {
        return Err(CellNetError::Malformed);
    }
    let version = read_u16(input, 4);
    if version > WIRE_VERSION {
        return Err(CellNetError::UnsupportedVersion);
    }
    MessageType::from_u8(input[6]).ok_or(CellNetError::Malformed)
}

/// Decoded plaintext Hello frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HelloFrame {
    pub node_pubkey: [u8; 32],
    pub node_id: u64,
    pub x25519_public: [u8; 32],
    pub nonce: [u8; 32],
    pub endpoint: u64,
    pub tick: u64,
    pub signature: [u8; 64],
}

pub fn encode_hello(frame: &HelloFrame, out: &mut [u8; HELLO_LEN]) {
    write_frame_head(out, MessageType::Hello);
    out[8..40].copy_from_slice(&frame.node_pubkey);
    write_u64(out, 40, frame.node_id);
    out[48..80].copy_from_slice(&frame.x25519_public);
    out[80..112].copy_from_slice(&frame.nonce);
    write_u64(out, 112, frame.endpoint);
    write_u64(out, 120, frame.tick);
    out[128..192].copy_from_slice(&frame.signature);
}

pub fn decode_hello(input: &[u8]) -> Result<HelloFrame, CellNetError> {
    check_frame_head(input, HELLO_LEN)?;
    if input.len() != HELLO_LEN {
        return Err(CellNetError::Malformed);
    }
    if input.len() != HELLO_LEN || MessageType::from_u8(input[6]) != Some(MessageType::Hello) {
        return Err(CellNetError::Malformed);
    }
    Ok(HelloFrame {
        node_pubkey: input[8..40].try_into().expect("32 bytes"),
        node_id: read_u64(input, 40),
        x25519_public: input[48..80].try_into().expect("32 bytes"),
        nonce: input[80..112].try_into().expect("32 bytes"),
        endpoint: read_u64(input, 112),
        tick: read_u64(input, 120),
        signature: input[128..192].try_into().expect("64 bytes"),
    })
}

/// Decoded plaintext HelloAck frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HelloAckFrame {
    pub node_pubkey: [u8; 32],
    pub node_id: u64,
    pub x25519_public: [u8; 32],
    pub nonce: [u8; 32],
    pub initiator_nonce: [u8; 32],
    pub tick: u64,
    pub signature: [u8; 64],
}

pub fn encode_hello_ack(frame: &HelloAckFrame, out: &mut [u8; HELLO_ACK_LEN]) {
    write_frame_head(out, MessageType::HelloAck);
    out[8..40].copy_from_slice(&frame.node_pubkey);
    write_u64(out, 40, frame.node_id);
    out[48..80].copy_from_slice(&frame.x25519_public);
    out[80..112].copy_from_slice(&frame.nonce);
    out[112..144].copy_from_slice(&frame.initiator_nonce);
    write_u64(out, 144, frame.tick);
    out[152..216].copy_from_slice(&frame.signature);
}

pub fn decode_hello_ack(input: &[u8]) -> Result<HelloAckFrame, CellNetError> {
    check_frame_head(input, HELLO_ACK_LEN)?;
    if input.len() != HELLO_ACK_LEN || MessageType::from_u8(input[6]) != Some(MessageType::HelloAck)
    {
        return Err(CellNetError::Malformed);
    }
    Ok(HelloAckFrame {
        node_pubkey: input[8..40].try_into().expect("32 bytes"),
        node_id: read_u64(input, 40),
        x25519_public: input[48..80].try_into().expect("32 bytes"),
        nonce: input[80..112].try_into().expect("32 bytes"),
        initiator_nonce: input[112..144].try_into().expect("32 bytes"),
        tick: read_u64(input, 144),
        signature: input[152..216].try_into().expect("64 bytes"),
    })
}

/// Decoded plaintext Announce frame.
#[derive(Clone, Copy, Debug)]
pub struct AnnounceFrame {
    pub node_pubkey: [u8; 32],
    pub node_id: u64,
    pub sequence: u64,
    pub tick: u64,
    pub offer_count: u8,
    pub offers: [CellOffer; MAX_OFFERS_PER_ANNOUNCE],
    pub signature: [u8; 64],
}

pub fn encode_announce(frame: &AnnounceFrame, out: &mut [u8; ANNOUNCE_LEN]) {
    write_frame_head(out, MessageType::Announce);
    out[8..40].copy_from_slice(&frame.node_pubkey);
    write_u64(out, 40, frame.node_id);
    write_u64(out, 48, frame.sequence);
    write_u64(out, 56, frame.tick);
    out[64] = frame.offer_count;
    out[65..72].fill(0);
    for (index, offer) in frame.offers.iter().enumerate() {
        encode_offer(offer, out, 72 + index * OFFER_WIRE_LEN);
    }
    out[ANNOUNCE_SIGNED_LEN..ANNOUNCE_LEN].copy_from_slice(&frame.signature);
}

pub fn decode_announce(input: &[u8]) -> Result<AnnounceFrame, CellNetError> {
    check_frame_head(input, ANNOUNCE_LEN)?;
    if input.len() != ANNOUNCE_LEN || MessageType::from_u8(input[6]) != Some(MessageType::Announce)
    {
        return Err(CellNetError::Malformed);
    }
    let offer_count = input[64];
    if offer_count as usize > MAX_OFFERS_PER_ANNOUNCE {
        return Err(CellNetError::Malformed);
    }
    let mut offers = [CellOffer::EMPTY; MAX_OFFERS_PER_ANNOUNCE];
    for (index, slot) in offers.iter_mut().enumerate() {
        *slot = decode_offer(input, 72 + index * OFFER_WIRE_LEN);
    }
    Ok(AnnounceFrame {
        node_pubkey: input[8..40].try_into().expect("32 bytes"),
        node_id: read_u64(input, 40),
        sequence: read_u64(input, 48),
        tick: read_u64(input, 56),
        offer_count,
        offers,
        signature: input[ANNOUNCE_SIGNED_LEN..ANNOUNCE_LEN]
            .try_into()
            .expect("64 bytes"),
    })
}

const _: () = {
    // Compile-time layout guards for the fixed wire format.
    assert!(ANNOUNCE_SIGNED_LEN == 456);
    assert!(ANNOUNCE_LEN == 520);
    assert!(HELLO_LEN == 192);
    assert!(HELLO_ACK_LEN == 216);
};

/// Parsed envelope header. The payload and tag follow in the same buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnvelopeHeader {
    pub message_type: MessageType,
    pub session_id: u64,
    pub sequence: u64,
    pub payload_len: usize,
}

pub fn encode_envelope_header(header: &EnvelopeHeader, out: &mut [u8]) -> Result<(), CellNetError> {
    if out.len() < ENVELOPE_HEADER_LEN || header.payload_len > MAX_ENVELOPE_PAYLOAD {
        return Err(CellNetError::Malformed);
    }
    write_frame_head(out, header.message_type);
    write_u64(out, 8, header.session_id);
    write_u64(out, 16, header.sequence);
    write_u16(out, 24, header.payload_len as u16);
    out[26..32].fill(0);
    Ok(())
}

pub fn decode_envelope_header(input: &[u8]) -> Result<EnvelopeHeader, CellNetError> {
    check_frame_head(input, ENVELOPE_HEADER_LEN + 16)?;
    let payload_len = read_u16(input, 24) as usize;
    if payload_len > MAX_ENVELOPE_PAYLOAD {
        return Err(CellNetError::Malformed);
    }
    if input.len() != ENVELOPE_HEADER_LEN + payload_len + 16 {
        return Err(CellNetError::Malformed);
    }
    Ok(EnvelopeHeader {
        message_type: MessageType::from_u8(input[6]).ok_or(CellNetError::Malformed)?,
        session_id: read_u64(input, 8),
        sequence: read_u64(input, 16),
        payload_len,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_offer() -> CellOffer {
        CellOffer {
            cell_id: 0x1122_3344_5566_7788,
            kind: crate::cell_kind::STORAGE,
            state: 3,
            abi_major: 1,
            abi_minor: 2,
            required_caps: 0xdead_beef,
            interface_hash: 0x0102_0304_0506_0708,
            ttl_ticks: 5000,
        }
    }

    #[test]
    fn offer_roundtrip() {
        let offer = sample_offer();
        let mut buf = [0u8; OFFER_WIRE_LEN];
        encode_offer(&offer, &mut buf, 0);
        assert_eq!(decode_offer(&buf, 0), offer);
    }

    #[test]
    fn announce_roundtrip_and_size() {
        let frame = AnnounceFrame {
            node_pubkey: [0x11; 32],
            node_id: 42,
            sequence: 7,
            tick: 900,
            offer_count: 2,
            offers: {
                let mut offers = [CellOffer::EMPTY; MAX_OFFERS_PER_ANNOUNCE];
                offers[0] = sample_offer();
                offers[1] = sample_offer();
                offers
            },
            signature: [0xAB; 64],
        };
        let mut buf = [0u8; ANNOUNCE_LEN];
        encode_announce(&frame, &mut buf);
        assert_eq!(ANNOUNCE_LEN, 520);
        let decoded = decode_announce(&buf).expect("decode");
        assert_eq!(decoded.node_id, 42);
        assert_eq!(decoded.sequence, 7);
        assert_eq!(decoded.offer_count, 2);
        assert_eq!(decoded.offers[0], sample_offer());
        assert_eq!(decoded.signature, [0xAB; 64]);
    }

    #[test]
    fn rejects_bad_magic_and_version() {
        let mut buf = [0u8; HELLO_LEN];
        assert!(decode_hello(&buf).is_err());
        buf[0..4].copy_from_slice(&WIRE_MAGIC.to_le_bytes());
        write_u16(&mut buf, 4, WIRE_VERSION + 1);
        assert_eq!(decode_hello(&buf), Err(CellNetError::UnsupportedVersion));
    }
}
