//! Two-node loopback integration tests for the cell fabric.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::vec::Vec;

use super::*;
use crate::directory::{CellOffer, RemoteHealth};
use crate::{MAX_ENVELOPE_WIRE, PublishPolicy, TrustPolicy, cell_kind};

const ADDR_A: u64 = 0xA;
const ADDR_B: u64 = 0xB;
const BROADCAST: u64 = u64::MAX;

/// Shared, partitionable wire between the two test nodes.
struct Wire {
    queue: VecDeque<(u64, u64, Vec<u8>)>,
    up: bool,
}

struct Endpoint {
    address: u64,
    wire: Rc<RefCell<Wire>>,
}

impl Transport for Endpoint {
    fn send_to(&mut self, address: NodeAddress, data: &[u8]) -> Result<(), TransportError> {
        let mut wire = self.wire.borrow_mut();
        if !wire.up {
            return Ok(()); // partition: silent loss
        }
        wire.queue.push_back((self.address, address, data.to_vec()));
        Ok(())
    }

    fn recv_from(
        &mut self,
        buffer: &mut [u8],
    ) -> Result<Option<(NodeAddress, usize)>, TransportError> {
        let mut wire = self.wire.borrow_mut();
        let Some(index) = wire.queue.iter().position(|(from, to, data)| {
            (*to == self.address || (*to == BROADCAST && *from != self.address))
                && data.len() <= buffer.len()
        }) else {
            return Ok(None);
        };
        let (from, _, data) = wire.queue.remove(index).expect("present");
        buffer[..data.len()].copy_from_slice(&data);
        Ok(Some((from, data.len())))
    }
}

struct Pair {
    a: CellFabric,
    b: CellFabric,
    ta: Endpoint,
    tb: Endpoint,
    wire: Rc<RefCell<Wire>>,
    now: u64,
    nonce_counter: u64,
}

fn offer(cell: u64, kind: u8, caps: u64, interface: u64) -> CellOffer {
    CellOffer {
        cell_id: cell,
        kind,
        state: 3,
        abi_major: 1,
        abi_minor: 0,
        required_caps: caps,
        interface_hash: interface,
        ttl_ticks: 2000,
    }
}

fn permissive_policy() -> PublishPolicy {
    PublishPolicy::new(0x0FFF, u64::MAX)
}

impl Pair {
    fn new() -> Self {
        Self::with_config(FabricConfig {
            publish_policy: permissive_policy(),
            ..FabricConfig::development()
        })
    }

    fn with_config(config: FabricConfig) -> Self {
        let wire = Rc::new(RefCell::new(Wire {
            queue: VecDeque::new(),
            up: true,
        }));
        Self {
            a: CellFabric::new([0xAA; 32], config),
            b: CellFabric::new([0xBB; 32], config),
            ta: Endpoint {
                address: ADDR_A,
                wire: wire.clone(),
            },
            tb: Endpoint {
                address: ADDR_B,
                wire: wire.clone(),
            },
            wire,
            now: 1_000,
            nonce_counter: 0,
        }
    }

    fn fresh_nonce(&mut self) -> ([u8; 32], [u8; 32]) {
        self.nonce_counter += 1;
        let counter = self.nonce_counter.to_le_bytes();
        let mut material = [0u8; 16];
        material[..8].copy_from_slice(&counter);
        material[8..].copy_from_slice(&counter);
        let nonce = awe_securityd::sha256(&material);
        let secret = awe_securityd::sha256(&nonce);
        (nonce, secret)
    }

    /// Drain both endpoints, processing every queued datagram and
    /// automatically answering validated Hellos with fresh randomness.
    fn pump(&mut self) {
        loop {
            let mut progress = false;
            let mut buffer = [0u8; MAX_ENVELOPE_WIRE];

            while let Ok(Some((from, len))) = self.ta.recv_from(&mut buffer) {
                progress = true;
                let mut frame = [0u8; MAX_ENVELOPE_WIRE];
                frame[..len].copy_from_slice(&buffer[..len]);
                let raw = frame;
                if let Ok(true) =
                    self.a
                        .process_datagram(from, &mut frame[..len], self.now, &mut self.ta)
                {
                    let (nonce, secret) = self.fresh_nonce();
                    self.a
                        .accept_hello(&raw[..len], nonce, secret, self.now, &mut self.ta)
                        .expect("accept hello A");
                }
            }
            while let Ok(Some((from, len))) = self.tb.recv_from(&mut buffer) {
                progress = true;
                let mut frame = [0u8; MAX_ENVELOPE_WIRE];
                frame[..len].copy_from_slice(&buffer[..len]);
                let raw = frame;
                if let Ok(true) =
                    self.b
                        .process_datagram(from, &mut frame[..len], self.now, &mut self.tb)
                {
                    let (nonce, secret) = self.fresh_nonce();
                    self.b
                        .accept_hello(&raw[..len], nonce, secret, self.now, &mut self.tb)
                        .expect("accept hello B");
                }
            }
            if !progress {
                break;
            }
        }
    }

    fn exchange_announcements(&mut self) {
        let announce_a = self.a.make_announcement(self.now);
        self.ta.send_to(BROADCAST, &announce_a).expect("send");
        let announce_b = self.b.make_announcement(self.now);
        self.tb.send_to(BROADCAST, &announce_b).expect("send");
        self.pump();
    }

    fn establish(&mut self) {
        self.exchange_announcements();
        let (nonce, secret) = self.fresh_nonce();
        let b_id = self.b.node_id();
        self.a
            .connect(b_id, nonce, secret, self.now, &mut self.ta)
            .expect("connect");
        self.pump();
        assert_eq!(self.a.peer_phase(b_id), Some(PeerPhase::Secured));
        assert_eq!(
            self.b.peer_phase(self.a.node_id()),
            Some(PeerPhase::Secured)
        );
    }

    fn tick_both(&mut self) {
        self.a.tick(self.now, &mut self.ta);
        self.b.tick(self.now, &mut self.tb);
        self.pump();
    }

    fn advance(&mut self, ticks: u64) {
        self.now += ticks;
    }
}

fn publish_a_source(pair: &mut Pair) {
    pair.a
        .publish_offer(offer(1001, cell_kind::APPLICATION, 0, 0xA001))
        .expect("publish A source cell");
    pair.exchange_announcements();
}

#[test]
fn accept_hello_requires_the_exact_inspected_signed_frame() {
    let mut pair = Pair::new();
    let (hello_nonce, hello_secret) = pair.fresh_nonce();
    let mut hello = HelloFrame {
        node_pubkey: pair.a.public_key(),
        node_id: pair.a.node_id(),
        x25519_public: awe_securityd::x25519_public_from_secret(&hello_secret),
        nonce: hello_nonce,
        endpoint: ADDR_A,
        tick: pair.now,
        signature: [0; 64],
    };
    let mut hello_wire = [0u8; HELLO_LEN];
    encode_hello(&hello, &mut hello_wire);
    hello.signature = pair.a.identity.sign(&hello_wire[..HELLO_SIGNED_LEN]);
    encode_hello(&hello, &mut hello_wire);

    pair.b
        .inspect_hello(ADDR_A, &hello_wire, pair.now)
        .expect("inspect");

    let (other_nonce, other_secret) = pair.fresh_nonce();
    let mut other = HelloFrame {
        node_pubkey: pair.a.public_key(),
        node_id: pair.a.node_id(),
        x25519_public: awe_securityd::x25519_public_from_secret(&other_secret),
        nonce: other_nonce,
        endpoint: ADDR_A,
        tick: pair.now + 1,
        signature: [0; 64],
    };
    let mut other_wire = [0u8; HELLO_LEN];
    encode_hello(&other, &mut other_wire);
    other.signature = pair.a.identity.sign(&other_wire[..HELLO_SIGNED_LEN]);
    encode_hello(&other, &mut other_wire);

    assert_eq!(
        pair.b
            .accept_hello(&other_wire, [0x55; 32], [0x66; 32], pair.now, &mut pair.tb),
        Err(CellNetError::Malformed)
    );
    pair.b
        .accept_hello(&hello_wire, [0x77; 32], [0x88; 32], pair.now, &mut pair.tb)
        .expect("accept exact inspected Hello");
}

#[test]
fn announcement_discovers_node_and_cells() {
    let mut pair = Pair::new();
    pair.b
        .publish_offer(offer(42, cell_kind::STORAGE, 0x1, 0xC0FFEE))
        .expect("publish");
    let announce = pair.b.make_announcement(pair.now);
    pair.tb.send_to(BROADCAST, &announce).expect("send");
    pair.pump();

    let b_id = pair.b.node_id();
    let entry = pair.a.directory().get(b_id, 42).expect("directory entry");
    assert_eq!(entry.offer.kind, cell_kind::STORAGE);
    assert_eq!(entry.offer.interface_hash, 0xC0FFEE);
    assert!(pair.a.directory().find_by_interface(0xC0FFEE).is_none()); // unreachable until secured
}

#[test]
fn announcement_tampering_and_spoofing_rejected() {
    let mut pair = Pair::new();
    pair.b
        .publish_offer(offer(42, cell_kind::STORAGE, 0x1, 0xC0FFEE))
        .expect("publish");
    let announce = pair.b.make_announcement(pair.now);

    // Tampered signature byte.
    let mut forged = announce;
    forged[100] ^= 0x01;
    let result = pair
        .a
        .process_datagram(ADDR_B, &mut forged, pair.now, &mut pair.ta);
    assert_eq!(result, Err(CellNetError::BadSignature));

    // Spoofed node id (does not match the key).
    let mut spoofed = announce;
    spoofed[40] ^= 0x01;
    let result = pair
        .a
        .process_datagram(ADDR_B, &mut spoofed, pair.now, &mut pair.ta);
    assert_eq!(result, Err(CellNetError::BadNodeBinding));
}

#[test]
fn handshake_establishes_session_and_encrypted_ipc_flows() {
    let mut pair = Pair::new();
    pair.b
        .publish_offer(offer(42, cell_kind::SERVICE, 0x0, 0xBEEF))
        .expect("publish");
    pair.establish();
    publish_a_source(&mut pair);

    let b_id = pair.b.node_id();
    let a_id = pair.a.node_id();

    // Anti-entropy: after securing, B's directory snapshot reaches A.
    pair.tick_both();
    let entry = pair.a.directory().get(b_id, 42).expect("synced entry");
    assert_eq!(entry.health, RemoteHealth::Reachable);

    pair.a
        .send_cell_ipc(
            crate::CellIpcRequest {
                to_node: b_id,
                to_cell: 42,
                channel: 7,
                from_cell: 1001,
            },
            b"read sector 0",
            pair.now,
            &mut pair.ta,
        )
        .expect("send ipc");
    pair.pump();

    let incoming = pair.b.next_incoming().expect("incoming ipc");
    assert_eq!(incoming.from_node, a_id);
    assert_eq!(incoming.to_cell, 42);
    assert_eq!(incoming.channel, 7);
    assert_eq!(incoming.from_cell, 1001);
    assert_eq!(&incoming.payload[..incoming.payload_len], b"read sector 0");

    // Refresh A's published offer and re-announce before the reply path.
    pair.a
        .publish_offer(offer(1001, cell_kind::APPLICATION, 0x0, 0xF00D))
        .expect("publish a");
    pair.exchange_announcements();
    pair.b
        .send_cell_ipc(
            crate::CellIpcRequest {
                to_node: a_id,
                to_cell: 1001,
                channel: 7,
                from_cell: 42,
            },
            b"sector data",
            pair.now,
            &mut pair.tb,
        )
        .expect("reply");
    pair.pump();
    let reply = pair.a.next_incoming().expect("reply ipc");
    assert_eq!(&reply.payload[..reply.payload_len], b"sector data");

    assert_eq!(pair.a.stats().sessions_established, 1);
    assert_eq!(pair.b.stats().sessions_established, 1);
    assert_eq!(pair.a.stats().frames_decrypt_failed, 0);
    assert_eq!(pair.b.stats().frames_replay_rejected, 0);
}

#[test]
fn send_rejects_unpublished_local_source_cell() {
    let mut pair = Pair::new();
    pair.b
        .publish_offer(offer(42, cell_kind::SERVICE, 0, 0xBEEF))
        .expect("publish destination");
    pair.establish();

    let result = pair.a.send_cell_ipc(
        crate::CellIpcRequest {
            to_node: pair.b.node_id(),
            to_cell: 42,
            channel: 1,
            from_cell: 999,
        },
        b"must be denied",
        pair.now,
        &mut pair.ta,
    );
    assert_eq!(result, Err(CellNetError::SourceNotPublished));
}

#[test]
fn receiver_rejects_unpublished_remote_source_cell() {
    let mut pair = Pair::new();
    pair.b
        .publish_offer(offer(42, cell_kind::SERVICE, 0, 0xBEEF))
        .expect("publish destination");
    pair.establish();

    // Bypass the sender API to model a compromised peer claiming an
    // unpublished source cell. The receiving fabric must reject it too.
    let peer_index = pair.a.peer_index(pair.b.node_id()).expect("peer");
    let mut body = [0u8; IPC_HEADER_LEN + CAP_TOKEN_LEN + MAX_IPC_PAYLOAD];
    body[0..8].copy_from_slice(&42u64.to_le_bytes());
    body[8..16].copy_from_slice(&1u64.to_le_bytes());
    body[16..24].copy_from_slice(&999u64.to_le_bytes());
    let mut wire = [0u8; MAX_ENVELOPE_WIRE];
    let len = pair.a.peers[peer_index]
        .as_mut()
        .expect("peer")
        .session
        .as_mut()
        .expect("session")
        .seal(MessageType::CellIpc, &body[..IPC_HEADER_LEN + CAP_TOKEN_LEN], &mut wire)
        .expect("seal");
    pair.ta.send_to(ADDR_B, &wire[..len]).expect("send");
    pair.pump();

    assert!(pair.b.next_incoming().is_none());
    assert_eq!(pair.b.stats().ipc_dropped, 1);
    assert_eq!(pair.b.stats().ipc_delivered, 0);
}

#[test]
fn tampered_ciphertext_is_rejected_and_counted() {
    let mut pair = Pair::new();
    pair.b
        .publish_offer(offer(42, cell_kind::SERVICE, 0x0, 0xBEEF))
        .expect("publish");
    pair.establish();
    publish_a_source(&mut pair);

    let a_id = pair.a.node_id();
    let last_rx = pair
        .b
        .peers
        .iter()
        .flatten()
        .find(|peer| peer.node_id == a_id)
        .expect("peer")
        .last_rx_tick;
    pair.advance(1);

    let b_id = pair.b.node_id();
    pair.a
        .send_cell_ipc(
            crate::CellIpcRequest {
                to_node: b_id,
                to_cell: 42,
                channel: 1,
                from_cell: 1001,
            },
            b"secret",
            pair.now,
            &mut pair.ta,
        )
        .expect("send");

    // Corrupt one payload byte while the frame is on the wire.
    {
        let mut wire = pair.wire.borrow_mut();
        let datagram = wire
            .queue
            .iter_mut()
            .find(|(_, to, _)| *to == ADDR_B)
            .expect("queued frame");
        let len = datagram.2.len();
        datagram.2[len - 20] ^= 0x01;
    }
    pair.pump();

    assert!(pair.b.next_incoming().is_none());
    assert_eq!(pair.b.stats().frames_decrypt_failed, 1);
    let current_rx = pair
        .b
        .peers
        .iter()
        .flatten()
        .find(|peer| peer.node_id == a_id)
        .expect("peer")
        .last_rx_tick;
    assert_eq!(
        current_rx, last_rx,
        "bad-tag traffic must not refresh liveness"
    );
}

#[test]
fn replayed_frame_is_rejected_and_counted() {
    let mut pair = Pair::new();
    pair.b
        .publish_offer(offer(42, cell_kind::SERVICE, 0x0, 0xBEEF))
        .expect("publish");
    pair.establish();
    publish_a_source(&mut pair);

    let b_id = pair.b.node_id();
    pair.a
        .send_cell_ipc(
            crate::CellIpcRequest {
                to_node: b_id,
                to_cell: 42,
                channel: 1,
                from_cell: 1001,
            },
            b"once",
            pair.now,
            &mut pair.ta,
        )
        .expect("send");

    // Deliver the same wire frame twice (attacker replay).
    let datagram = {
        let mut wire = pair.wire.borrow_mut();
        let index = wire
            .queue
            .iter()
            .position(|(_, to, _)| *to == ADDR_B)
            .expect("queued frame");
        wire.queue.remove(index).expect("frame")
    };
    let mut first = [0u8; MAX_ENVELOPE_WIRE];
    first[..datagram.2.len()].copy_from_slice(&datagram.2);
    pair.b
        .process_datagram(
            datagram.0,
            &mut first[..datagram.2.len()],
            pair.now,
            &mut pair.tb,
        )
        .expect("first delivery");
    let mut second = [0u8; MAX_ENVELOPE_WIRE];
    second[..datagram.2.len()].copy_from_slice(&datagram.2);
    let result = pair.b.process_datagram(
        datagram.0,
        &mut second[..datagram.2.len()],
        pair.now,
        &mut pair.tb,
    );
    assert_eq!(result, Err(CellNetError::ReplayRejected));
    assert_eq!(pair.b.stats().frames_replay_rejected, 1);
    assert_eq!(pair.b.next_incoming().map(|m| m.to_cell), Some(42));
    assert!(pair.b.next_incoming().is_none());
}

#[test]
fn ipc_to_unpublished_cell_is_dropped() {
    let mut pair = Pair::new();
    pair.b
        .publish_offer(offer(42, cell_kind::SERVICE, 0x0, 0xBEEF))
        .expect("publish");
    pair.establish();
    publish_a_source(&mut pair);
    pair.tick_both();

    // B withdraws the cell but A still holds a stale directory entry.
    assert!(pair.b.withdraw_offer(42));
    let b_id = pair.b.node_id();
    pair.a
        .send_cell_ipc(
            crate::CellIpcRequest {
                to_node: b_id,
                to_cell: 42,
                channel: 1,
                from_cell: 1001,
            },
            b"stale call",
            pair.now,
            &mut pair.ta,
        )
        .expect("send still works with stale directory");
    pair.pump();

    assert!(pair.b.next_incoming().is_none());
    assert_eq!(pair.b.stats().ipc_dropped, 1);

    // The next anti-entropy snapshot repairs A's stale entry.
    pair.advance(150);
    pair.tick_both();
    assert!(pair.a.directory().get(b_id, 42).is_none());
}

#[test]
fn suspect_offline_and_partition_recovery() {
    let mut pair = Pair::new();
    pair.b
        .publish_offer(offer(42, cell_kind::SERVICE, 0x0, 0xBEEF))
        .expect("publish");
    pair.establish();
    pair.tick_both();
    let b_id = pair.b.node_id();

    // Cut the link. A observes silence, degrades, then drops the session.
    pair.wire.borrow_mut().up = false;
    pair.advance(400);
    pair.tick_both();
    assert_eq!(pair.a.peer_phase(b_id), Some(PeerPhase::Suspect));
    assert_eq!(
        pair.a.directory().get(b_id, 42).map(|e| e.health),
        Some(RemoteHealth::Suspect)
    );

    pair.advance(500);
    pair.tick_both();
    assert_eq!(pair.a.peer_phase(b_id), Some(PeerPhase::Offline));
    assert_eq!(
        pair.a.directory().get(b_id, 42).map(|e| e.health),
        Some(RemoteHealth::Unreachable)
    );
    let mut saw_lost = false;
    while let Some(event) = pair.a.next_event() {
        if matches!(event, FabricEvent::PeerLost { node_id } if node_id == b_id) {
            saw_lost = true;
        }
    }
    assert!(saw_lost);

    // Heal the partition. Backoff must expire before re-dialing.
    pair.wire.borrow_mut().up = true;
    let (nonce, secret) = pair.fresh_nonce();
    let early = pair.a.connect(b_id, nonce, secret, pair.now, &mut pair.ta);
    assert_eq!(early, Err(CellNetError::BackoffActive));

    pair.advance(1600);
    let mut due = [0u64; 4];
    assert_eq!(pair.a.reconnect_due(pair.now, &mut due), 1);
    assert_eq!(due[0], b_id);

    let (nonce, secret) = pair.fresh_nonce();
    pair.a
        .connect(b_id, nonce, secret, pair.now, &mut pair.ta)
        .expect("reconnect");
    pair.pump();
    assert_eq!(pair.a.peer_phase(b_id), Some(PeerPhase::Secured));

    // Partition recovery: the fresh DirectorySync repopulates A's
    // directory for B without any new announcement.
    pair.tick_both();
    let entry = pair.a.directory().get(b_id, 42).expect("recovered entry");
    assert_eq!(entry.health, RemoteHealth::Reachable);
}

#[test]
fn publish_policy_filters_offers() {
    let mut pair = Pair::with_config(FabricConfig {
        publish_policy: PublishPolicy::new(1 << cell_kind::STORAGE, 0x3),
        ..FabricConfig::development()
    });
    // Wrong kind.
    assert_eq!(
        pair.a
            .publish_offer(offer(1, cell_kind::SERVICE, 0x0, 0xAA)),
        Err(CellNetError::PolicyDenied)
    );
    // Capability outside the mask.
    assert_eq!(
        pair.a
            .publish_offer(offer(2, cell_kind::STORAGE, 0x8, 0xBB)),
        Err(CellNetError::PolicyDenied)
    );
    // Permitted.
    pair.a
        .publish_offer(offer(3, cell_kind::STORAGE, 0x3, 0xCC))
        .expect("permitted offer");
}

#[test]
fn pinned_trust_rejects_unknown_keys() {
    let mut pinned = FabricConfig::development();
    pinned.trust = TrustPolicy::Pinned;
    pinned.publish_policy = permissive_policy();
    let mut pair = Pair::with_config(pinned);

    pair.b
        .publish_offer(offer(42, cell_kind::SERVICE, 0x0, 0xBEEF))
        .expect("publish");
    let announce = pair.b.make_announcement(pair.now);
    pair.tb.send_to(BROADCAST, &announce).expect("send");
    pair.pump();

    assert!(pair.a.directory().get(pair.b.node_id(), 42).is_none());
    assert!(pair.a.stats().frames_untrusted >= 1);

    // After pinning B's key, discovery and handshake succeed.
    pair.a.add_trusted_key(pair.b.public_key()).expect("pin");
    let announce = pair.b.make_announcement(pair.now);
    pair.tb.send_to(BROADCAST, &announce).expect("send");
    pair.pump();
    assert!(pair.a.directory().get(pair.b.node_id(), 42).is_some());

    let (nonce, secret) = pair.fresh_nonce();
    pair.a
        .connect(pair.b.node_id(), nonce, secret, pair.now, &mut pair.ta)
        .expect("connect");
    // B is in Pinned mode too and does not trust A yet.
    pair.pump();
    assert_ne!(
        pair.a.peer_phase(pair.b.node_id()),
        Some(PeerPhase::Secured)
    );
}

#[test]
fn low_order_x25519_key_is_rejected() {
    use crate::session::NodeIdentity;
    use crate::wire::{HELLO_SIGNED_LEN, HelloFrame, encode_hello};

    let mut pair = Pair::new();
    // Handcraft a Hello carrying a low-order X25519 public key, but validly
    // signed by a real Ed25519 identity: signature checks must pass and the
    // contributory check must still refuse the session.
    let attacker = NodeIdentity::from_seed(&[0xCC; 32]);
    let mut hello = HelloFrame {
        node_pubkey: attacker.public_key,
        node_id: attacker.node_id,
        x25519_public: [0u8; 32],
        nonce: [0x09; 32],
        endpoint: 0,
        tick: pair.now,
        signature: [0u8; 64],
    };
    let mut wire = [0u8; crate::HELLO_LEN];
    encode_hello(&hello, &mut wire);
    hello.signature = attacker.sign(&wire[..HELLO_SIGNED_LEN]);
    encode_hello(&hello, &mut wire);

    let mut frame = [0u8; MAX_ENVELOPE_WIRE];
    frame[..crate::HELLO_LEN].copy_from_slice(&wire);
    let inspected =
        pair.b
            .process_datagram(0xC, &mut frame[..crate::HELLO_LEN], pair.now, &mut pair.tb);
    assert_eq!(inspected, Ok(true));
    let result = pair
        .b
        .accept_hello(&wire, [0x11; 32], [0x22; 32], pair.now, &mut pair.tb);
    assert_eq!(result, Err(CellNetError::LowOrderKey));
    assert_ne!(
        pair.b.peer_phase(attacker.node_id),
        Some(PeerPhase::Secured)
    );
}

#[test]
fn directory_ttl_expires_without_heartbeats() {
    let mut pair = Pair::new();
    pair.b
        .publish_offer(offer(42, cell_kind::SERVICE, 0x0, 0xBEEF))
        .expect("publish");
    let announce = pair.b.make_announcement(pair.now);
    pair.tb.send_to(BROADCAST, &announce).expect("send");
    pair.pump();
    let b_id = pair.b.node_id();
    assert!(pair.a.directory().get(b_id, 42).is_some());

    // TTL is capped at the publisher's configured offer_ttl (3000 here).
    pair.advance(3100);
    pair.a.tick(pair.now, &mut pair.ta);
    assert!(pair.a.directory().get(b_id, 42).is_none());
}

// ---------------------------------------------------------------------
// Capability-authorized remote cell IPC (require_ipc_caps = true)
// ---------------------------------------------------------------------

fn issuer_keypair() -> ([u8; 32], [u8; 64]) {
    awe_securityd::ed25519_keypair_from_seed(&[0x5E; 32])
}

fn enforced_pair() -> Pair {
    Pair::with_config(FabricConfig {
        publish_policy: permissive_policy(),
        ..FabricConfig::capability_enforced()
    })
}

fn grant_for(
    issuer_secret: &[u8; 64],
    node: u64,
    cell: u64,
    caps: u64,
    sequence: u64,
    now: u64,
) -> CapabilityGrant {
    let mut grant = CapabilityGrant {
        version: 0,
        grantee_node: node,
        cell_id: cell,
        caps,
        not_before: now.saturating_sub(10),
        not_after: now.saturating_add(1000),
        sequence,
        signature: [0u8; 64],
    };
    grant.sign_with(issuer_secret);
    grant
}

#[test]
fn authorized_ipc_flows_in_enforced_mode() {
    let (issuer_public, issuer_secret) = issuer_keypair();
    let mut pair = enforced_pair();
    pair.b
        .publish_offer(offer(42, cell_kind::SERVICE, 0x3, 0xBEEF))
        .expect("publish");
    pair.establish();
    publish_a_source(&mut pair);
    pair.tick_both();
    pair.a.set_capability_authority(issuer_public);
    pair.b.set_capability_authority(issuer_public);

    let b_id = pair.b.node_id();
    let a_id = pair.a.node_id();
    let grant = grant_for(&issuer_secret, a_id, 1001, 0x3, 1, pair.now);
    pair.a
        .send_cell_ipc_authorized(
            crate::CellIpcRequest {
                to_node: b_id,
                to_cell: 42,
                channel: 7,
                from_cell: 1001,
            },
            &grant,
            b"authorized call",
            pair.now,
            &mut pair.ta,
        )
        .expect("authorized send");
    pair.pump();

    let incoming = pair.b.next_incoming().expect("delivered");
    assert_eq!(incoming.from_node, a_id);
    assert_eq!(incoming.from_cell, 1001);
    assert_eq!(incoming.to_cell, 42);
    assert_eq!(
        &incoming.payload[..incoming.payload_len],
        b"authorized call"
    );
    assert_eq!(pair.b.stats().ipc_delivered, 1);
    assert_eq!(pair.b.stats().ipc_dropped, 0);
}

#[test]
fn enforced_mode_fails_closed_without_a_grant() {
    let (issuer_public, _issuer_secret) = issuer_keypair();
    let mut pair = enforced_pair();
    pair.b
        .publish_offer(offer(42, cell_kind::SERVICE, 0x3, 0xBEEF))
        .expect("publish");
    pair.establish();
    publish_a_source(&mut pair);
    pair.tick_both();
    pair.a.set_capability_authority(issuer_public);
    pair.b.set_capability_authority(issuer_public);

    let b_id = pair.b.node_id();
    // The legacy, grant-free API must be refused in enforced mode.
    assert_eq!(
        pair.a.send_cell_ipc(
            crate::CellIpcRequest {
                to_node: b_id,
                to_cell: 42,
                channel: 1,
                from_cell: 1001,
            },
            b"no token",
            pair.now,
            &mut pair.ta,
        ),
        Err(CellNetError::CapabilityDenied)
    );
    assert!(pair.b.next_incoming().is_none());
}

#[test]
fn insufficient_grant_caps_are_denied_on_send() {
    let (issuer_public, issuer_secret) = issuer_keypair();
    let mut pair = enforced_pair();
    pair.b
        .publish_offer(offer(42, cell_kind::SERVICE, 0x3, 0xBEEF))
        .expect("publish");
    pair.establish();
    publish_a_source(&mut pair);
    pair.tick_both();
    pair.a.set_capability_authority(issuer_public);

    let b_id = pair.b.node_id();
    let a_id = pair.a.node_id();
    // Grant covers only 0x1; the destination cell requires 0x3.
    let grant = grant_for(&issuer_secret, a_id, 1001, 0x1, 1, pair.now);
    assert_eq!(
        pair.a.send_cell_ipc_authorized(
            crate::CellIpcRequest {
                to_node: b_id,
                to_cell: 42,
                channel: 1,
                from_cell: 1001,
            },
            &grant,
            b"under-caps",
            pair.now,
            &mut pair.ta,
        ),
        Err(CellNetError::CapabilityDenied)
    );
}

#[test]
fn expired_grant_is_rejected_on_send() {
    let (issuer_public, issuer_secret) = issuer_keypair();
    let mut pair = enforced_pair();
    pair.b
        .publish_offer(offer(42, cell_kind::SERVICE, 0x3, 0xBEEF))
        .expect("publish");
    pair.establish();
    publish_a_source(&mut pair);
    pair.tick_both();
    pair.a.set_capability_authority(issuer_public);

    let b_id = pair.b.node_id();
    let a_id = pair.a.node_id();
    let mut grant = grant_for(&issuer_secret, a_id, 1001, 0x3, 1, pair.now);
    grant.not_after = pair.now.saturating_sub(1);
    grant.sign_with(&issuer_secret);
    assert_eq!(
        pair.a.send_cell_ipc_authorized(
            crate::CellIpcRequest {
                to_node: b_id,
                to_cell: 42,
                channel: 1,
                from_cell: 1001,
            },
            &grant,
            b"expired",
            pair.now,
            &mut pair.ta,
        ),
        Err(CellNetError::CapabilityExpired)
    );
}

#[test]
fn grant_sequence_is_single_use_on_send() {
    let (issuer_public, issuer_secret) = issuer_keypair();
    let mut pair = enforced_pair();
    pair.b
        .publish_offer(offer(42, cell_kind::SERVICE, 0x3, 0xBEEF))
        .expect("publish");
    pair.establish();
    publish_a_source(&mut pair);
    pair.tick_both();
    pair.a.set_capability_authority(issuer_public);
    pair.b.set_capability_authority(issuer_public);

    let b_id = pair.b.node_id();
    let a_id = pair.a.node_id();
    let grant = grant_for(&issuer_secret, a_id, 1001, 0x3, 1, pair.now);
    pair.a
        .send_cell_ipc_authorized(
            crate::CellIpcRequest {
                to_node: b_id,
                to_cell: 42,
                channel: 1,
                from_cell: 1001,
            },
            &grant,
            b"first",
            pair.now,
            &mut pair.ta,
        )
        .expect("first send");
    assert_eq!(
        pair.a.send_cell_ipc_authorized(
            crate::CellIpcRequest {
                to_node: b_id,
                to_cell: 42,
                channel: 1,
                from_cell: 1001,
            },
            &grant,
            b"second",
            pair.now,
            &mut pair.ta,
        ),
        Err(CellNetError::CapabilityReplayRejected)
    );
}

#[test]
fn grant_from_a_foreign_issuer_fails_verification() {
    let (issuer_public, _issuer_secret) = issuer_keypair();
    let (_other_public, other_secret) = awe_securityd::ed25519_keypair_from_seed(&[0x6F; 32]);
    let mut pair = enforced_pair();
    pair.b
        .publish_offer(offer(42, cell_kind::SERVICE, 0x3, 0xBEEF))
        .expect("publish");
    pair.establish();
    publish_a_source(&mut pair);
    pair.tick_both();
    pair.a.set_capability_authority(issuer_public);

    let b_id = pair.b.node_id();
    let a_id = pair.a.node_id();
    let grant = grant_for(&other_secret, a_id, 1001, 0x3, 1, pair.now);
    assert_eq!(
        pair.a.send_cell_ipc_authorized(
            crate::CellIpcRequest {
                to_node: b_id,
                to_cell: 42,
                channel: 1,
                from_cell: 1001,
            },
            &grant,
            b"forged",
            pair.now,
            &mut pair.ta,
        ),
        Err(CellNetError::BadSignature)
    );
}

#[test]
fn receiver_drops_a_forged_capability_token() {
    let (issuer_public, _issuer_secret) = issuer_keypair();
    let mut pair = enforced_pair();
    pair.b
        .publish_offer(offer(42, cell_kind::SERVICE, 0x3, 0xBEEF))
        .expect("publish");
    pair.establish();
    publish_a_source(&mut pair);
    pair.tick_both();
    pair.b.set_capability_authority(issuer_public);

    // A compromised sender seals a valid-looking IPC payload whose
    // capability token is all zeros: the receiver must drop it.
    let peer_index = pair.a.peer_index(pair.b.node_id()).expect("peer");
    let mut body = [0u8; IPC_HEADER_LEN + CAP_TOKEN_LEN + MAX_IPC_PAYLOAD];
    body[0..8].copy_from_slice(&42u64.to_le_bytes());
    body[8..16].copy_from_slice(&1u64.to_le_bytes());
    body[16..24].copy_from_slice(&1001u64.to_le_bytes());
    let mut wire = [0u8; MAX_ENVELOPE_WIRE];
    let len = pair.a.peers[peer_index]
        .as_mut()
        .expect("peer")
        .session
        .as_mut()
        .expect("session")
        .seal(
            MessageType::CellIpc,
            &body[..IPC_HEADER_LEN + CAP_TOKEN_LEN],
            &mut wire,
        )
        .expect("seal");
    pair.ta.send_to(ADDR_B, &wire[..len]).expect("send");
    pair.pump();

    assert!(pair.b.next_incoming().is_none());
    assert_eq!(pair.b.stats().ipc_dropped, 1);
    assert_eq!(pair.b.stats().ipc_delivered, 0);
}
