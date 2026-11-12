//! AWEOS Network Cell Service (`cellnetd`) — distributed cell fabric.
//!
//! `cellnetd` extends the kernel's local cell runtime (`aweos_kernel::cell`)
//! across machine boundaries. It provides:
//!
//! - signed cell service announcements for distributed discovery,
//! - mutually authenticated session establishment (Ed25519 identity,
//!   X25519 key agreement, HKDF-SHA256 key schedule),
//! - AEAD-protected remote cell IPC envelopes (ChaCha20-Poly1305) with a
//!   sliding anti-replay window,
//! - heartbeat-based health tracking, exponential-backoff reconnect and
//!   snapshot directory synchronization for network-partition recovery.
//!
//! All cryptography is delegated to `awe-securityd` (the Security Cell);
//! this crate owns protocol state machines only. Everything is fixed
//! capacity and allocation-free, matching the CellKernel service style.
//!
//! This is a transport-neutral contract layer: it does not open sockets and
//! does not assume the underlying network is reliable, private or
//! low-latency. Every remote message is authenticated; data-plane messages
//! are additionally encrypted. See `docs/CELLNET_ARCHITECTURE.md`.

#![no_std]

#[cfg(test)]
extern crate std;

mod directory;
mod fabric;
mod session;
mod wire;

pub use directory::{CellOffer, Directory, RemoteCell, RemoteHealth};
pub use fabric::{
    CellFabric, CellIpcRequest, FabricConfig, FabricEvent, FabricStats, IncomingIpc, PeerPhase,
    Transport, TransportError,
};
pub use session::{NodeIdentity, NodeRecord, Session};
pub use wire::{
    ANNOUNCE_LEN, ENVELOPE_HEADER_LEN, HELLO_ACK_LEN, HELLO_ACK_SIGNED_LEN, HELLO_LEN,
    HELLO_SIGNED_LEN, HelloAckFrame, HelloFrame, MAX_ENVELOPE_PAYLOAD, MAX_ENVELOPE_WIRE,
    MessageType, OFFER_WIRE_LEN, decode_hello, decode_hello_ack, encode_hello, encode_hello_ack,
};

/// ABI version of the cellnet fabric contract.
pub const CELLNET_ABI_MAJOR: u16 = 0;
pub const CELLNET_ABI_MINOR: u16 = 1;

/// Wire protocol version. Decoders reject anything newer.
pub const WIRE_VERSION: u16 = 1;
/// Wire magic, ASCII "AWCN" little-endian.
pub const WIRE_MAGIC: u32 = u32::from_le_bytes(*b"AWCN");

/// Maximum simultaneously discovered nodes.
pub const MAX_NODES: usize = 16;
/// Maximum simultaneous session peers.
pub const MAX_PEERS: usize = 8;
/// Maximum remote cells tracked across all nodes.
pub const MAX_REMOTE_CELLS: usize = 32;
/// Maximum offers published by this node.
pub const MAX_LOCAL_OFFERS: usize = 8;
/// Maximum offers carried by one announcement.
pub const MAX_OFFERS_PER_ANNOUNCE: usize = 8;
/// Maximum trusted public keys in pinned trust mode.
pub const MAX_TRUSTED_KEYS: usize = 8;
/// Maximum payload of a single remote cell IPC message.
pub const MAX_IPC_PAYLOAD: usize = 512;
/// Bounded queue of decrypted inbound IPC awaiting local delivery.
pub const INCOMING_QUEUE: usize = 16;
/// Bounded queue of fabric events awaiting the supervisor.
pub const EVENT_QUEUE: usize = 16;
/// Anti-replay window width in sequence numbers.
pub const REPLAY_WINDOW: u64 = 64;
/// Hello retransmission attempts before the peer is dropped.
pub const MAX_HANDSHAKE_ATTEMPTS: u8 = 4;

/// Cell-kind discriminants mirroring `aweos_kernel::cell::CellKind`.
///
/// The values are ABI-stable and intentionally duplicated here so the
/// service layer does not depend on the kernel crate.
pub mod cell_kind {
    pub const KERNEL_CORE: u8 = 0;
    pub const MEMORY: u8 = 1;
    pub const PROCESS: u8 = 2;
    pub const SCHEDULER: u8 = 3;
    pub const DRIVER: u8 = 4;
    pub const STORAGE: u8 = 5;
    pub const NETWORK: u8 = 6;
    pub const SECURITY: u8 = 7;
    pub const SERVICE: u8 = 8;
    pub const USER_INTERFACE: u8 = 9;
    pub const APPLICATION: u8 = 10;
    pub const VIRTUALIZATION: u8 = 11;
    pub const COUNT: u8 = 12;
}

/// Errors reported by the cellnet fabric.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellNetError {
    /// Peer key is not permitted by the configured trust policy.
    Untrusted,
    /// Ed25519 signature verification failed.
    BadSignature,
    /// `node_id` does not match the claimed public key.
    BadNodeBinding,
    /// Wire version is newer than this implementation.
    UnsupportedVersion,
    /// Frame failed structural validation.
    Malformed,
    /// Node or peer is not known.
    UnknownNode,
    /// No free slot in the peer table.
    PeerTableFull,
    /// Node table is full.
    NodeTableFull,
    /// Operation requires an established session.
    NotSecured,
    /// Sequence number falls inside the anti-replay window as a duplicate.
    ReplayRejected,
    /// AEAD authentication failed.
    DecryptFailed,
    /// X25519 peer key is low-order (non-contributory).
    LowOrderKey,
    /// Destination cell is not present in the remote directory.
    UnknownRemoteCell,
    /// Destination cell is not published by this node.
    DestinationNotPublished,
    /// Claimed source cell is not in the authenticated publisher's directory.
    SourceNotPublished,
    /// Directory cannot hold more remote cells.
    DirectoryFull,
    /// Local offer table is full.
    OfferTableFull,
    /// Publish policy rejected the offer.
    PolicyDenied,
    /// A reconnect attempt is still in backoff.
    BackoffActive,
    /// Underlying transport failed.
    Transport(TransportError),
}

/// Trust policy applied to discovered nodes and handshake initiators.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrustPolicy {
    /// Accept any well-formed, self-consistent identity on first contact
    /// (trust on first use). Suitable only for development meshes.
    Open,
    /// Accept only public keys previously registered with
    /// `CellFabric::add_trusted_key`.
    Pinned,
}

/// Which cells this node is allowed to publish to the mesh.
///
/// An offer is publishable only when its kind bit is present in
/// `kind_mask` **and** every capability it requires is inside `cap_mask`.
/// The default policy publishes nothing (deny-all).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PublishPolicy {
    pub kind_mask: u16,
    pub cap_mask: u64,
}

impl PublishPolicy {
    /// Deny-all policy: nothing leaves this node.
    pub const fn deny_all() -> Self {
        Self {
            kind_mask: 0,
            cap_mask: 0,
        }
    }

    pub const fn new(kind_mask: u16, cap_mask: u64) -> Self {
        Self {
            kind_mask,
            cap_mask,
        }
    }

    pub fn permits(&self, offer: &CellOffer) -> bool {
        if offer.kind >= cell_kind::COUNT {
            return false;
        }
        if self.kind_mask & (1u16 << offer.kind) == 0 {
            return false;
        }
        offer.required_caps & !self.cap_mask == 0
    }
}

/// Derive the canonical node id for a public key.
///
/// `node_id = first 8 bytes of SHA-256(public_key)` little-endian. The
/// binding is enforced on every signed frame, so a node cannot claim an id
/// that does not match its key.
pub fn node_id_for_pubkey(public_key: &[u8; 32]) -> u64 {
    let hash = awe_securityd::sha256(public_key);
    u64::from_le_bytes(hash[..8].try_into().expect("8 bytes"))
}

/// Overwrite key material in place; used when sessions are torn down.
pub(crate) fn scrub(buffer: &mut [u8]) {
    for byte in buffer.iter_mut() {
        // SAFETY: `byte` is a valid, aligned, writable reference into `buffer`.
        unsafe { core::ptr::write_volatile(byte, 0) };
    }
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
}
