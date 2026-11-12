//! The cell fabric: peer sessions, discovery announcements, remote cell
//! IPC and failure recovery, driven by explicit ticks and a caller-owned
//! transport.

use crate::directory::{CellOffer, Directory, RemoteCell, RemoteHealth};
use crate::session::{NodeIdentity, NodeRecord, Session, finished_payload};
use crate::wire::{
    ANNOUNCE_SIGNED_LEN, AnnounceFrame, HELLO_ACK_SIGNED_LEN, HELLO_LEN, HELLO_SIGNED_LEN, HelloAckFrame,
    HelloFrame, IPC_HEADER_LEN, MAX_ENVELOPE_PAYLOAD, MessageType, decode_announce, decode_hello,
    decode_hello_ack, encode_announce, encode_hello, encode_hello_ack,
};
use crate::{
    CELLNET_ABI_MAJOR, CELLNET_ABI_MINOR, CellNetError, EVENT_QUEUE, INCOMING_QUEUE,
    MAX_ENVELOPE_WIRE, MAX_IPC_PAYLOAD, MAX_LOCAL_OFFERS, MAX_NODES, MAX_OFFERS_PER_ANNOUNCE,
    MAX_PEERS, MAX_TRUSTED_KEYS, PublishPolicy, TrustPolicy, cell_kind, node_id_for_pubkey, scrub,
};

/// Opaque transport address used by the caller's transport layer
/// (for example a packed IPv4 address + UDP port in `netd`).
pub type NodeAddress = u64;

/// Minimal datagram transport contract.
///
/// Implementations are owned by the caller (netd socket layer in
/// production, an in-memory link in tests). The fabric never blocks: a
/// transport that has nothing to receive returns `Ok(None)`.
pub trait Transport {
    fn send_to(&mut self, address: NodeAddress, data: &[u8]) -> Result<(), TransportError>;
    fn recv_from(
        &mut self,
        buffer: &mut [u8],
    ) -> Result<Option<(NodeAddress, usize)>, TransportError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportError {
    Unreachable,
    BufferTooSmall,
    Io,
}

/// Session/liveness phase of a peer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeerPhase {
    /// Hello sent, waiting for HelloAck.
    HandshakeInitiator,
    /// HelloAck sent, waiting for the initiator's Finished.
    AwaitingConfirm,
    /// Session established; heartbeats flowing.
    Secured,
    /// Heartbeats missed; session kept but health degraded.
    Suspect,
    /// No usable session. A backoff deadline may gate reconnection.
    Offline,
}

/// Events the fabric reports to its supervisor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FabricEvent {
    PeerSecured { node_id: u64 },
    PeerSuspect { node_id: u64 },
    PeerLost { node_id: u64 },
    DirectoryChanged { node_id: u64 },
    IpcDelivered { from_node: u64, cell_id: u64 },
}

/// Decrypted inbound cell IPC awaiting local delivery.
#[derive(Clone, Copy, Debug)]
pub struct IncomingIpc {
    pub from_node: u64,
    pub from_cell: u64,
    pub to_cell: u64,
    pub channel: u64,
    pub payload_len: usize,
    pub payload: [u8; MAX_IPC_PAYLOAD],
}

/// Fabric counters for diagnostics and audit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FabricStats {
    pub frames_malformed: u64,
    pub frames_bad_signature: u64,
    pub frames_untrusted: u64,
    pub frames_decrypt_failed: u64,
    pub frames_replay_rejected: u64,
    pub ipc_delivered: u64,
    pub ipc_dropped: u64,
    pub sessions_established: u64,
    pub sessions_lost: u64,
}

/// Runtime tuning, in caller-defined ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FabricConfig {
    /// Interval between heartbeats (and directory-sync retries) to a
    /// secured peer.
    pub heartbeat_interval: u64,
    /// Silence after which a secured peer becomes suspect.
    pub suspect_after: u64,
    /// Silence after which the session is dropped.
    pub offline_after: u64,
    /// Handshake inactivity after which the attempt is abandoned. The
    /// supervisor re-dials with fresh randomness.
    pub handshake_timeout: u64,
    /// Base reconnect backoff; doubles per failure up to `backoff_max`.
    pub backoff_base: u64,
    pub backoff_max: u64,
    /// Upper bound applied to locally published offer TTLs.
    pub offer_ttl: u64,
    pub trust: TrustPolicy,
    pub publish_policy: PublishPolicy,
    /// Whether `reconnect_due` reports persistent peers after backoff.
    pub auto_reconnect: bool,
}

impl FabricConfig {
    pub const fn development() -> Self {
        Self {
            heartbeat_interval: 100,
            suspect_after: 350,
            offline_after: 800,
            handshake_timeout: 200,
            backoff_base: 100,
            backoff_max: 1600,
            offer_ttl: 3000,
            trust: TrustPolicy::Open,
            publish_policy: PublishPolicy::deny_all(),
            auto_reconnect: true,
        }
    }
}

/// Key material and transcript state for an in-flight handshake.
struct PendingHandshake {
    nonce: [u8; 32],
    x25519_secret: [u8; 32],
    /// Exact signed byte ranges needed to reconstruct the transcript.
    hello_signed: [u8; HELLO_SIGNED_LEN],
    hello_ack_signed: [u8; HELLO_ACK_SIGNED_LEN],
    last_attempt_tick: u64,
}

impl Drop for PendingHandshake {
    fn drop(&mut self) {
        scrub(&mut self.x25519_secret);
        scrub(&mut self.nonce);
    }
}

struct Peer {
    node_id: u64,
    address: NodeAddress,
    phase: PeerPhase,
    persistent: bool,
    session: Option<Session>,
    handshake: Option<PendingHandshake>,
    /// Exact Hello bytes accepted by inspect_hello and awaiting supervisor approval.
    pending_hello: Option<[u8; HELLO_LEN]>,
    /// Transcript hash of the completed handshake, kept until the peer's
    /// Finished message has been verified.
    pending_transcript: Option<[u8; 32]>,
    /// Whether a directory snapshot is still owed to this peer.
    needs_sync: bool,
    last_rx_tick: u64,
    last_tx_tick: u64,
    backoff_until: u64,
    backoff_attempts: u8,
}

impl Peer {
    fn new(node_id: u64, address: NodeAddress, phase: PeerPhase, persistent: bool) -> Self {
        Self {
            node_id,
            address,
            phase,
            persistent,
            session: None,
            handshake: None,
            pending_hello: None,
            pending_transcript: None,
            needs_sync: false,
            last_rx_tick: 0,
            last_tx_tick: 0,
            backoff_until: 0,
            backoff_attempts: 0,
        }
    }
}

/// Addressing for one remote cell IPC call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellIpcRequest {
    pub to_node: u64,
    pub to_cell: u64,
    pub channel: u64,
    pub from_cell: u64,
}

/// The distributed cell fabric for one node.
pub struct CellFabric {
    identity: NodeIdentity,
    config: FabricConfig,
    nodes: [Option<NodeRecord>; MAX_NODES],
    peers: [Option<Peer>; MAX_PEERS],
    directory: Directory,
    local_offers: [Option<CellOffer>; MAX_LOCAL_OFFERS],
    announce_sequence: u64,
    trusted_keys: [Option<[u8; 32]>; MAX_TRUSTED_KEYS],
    incoming: [Option<IncomingIpc>; INCOMING_QUEUE],
    events: [Option<FabricEvent>; EVENT_QUEUE],
    stats: FabricStats,
}

impl CellFabric {
    pub fn new(seed: [u8; 32], config: FabricConfig) -> Self {
        Self {
            identity: NodeIdentity::from_seed(&seed),
            config,
            nodes: [None; MAX_NODES],
            peers: core::array::from_fn(|_| None),
            directory: Directory::new(),
            local_offers: [None; MAX_LOCAL_OFFERS],
            announce_sequence: 0,
            trusted_keys: [None; MAX_TRUSTED_KEYS],
            incoming: [None; INCOMING_QUEUE],
            events: [None; EVENT_QUEUE],
            stats: FabricStats::default(),
        }
    }

    pub fn node_id(&self) -> u64 {
        self.identity.node_id
    }

    pub fn public_key(&self) -> [u8; 32] {
        self.identity.public_key
    }

    pub fn stats(&self) -> FabricStats {
        self.stats
    }

    pub fn directory(&self) -> &Directory {
        &self.directory
    }

    pub fn peer_phase(&self, node_id: u64) -> Option<PeerPhase> {
        self.peers
            .iter()
            .flatten()
            .find(|p| p.node_id == node_id)
            .map(|p| p.phase)
    }

    pub fn local_offers(&self) -> impl Iterator<Item = &CellOffer> {
        self.local_offers.iter().flatten()
    }

    pub fn abi_version(&self) -> (u16, u16) {
        (CELLNET_ABI_MAJOR, CELLNET_ABI_MINOR)
    }

    /// Register a pinned trust key (no effect in `TrustPolicy::Open`).
    pub fn add_trusted_key(&mut self, public_key: [u8; 32]) -> Result<(), CellNetError> {
        if self
            .trusted_keys
            .iter()
            .flatten()
            .any(|key| *key == public_key)
        {
            return Ok(());
        }
        let slot = self
            .trusted_keys
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(CellNetError::NodeTableFull)?;
        *slot = Some(public_key);
        Ok(())
    }

    fn is_trusted(&self, public_key: &[u8; 32]) -> bool {
        match self.config.trust {
            TrustPolicy::Open => true,
            TrustPolicy::Pinned => self.trusted_keys.iter().flatten().any(|k| k == public_key),
        }
    }

    fn push_event(&mut self, event: FabricEvent) {
        if let Some(slot) = self.events.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(event);
        }
    }

    pub fn next_event(&mut self) -> Option<FabricEvent> {
        let index = self.events.iter().position(Option::is_some)?;
        self.events[index].take()
    }

    // ------------------------------------------------------------------
    // Discovery plane
    // ------------------------------------------------------------------

    fn learn_node(
        &mut self,
        node_id: u64,
        public_key: [u8; 32],
        address: NodeAddress,
        announce_sequence: u64,
        now: u64,
    ) -> Result<(), CellNetError> {
        if let Some(record) = self
            .nodes
            .iter_mut()
            .flatten()
            .find(|n| n.node_id == node_id)
        {
            if record.public_key != public_key {
                // A node id cannot change keys mid-flight; that is spoofing.
                return Err(CellNetError::BadNodeBinding);
            }
            record.address = address;
            record.announce_sequence = record.announce_sequence.max(announce_sequence);
            record.last_seen_tick = now;
            return Ok(());
        }
        let slot = self
            .nodes
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(CellNetError::NodeTableFull)?;
        *slot = Some(NodeRecord {
            node_id,
            public_key,
            address,
            announce_sequence,
            last_seen_tick: now,
        });
        Ok(())
    }

    fn node(&self, node_id: u64) -> Option<NodeRecord> {
        self.nodes
            .iter()
            .flatten()
            .find(|n| n.node_id == node_id)
            .copied()
    }

    /// Publish a local cell to the mesh, subject to the publish policy.
    pub fn publish_offer(&mut self, mut offer: CellOffer) -> Result<(), CellNetError> {
        if !offer.is_valid() {
            return Err(CellNetError::Malformed);
        }
        if !self.config.publish_policy.permits(&offer) {
            return Err(CellNetError::PolicyDenied);
        }
        if offer.ttl_ticks > self.config.offer_ttl {
            offer.ttl_ticks = self.config.offer_ttl;
        }
        if let Some(slot) = self
            .local_offers
            .iter_mut()
            .find(|entry| entry.is_some_and(|o| o.cell_id == offer.cell_id))
        {
            *slot = Some(offer);
        } else {
            let slot = self
                .local_offers
                .iter_mut()
                .find(|slot| slot.is_none())
                .ok_or(CellNetError::OfferTableFull)?;
            *slot = Some(offer);
        }
        self.announce_sequence = self.announce_sequence.saturating_add(1);
        self.mark_sync_debt();
        Ok(())
    }

    /// Stop publishing a cell. Removal propagates through the next
    /// announcement and the next DirectorySync snapshots.
    pub fn withdraw_offer(&mut self, cell_id: u64) -> bool {
        if let Some(slot) = self
            .local_offers
            .iter_mut()
            .find(|entry| entry.is_some_and(|o| o.cell_id == cell_id))
        {
            *slot = None;
            self.announce_sequence = self.announce_sequence.saturating_add(1);
            self.mark_sync_debt();
            return true;
        }
        false
    }

    /// Every secured peer is owed a fresh directory snapshot.
    fn mark_sync_debt(&mut self) {
        for peer in self.peers.iter_mut().flatten() {
            if peer.phase == PeerPhase::Secured {
                peer.needs_sync = true;
            }
        }
    }

    fn is_published(&self, cell_id: u64) -> bool {
        self.local_offers
            .iter()
            .flatten()
            .any(|o| o.cell_id == cell_id)
    }

    /// Build a signed discovery announcement for broadcast.
    pub fn make_announcement(&mut self, now: u64) -> [u8; crate::ANNOUNCE_LEN] {
        let mut offers = [CellOffer::EMPTY; MAX_OFFERS_PER_ANNOUNCE];
        let mut count = 0usize;
        for offer in self.local_offers.iter().flatten() {
            if count >= MAX_OFFERS_PER_ANNOUNCE {
                break;
            }
            offers[count] = *offer;
            count += 1;
        }
        let mut frame = AnnounceFrame {
            node_pubkey: self.identity.public_key,
            node_id: self.identity.node_id,
            sequence: self.announce_sequence,
            tick: now,
            offer_count: count as u8,
            offers,
            signature: [0u8; 64],
        };
        let mut wire = [0u8; crate::ANNOUNCE_LEN];
        encode_announce(&frame, &mut wire);
        frame.signature = self.identity.sign(&wire[..ANNOUNCE_SIGNED_LEN]);
        encode_announce(&frame, &mut wire);
        wire
    }

    fn handle_announcement(
        &mut self,
        source: NodeAddress,
        bytes: &[u8],
        now: u64,
    ) -> Result<(), CellNetError> {
        let frame = decode_announce(bytes)?;
        if frame.node_id != node_id_for_pubkey(&frame.node_pubkey) {
            self.stats.frames_bad_signature += 1;
            return Err(CellNetError::BadNodeBinding);
        }
        if frame.node_id == self.identity.node_id {
            return Ok(()); // our own broadcast looped back
        }
        if !self.is_trusted(&frame.node_pubkey) {
            self.stats.frames_untrusted += 1;
            return Err(CellNetError::Untrusted);
        }
        if !awe_securityd::ed25519_verify(
            &frame.node_pubkey,
            &bytes[..ANNOUNCE_SIGNED_LEN],
            &frame.signature,
        ) {
            self.stats.frames_bad_signature += 1;
            return Err(CellNetError::BadSignature);
        }
        if let Some(record) = self.node(frame.node_id)
            && frame.sequence < record.announce_sequence
        {
            return Ok(()); // stale replay of an old announcement
        }
        self.learn_node(
            frame.node_id,
            frame.node_pubkey,
            source,
            frame.sequence,
            now,
        )?;

        for offer in frame.offers.iter().take(frame.offer_count as usize) {
            if !offer.is_valid() {
                continue;
            }
            self.directory.upsert(RemoteCell {
                node_id: frame.node_id,
                offer: *offer,
                sequence: frame.sequence,
                last_seen_tick: now,
                expires_tick: now.saturating_add(offer.ttl_ticks),
                health: match self.peer_phase(frame.node_id) {
                    Some(PeerPhase::Secured) => RemoteHealth::Reachable,
                    Some(PeerPhase::Suspect) => RemoteHealth::Suspect,
                    _ => RemoteHealth::Unreachable,
                },
            })?;
        }
        self.push_event(FabricEvent::DirectoryChanged {
            node_id: frame.node_id,
        });
        Ok(())
    }

    // ------------------------------------------------------------------
    // Session establishment
    // ------------------------------------------------------------------

    /// Initiate a session with a discovered node. The caller supplies the
    /// per-attempt randomness; nothing in this crate depends on a global
    /// RNG, which keeps no_std targets deterministic and testable.
    pub fn connect(
        &mut self,
        node_id: u64,
        nonce: [u8; 32],
        x25519_secret: [u8; 32],
        now: u64,
        transport: &mut impl Transport,
    ) -> Result<(), CellNetError> {
        let record = self.node(node_id).ok_or(CellNetError::UnknownNode)?;

        let index = if let Some(index) = self.peer_index(node_id) {
            let peer = self.peers[index].as_ref().expect("peer");
            match peer.phase {
                PeerPhase::Offline if now >= peer.backoff_until => index,
                PeerPhase::Offline => return Err(CellNetError::BackoffActive),
                _ => return Ok(()), // handshake in flight or already secured
            }
        } else {
            let slot = self
                .peers
                .iter()
                .position(Option::is_none)
                .ok_or(CellNetError::PeerTableFull)?;
            self.peers[slot] = Some(Peer::new(
                node_id,
                record.address,
                PeerPhase::HandshakeInitiator,
                true,
            ));
            slot
        };

        let x25519_public = awe_securityd::x25519_public_from_secret(&x25519_secret);
        let mut frame = HelloFrame {
            node_pubkey: self.identity.public_key,
            node_id: self.identity.node_id,
            x25519_public,
            nonce,
            endpoint: 0,
            tick: now,
            signature: [0u8; 64],
        };
        let mut wire = [0u8; crate::HELLO_LEN];
        encode_hello(&frame, &mut wire);
        frame.signature = self.identity.sign(&wire[..HELLO_SIGNED_LEN]);
        encode_hello(&frame, &mut wire);

        let peer = self.peers[index].as_mut().expect("peer");
        transport
            .send_to(peer.address, &wire)
            .map_err(CellNetError::Transport)?;

        let mut hello_signed = [0u8; HELLO_SIGNED_LEN];
        hello_signed.copy_from_slice(&wire[..HELLO_SIGNED_LEN]);
        peer.handshake = Some(PendingHandshake {
            nonce,
            x25519_secret,
            hello_signed,
            hello_ack_signed: [0u8; HELLO_ACK_SIGNED_LEN],
            last_attempt_tick: now,
        });
        peer.session = None;
        peer.pending_transcript = None;
        peer.phase = PeerPhase::HandshakeInitiator;
        peer.last_tx_tick = now;
        Ok(())
    }

    fn transcript(hello_signed: &[u8], hello_ack_signed: &[u8]) -> [u8; 32] {
        let mut joined = [0u8; HELLO_SIGNED_LEN + HELLO_ACK_SIGNED_LEN];
        joined[..HELLO_SIGNED_LEN].copy_from_slice(hello_signed);
        joined[HELLO_SIGNED_LEN..].copy_from_slice(hello_ack_signed);
        awe_securityd::sha256(&joined)
    }

    /// Validate an incoming Hello and register the pending responder
    /// handshake. Returns the decoded frame so the supervisor can decide
    /// whether to call `accept_hello` with fresh randomness.
    pub fn inspect_hello(
        &mut self,
        source: NodeAddress,
        bytes: &[u8],
        now: u64,
    ) -> Result<HelloFrame, CellNetError> {
        let frame = decode_hello(bytes)?;
        if frame.node_id == self.identity.node_id {
            return Err(CellNetError::Malformed);
        }
        if frame.node_id != node_id_for_pubkey(&frame.node_pubkey) {
            self.stats.frames_bad_signature += 1;
            return Err(CellNetError::BadNodeBinding);
        }
        if !self.is_trusted(&frame.node_pubkey) {
            self.stats.frames_untrusted += 1;
            return Err(CellNetError::Untrusted);
        }
        if !awe_securityd::ed25519_verify(
            &frame.node_pubkey,
            &bytes[..HELLO_SIGNED_LEN],
            &frame.signature,
        ) {
            self.stats.frames_bad_signature += 1;
            return Err(CellNetError::BadSignature);
        }
        self.learn_node(frame.node_id, frame.node_pubkey, source, 0, now)?;

        if self.peer_index(frame.node_id).is_none() {
            let slot = self
                .peers
                .iter()
                .position(Option::is_none)
                .ok_or(CellNetError::PeerTableFull)?;
            self.peers[slot] = Some(Peer::new(
                frame.node_id,
                source,
                PeerPhase::AwaitingConfirm,
                true,
            ));
        }
        let peer = self.peers[self.peer_index(frame.node_id).expect("peer")]
            .as_mut()
            .expect("peer");
        peer.address = source;
        peer.last_rx_tick = now;
        let mut validated_hello = [0u8; HELLO_LEN];
        validated_hello.copy_from_slice(bytes);
        peer.pending_hello = Some(validated_hello);
        Ok(frame)
    }

    /// Answer a validated Hello with caller-supplied randomness, completing
    /// the responder side of the handshake: HelloAck + Finished are sent
    /// and the session enters `AwaitingConfirm`.
    pub fn accept_hello(
        &mut self,
        hello_bytes: &[u8],
        nonce: [u8; 32],
        x25519_secret: [u8; 32],
        now: u64,
        transport: &mut impl Transport,
    ) -> Result<(), CellNetError> {
        let frame = decode_hello(hello_bytes)?;
        if frame.node_id == self.identity.node_id
            || frame.node_id != node_id_for_pubkey(&frame.node_pubkey)
        {
            self.stats.frames_bad_signature += 1;
            return Err(CellNetError::BadNodeBinding);
        }
        if !self.is_trusted(&frame.node_pubkey) {
            self.stats.frames_untrusted += 1;
            return Err(CellNetError::Untrusted);
        }
        if !awe_securityd::ed25519_verify(
            &frame.node_pubkey,
            &hello_bytes[..HELLO_SIGNED_LEN],
            &frame.signature,
        ) {
            self.stats.frames_bad_signature += 1;
            return Err(CellNetError::BadSignature);
        }
        let record = self.node(frame.node_id).ok_or(CellNetError::UnknownNode)?;
        if record.public_key != frame.node_pubkey {
            self.stats.frames_bad_signature += 1;
            return Err(CellNetError::BadNodeBinding);
        }
        let index = self
            .peer_index(frame.node_id)
            .ok_or(CellNetError::UnknownNode)?;
        let pending = self.peers[index]
            .as_ref()
            .and_then(|peer| peer.pending_hello.as_ref())
            .ok_or(CellNetError::Malformed)?;
        if &pending[..] != hello_bytes {
            return Err(CellNetError::Malformed);
        }

        let x25519_public = awe_securityd::x25519_public_from_secret(&x25519_secret);
        let mut ack = HelloAckFrame {
            node_pubkey: self.identity.public_key,
            node_id: self.identity.node_id,
            x25519_public,
            nonce,
            initiator_nonce: frame.nonce,
            tick: now,
            signature: [0u8; 64],
        };
        let mut wire = [0u8; crate::HELLO_ACK_LEN];
        encode_hello_ack(&ack, &mut wire);
        ack.signature = self.identity.sign(&wire[..HELLO_ACK_SIGNED_LEN]);
        encode_hello_ack(&ack, &mut wire);

        // Derive before sending, so a failed derivation cannot leave a
        // half-open state.
        let transcript = Self::transcript(
            &hello_bytes[..HELLO_SIGNED_LEN],
            &wire[..HELLO_ACK_SIGNED_LEN],
        );
        let mut shared = awe_securityd::x25519_shared(&x25519_secret, &frame.x25519_public)
            .ok_or(CellNetError::LowOrderKey)?;
        let session = Session::derive(&shared, &transcript, &frame.nonce, &nonce, false)
            .ok_or(CellNetError::Malformed)?;
        scrub(&mut shared);

        let peer = self.peers[index].as_mut().expect("peer");
        transport
            .send_to(peer.address, &wire)
            .map_err(CellNetError::Transport)?;

        let mut hello_signed = [0u8; HELLO_SIGNED_LEN];
        hello_signed.copy_from_slice(&hello_bytes[..HELLO_SIGNED_LEN]);
        let mut hello_ack_signed = [0u8; HELLO_ACK_SIGNED_LEN];
        hello_ack_signed.copy_from_slice(&wire[..HELLO_ACK_SIGNED_LEN]);
        peer.handshake = Some(PendingHandshake {
            nonce,
            x25519_secret,
            hello_signed,
            hello_ack_signed,
            last_attempt_tick: now,
        });
        peer.pending_hello = None;
        peer.session = Some(session);
        peer.pending_transcript = Some(transcript);
        peer.phase = PeerPhase::AwaitingConfirm;
        peer.last_tx_tick = now;
        self.send_finished(index, now, transport)?;
        Ok(())
    }

    fn handle_hello_ack(
        &mut self,
        source: NodeAddress,
        bytes: &[u8],
        now: u64,
        transport: &mut impl Transport,
    ) -> Result<(), CellNetError> {
        let frame = decode_hello_ack(bytes)?;
        let index = self
            .peer_index(frame.node_id)
            .ok_or(CellNetError::UnknownNode)?;

        let (transcript, session) = {
            let peer = self.peers[index].as_ref().expect("peer");
            if peer.phase != PeerPhase::HandshakeInitiator {
                return Ok(());
            }
            let handshake = peer.handshake.as_ref().ok_or(CellNetError::Malformed)?;
            if frame.initiator_nonce != handshake.nonce {
                self.stats.frames_bad_signature += 1;
                return Err(CellNetError::BadSignature);
            }
            if frame.node_id != node_id_for_pubkey(&frame.node_pubkey) {
                self.stats.frames_bad_signature += 1;
                return Err(CellNetError::BadNodeBinding);
            }
            if let Some(record) = self.node(frame.node_id)
                && record.public_key != frame.node_pubkey
            {
                // The responder must own the identity we discovered.
                self.stats.frames_bad_signature += 1;
                return Err(CellNetError::BadNodeBinding);
            }
            if !awe_securityd::ed25519_verify(
                &frame.node_pubkey,
                &bytes[..HELLO_ACK_SIGNED_LEN],
                &frame.signature,
            ) {
                self.stats.frames_bad_signature += 1;
                return Err(CellNetError::BadSignature);
            }
            let transcript =
                Self::transcript(&handshake.hello_signed, &bytes[..HELLO_ACK_SIGNED_LEN]);
            let mut shared =
                awe_securityd::x25519_shared(&handshake.x25519_secret, &frame.x25519_public)
                    .ok_or(CellNetError::LowOrderKey)?;
            let session =
                Session::derive(&shared, &transcript, &handshake.nonce, &frame.nonce, true)
                    .ok_or(CellNetError::Malformed)?;
            scrub(&mut shared);
            (transcript, session)
        };

        let peer = self.peers[index].as_mut().expect("peer");
        peer.address = source;
        peer.last_rx_tick = now;
        peer.session = Some(session);
        peer.pending_transcript = Some(transcript);
        peer.phase = PeerPhase::AwaitingConfirm;
        if let Some(handshake) = peer.handshake.as_mut() {
            handshake
                .hello_ack_signed
                .copy_from_slice(&bytes[..HELLO_ACK_SIGNED_LEN]);
        }
        self.send_finished(index, now, transport)?;
        Ok(())
    }

    /// Seal and send the Finished key-confirmation using the peer's live
    /// session (advancing its transmit counter).
    fn send_finished(
        &mut self,
        index: usize,
        now: u64,
        transport: &mut impl Transport,
    ) -> Result<(), CellNetError> {
        let peer = self.peers[index].as_mut().expect("peer");
        let transcript = peer.pending_transcript.ok_or(CellNetError::Malformed)?;
        let session = peer.session.as_mut().ok_or(CellNetError::NotSecured)?;
        let payload = finished_payload(session, &transcript);
        let mut wire = [0u8; MAX_ENVELOPE_WIRE];
        let len = session
            .seal(MessageType::Finished, &payload, &mut wire)
            .map_err(|_| CellNetError::Malformed)?;
        transport
            .send_to(peer.address, &wire[..len])
            .map_err(CellNetError::Transport)?;
        peer.last_tx_tick = now;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Encrypted data plane
    // ------------------------------------------------------------------

    fn peer_index(&self, node_id: u64) -> Option<usize> {
        self.peers
            .iter()
            .position(|p| p.as_ref().is_some_and(|peer| peer.node_id == node_id))
    }

    fn peer_index_by_session(&self, session_id: u64) -> Option<usize> {
        self.peers.iter().position(|p| {
            p.as_ref()
                .and_then(|peer| peer.session.as_ref())
                .is_some_and(|session| session.session_id == session_id)
        })
    }

    fn handle_envelope(
        &mut self,
        _source: NodeAddress,
        buffer: &mut [u8],
        now: u64,
    ) -> Result<(), CellNetError> {
        if buffer.len() < crate::ENVELOPE_HEADER_LEN + 16 {
            self.stats.frames_malformed += 1;
            return Err(CellNetError::Malformed);
        }
        let session_id = u64::from_le_bytes(
            buffer[8..16]
                .try_into()
                .map_err(|_| CellNetError::Malformed)?,
        );
        let index = self
            .peer_index_by_session(session_id)
            .ok_or(CellNetError::UnknownNode)?;

        let (message_type, range) = {
            let peer = self.peers[index].as_mut().expect("peer");
            let session = peer.session.as_mut().ok_or(CellNetError::NotSecured)?;
            match session.open(buffer) {
                Ok(ok) => ok,
                Err(error) => {
                    match error {
                        CellNetError::DecryptFailed => self.stats.frames_decrypt_failed += 1,
                        CellNetError::ReplayRejected => self.stats.frames_replay_rejected += 1,
                        _ => self.stats.frames_malformed += 1,
                    }
                    return Err(error);
                }
            }
        };

        // Only an authenticated, non-replayed envelope counts as peer activity.
        self.peers[index].as_mut().expect("peer").last_rx_tick = now;

        match message_type {
            MessageType::Finished => self.handle_finished(index, &buffer[range], now),
            MessageType::Heartbeat => {
                let peer = self.peers[index].as_mut().expect("peer");
                if peer.phase == PeerPhase::Suspect {
                    peer.phase = PeerPhase::Secured;
                    self.directory
                        .set_node_health(peer.node_id, RemoteHealth::Reachable);
                }
                Ok(())
            }
            MessageType::CellIpc => self.handle_cell_ipc(index, &buffer[range]),
            MessageType::DirectorySync => self.handle_directory_sync(index, &buffer[range], now),
            _ => {
                self.stats.frames_malformed += 1;
                Err(CellNetError::Malformed)
            }
        }
    }

    fn handle_finished(
        &mut self,
        index: usize,
        payload: &[u8],
        now: u64,
    ) -> Result<(), CellNetError> {
        let tag: [u8; 32] = payload
            .get(..crate::wire::FINISHED_PAYLOAD_LEN)
            .and_then(|p| p.try_into().ok())
            .ok_or(CellNetError::Malformed)?;
        let peer = self.peers[index].as_mut().expect("peer");
        if peer.phase != PeerPhase::AwaitingConfirm {
            return Ok(());
        }
        let transcript = peer.pending_transcript.ok_or(CellNetError::Malformed)?;
        let verified = peer
            .session
            .as_ref()
            .ok_or(CellNetError::NotSecured)?
            .verify_confirm(&transcript, &tag);
        if !verified {
            self.stats.frames_decrypt_failed += 1;
            return Err(CellNetError::DecryptFailed);
        }
        peer.phase = PeerPhase::Secured;
        peer.needs_sync = true;
        peer.last_rx_tick = now;
        peer.backoff_attempts = 0;
        peer.handshake = None;
        peer.pending_transcript = None;
        let node_id = peer.node_id;
        self.stats.sessions_established += 1;
        self.directory
            .set_node_health(node_id, RemoteHealth::Reachable);
        self.push_event(FabricEvent::PeerSecured { node_id });
        Ok(())
    }

    fn handle_cell_ipc(&mut self, index: usize, payload: &[u8]) -> Result<(), CellNetError> {
        if payload.len() < IPC_HEADER_LEN || payload.len() > IPC_HEADER_LEN + MAX_IPC_PAYLOAD {
            self.stats.ipc_dropped += 1;
            return Err(CellNetError::Malformed);
        }
        let to_cell = u64::from_le_bytes(payload[0..8].try_into().expect("8 bytes"));
        let channel = u64::from_le_bytes(payload[8..16].try_into().expect("8 bytes"));
        let from_cell = u64::from_le_bytes(payload[16..24].try_into().expect("8 bytes"));
        let from_node = self.peers[index].as_ref().expect("peer").node_id;

        // A peer may identify only cells that it currently publishes.
        if self.directory.get(from_node, from_cell).is_none() {
            self.stats.ipc_dropped += 1;
            return Err(CellNetError::SourceNotPublished);
        }

        // Remote peers may only address cells this node actually publishes.
        if !self.is_published(to_cell) {
            self.stats.ipc_dropped += 1;
            return Err(CellNetError::DestinationNotPublished);
        }
        let body_len = payload.len() - IPC_HEADER_LEN;
        let mut message = IncomingIpc {
            from_node,
            from_cell,
            to_cell,
            channel,
            payload_len: body_len,
            payload: [0u8; MAX_IPC_PAYLOAD],
        };
        message.payload[..body_len].copy_from_slice(&payload[IPC_HEADER_LEN..]);
        let slot = self
            .incoming
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or_else(|| {
                self.stats.ipc_dropped += 1;
                CellNetError::Malformed
            })?;
        *slot = Some(message);
        self.stats.ipc_delivered += 1;
        self.push_event(FabricEvent::IpcDelivered {
            from_node,
            cell_id: to_cell,
        });
        Ok(())
    }

    fn handle_directory_sync(
        &mut self,
        index: usize,
        payload: &[u8],
        now: u64,
    ) -> Result<(), CellNetError> {
        if payload.len() < 16 {
            return Err(CellNetError::Malformed);
        }
        let sequence = u64::from_le_bytes(payload[0..8].try_into().expect("8 bytes"));
        let count = payload[8] as usize;
        if count > MAX_OFFERS_PER_ANNOUNCE
            || payload.len() != 16 + count * crate::wire::OFFER_WIRE_LEN
        {
            return Err(CellNetError::Malformed);
        }
        let node_id = self.peers[index].as_ref().expect("peer").node_id;
        // Snapshots are authoritative for the sending node.
        self.directory.remove_node(node_id);
        for entry in 0..count {
            let offset = 16 + entry * crate::wire::OFFER_WIRE_LEN;
            let offer = crate::wire::decode_offer(payload, offset);
            if !offer.is_valid() {
                continue;
            }
            self.directory.upsert(RemoteCell {
                node_id,
                offer,
                sequence,
                last_seen_tick: now,
                expires_tick: now.saturating_add(offer.ttl_ticks),
                health: RemoteHealth::Reachable,
            })?;
        }
        self.push_event(FabricEvent::DirectoryChanged { node_id });
        Ok(())
    }

    fn send_directory_sync(
        &mut self,
        index: usize,
        now: u64,
        transport: &mut impl Transport,
    ) -> Result<(), CellNetError> {
        let mut payload = [0u8; MAX_ENVELOPE_PAYLOAD];
        payload[0..8].copy_from_slice(&self.announce_sequence.to_le_bytes());
        let mut count = 0usize;
        for offer in self.local_offers.iter().flatten() {
            if count >= MAX_OFFERS_PER_ANNOUNCE {
                break;
            }
            crate::wire::encode_offer(
                offer,
                &mut payload,
                16 + count * crate::wire::OFFER_WIRE_LEN,
            );
            count += 1;
        }
        payload[8] = count as u8;
        payload[9..16].fill(0);
        let payload_len = 16 + count * crate::wire::OFFER_WIRE_LEN;

        let peer = self.peers[index].as_mut().expect("peer");
        let session = peer.session.as_mut().ok_or(CellNetError::NotSecured)?;
        let mut wire = [0u8; MAX_ENVELOPE_WIRE];
        let len = session
            .seal(
                MessageType::DirectorySync,
                &payload[..payload_len],
                &mut wire,
            )
            .map_err(|_| CellNetError::Malformed)?;
        transport
            .send_to(peer.address, &wire[..len])
            .map_err(CellNetError::Transport)?;
        peer.last_tx_tick = now;
        peer.needs_sync = false;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Remote IPC API
    // ------------------------------------------------------------------

    /// Send an encrypted, authenticated IPC message to a discovered remote
    /// cell. The source must be a currently published local cell, the
    /// destination must exist in the directory, and the session must be
    /// established. Caller capability authorization remains an upstream
    /// service-boundary responsibility; this transport API does not accept
    /// or validate caller capability tokens.
    pub fn send_cell_ipc(
        &mut self,
        request: CellIpcRequest,
        payload: &[u8],
        now: u64,
        transport: &mut impl Transport,
    ) -> Result<(), CellNetError> {
        let CellIpcRequest {
            to_node,
            to_cell,
            channel,
            from_cell,
        } = request;
        if payload.len() > MAX_IPC_PAYLOAD {
            return Err(CellNetError::Malformed);
        }
        if !self.is_published(from_cell) {
            return Err(CellNetError::SourceNotPublished);
        }
        if self.directory.get(to_node, to_cell).is_none() {
            return Err(CellNetError::UnknownRemoteCell);
        }
        let index = self.peer_index(to_node).ok_or(CellNetError::UnknownNode)?;
        let peer = self.peers[index].as_mut().expect("peer");
        if peer.phase != PeerPhase::Secured {
            return Err(CellNetError::NotSecured);
        }

        let mut body = [0u8; IPC_HEADER_LEN + MAX_IPC_PAYLOAD];
        body[0..8].copy_from_slice(&to_cell.to_le_bytes());
        body[8..16].copy_from_slice(&channel.to_le_bytes());
        body[16..24].copy_from_slice(&from_cell.to_le_bytes());
        body[IPC_HEADER_LEN..IPC_HEADER_LEN + payload.len()].copy_from_slice(payload);

        let session = peer.session.as_mut().ok_or(CellNetError::NotSecured)?;
        let mut wire = [0u8; MAX_ENVELOPE_WIRE];
        let len = session
            .seal(
                MessageType::CellIpc,
                &body[..IPC_HEADER_LEN + payload.len()],
                &mut wire,
            )
            .map_err(|_| CellNetError::Malformed)?;
        transport
            .send_to(peer.address, &wire[..len])
            .map_err(CellNetError::Transport)?;
        peer.last_tx_tick = now;
        Ok(())
    }

    /// Dequeue the next decrypted inbound IPC for local delivery.
    pub fn next_incoming(&mut self) -> Option<IncomingIpc> {
        let index = self.incoming.iter().position(Option::is_some)?;
        self.incoming[index].take()
    }

    // ------------------------------------------------------------------
    // Frame dispatch and maintenance
    // ------------------------------------------------------------------

    /// Process one received datagram. A validated Hello is registered but
    /// **not** answered automatically: the supervisor must call
    /// `accept_hello` with fresh randomness. Returns `Ok(true)` for that
    /// case.
    pub fn process_datagram(
        &mut self,
        source: NodeAddress,
        bytes: &mut [u8],
        now: u64,
        transport: &mut impl Transport,
    ) -> Result<bool, CellNetError> {
        if bytes.len() < 8 {
            self.stats.frames_malformed += 1;
            return Err(CellNetError::Malformed);
        }
        let message_type = match crate::wire::check_frame_head(bytes, 8) {
            Ok(kind) => kind,
            Err(error) => {
                self.stats.frames_malformed += 1;
                return Err(error);
            }
        };
        match message_type {
            MessageType::Hello => self.inspect_hello(source, bytes, now).map(|_| true),
            MessageType::HelloAck => self
                .handle_hello_ack(source, bytes, now, transport)
                .map(|_| false),
            MessageType::Announce => self.handle_announcement(source, bytes, now).map(|_| false),
            _ => self.handle_envelope(source, bytes, now).map(|_| false),
        }
    }

    /// Periodic maintenance: heartbeats, suspect/offline transitions,
    /// handshake timeouts, directory-sync debt and expiry sweeps.
    pub fn tick(&mut self, now: u64, transport: &mut impl Transport) {
        if self.directory.sweep_expired(now) > 0 {
            self.push_event(FabricEvent::DirectoryChanged { node_id: 0 });
        }

        #[derive(PartialEq)]
        enum Action {
            None,
            Drop,
            Sync,
        }

        let mut suspect_nodes = [0u64; MAX_PEERS];
        let mut suspect_count = 0usize;

        for index in 0..MAX_PEERS {
            let mut action = Action::None;
            {
                let Some(peer) = self.peers[index].as_mut() else {
                    continue;
                };
                match peer.phase {
                    PeerPhase::Secured | PeerPhase::Suspect => {
                        let silence = now.saturating_sub(peer.last_rx_tick);
                        if silence >= self.config.offline_after {
                            action = Action::Drop;
                        } else {
                            if peer.phase == PeerPhase::Secured
                                && silence >= self.config.suspect_after
                            {
                                peer.phase = PeerPhase::Suspect;
                                suspect_nodes[suspect_count] = peer.node_id;
                                suspect_count += 1;
                            }
                            if peer.needs_sync {
                                action = Action::Sync;
                            } else if now.saturating_sub(peer.last_tx_tick)
                                >= self.config.heartbeat_interval
                            {
                                let payload = now.to_le_bytes();
                                let mut wire = [0u8; MAX_ENVELOPE_WIRE];
                                let sealed = peer.session.as_mut().and_then(|session| {
                                    session
                                        .seal(MessageType::Heartbeat, &payload, &mut wire)
                                        .ok()
                                });
                                if let Some(len) = sealed
                                    && transport.send_to(peer.address, &wire[..len]).is_ok()
                                {
                                    peer.last_tx_tick = now;
                                }
                            }
                        }
                    }
                    PeerPhase::HandshakeInitiator => {
                        let stale = now.saturating_sub(
                            peer.handshake
                                .as_ref()
                                .map(|h| h.last_attempt_tick)
                                .unwrap_or(now),
                        );
                        if stale >= self.config.handshake_timeout {
                            // Retransmission is deliberately not done here:
                            // re-dialing requires fresh randomness, so the
                            // supervisor re-calls `connect` after backoff.
                            action = Action::Drop;
                        }
                    }
                    PeerPhase::AwaitingConfirm => {
                        if now.saturating_sub(peer.last_tx_tick) >= self.config.offline_after {
                            action = Action::Drop;
                        }
                    }
                    PeerPhase::Offline => {}
                }
            }
            match action {
                Action::Drop => self.drop_session(index, now),
                Action::Sync => {
                    let _ = self.send_directory_sync(index, now, transport);
                }
                Action::None => {}
            }
        }

        for node_id in suspect_nodes.iter().take(suspect_count) {
            self.directory
                .set_node_health(*node_id, RemoteHealth::Suspect);
            self.push_event(FabricEvent::PeerSuspect { node_id: *node_id });
        }
    }

    /// Node ids of persistent peers whose reconnect backoff has expired.
    /// The supervisor re-dials them with fresh randomness.
    pub fn reconnect_due(&self, now: u64, out: &mut [u64]) -> usize {
        if !self.config.auto_reconnect {
            return 0;
        }
        let mut count = 0;
        for peer in self.peers.iter().flatten() {
            if count >= out.len() {
                break;
            }
            if peer.phase == PeerPhase::Offline
                && peer.persistent
                && now >= peer.backoff_until
                && self.node(peer.node_id).is_some()
            {
                out[count] = peer.node_id;
                count += 1;
            }
        }
        count
    }

    /// Drop a peer session, scrub key material and mark its directory
    /// entries unreachable. The node record survives for reconnection.
    fn drop_session(&mut self, index: usize, now: u64) {
        let Some(peer) = self.peers[index].as_mut() else {
            return;
        };
        let node_id = peer.node_id;
        peer.session = None;
        peer.handshake = None;
        peer.pending_transcript = None;
        peer.needs_sync = false;
        peer.phase = PeerPhase::Offline;
        peer.backoff_attempts = peer.backoff_attempts.saturating_add(1);
        let shift = peer.backoff_attempts.min(10) as u32;
        let backoff = self
            .config
            .backoff_base
            .saturating_mul(1u64 << shift)
            .min(self.config.backoff_max);
        peer.backoff_until = now.saturating_add(backoff);
        self.stats.sessions_lost += 1;
        self.directory
            .set_node_health(node_id, RemoteHealth::Unreachable);
        self.push_event(FabricEvent::PeerLost { node_id });
    }

    /// Forget a peer entirely (session, node record and directory entries).
    pub fn remove_peer(&mut self, node_id: u64) -> bool {
        let Some(index) = self.peer_index(node_id) else {
            return false;
        };
        self.peers[index] = None;
        self.directory.remove_node(node_id);
        if let Some(slot) = self
            .nodes
            .iter_mut()
            .find(|n| n.is_some_and(|record| record.node_id == node_id))
        {
            *slot = None;
        }
        true
    }

    pub fn cell_kind_name(kind: u8) -> &'static str {
        match kind {
            cell_kind::KERNEL_CORE => "kernel-core",
            cell_kind::MEMORY => "memory",
            cell_kind::PROCESS => "process",
            cell_kind::SCHEDULER => "scheduler",
            cell_kind::DRIVER => "driver",
            cell_kind::STORAGE => "storage",
            cell_kind::NETWORK => "network",
            cell_kind::SECURITY => "security",
            cell_kind::SERVICE => "service",
            cell_kind::USER_INTERFACE => "user-interface",
            cell_kind::APPLICATION => "application",
            cell_kind::VIRTUALIZATION => "virtualization",
            _ => "unknown",
        }
    }
}

#[cfg(test)]
mod tests;
