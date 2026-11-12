# AWE_OS CellNet — Network Cell Architecture (ABI 0.1)

**Status:** first executable, bounded distributed-cell fabric merged as
`services/cellnetd` (`awe-cellnetd`), with host-side loopback integration
tests. **Scope honesty:** this is the authenticated discovery + session +
remote-IPC contract layer. It is not yet wired to a real NIC datapath, does
not provide multicast discovery, and does not claim forward secrecy beyond
per-session ephemeral keys.

`cellnetd` is the distributed counterpart of the kernel's local cell
runtime (`aweos_kernel::cell`, ABI 1.1). Local cells keep their existing
lifecycle; `cellnetd` makes *published* cells discoverable and addressable
across machines.

## Design rules

1. **Bounded and allocation-free**, like every other service crate: fixed
   peer table (8), node table (16), remote directory (32 cells), local
   offer table (8), incoming queue (16).
2. **No invented cryptography.** All primitives live in `awe-securityd`
   (the Security Cell): Ed25519 (identity/signatures), X25519 (key
   agreement), HKDF-SHA256 (key schedule), ChaCha20-Poly1305 (AEAD),
   SHA-256/HMAC-SHA256 (transcript/confirmation).
3. **No trusted network.** Every control-plane frame is signed; every
   data-plane frame is encrypted and authenticated; a 64-bit sliding
   anti-replay window rejects duplicates.
4. **No hidden RNG.** Handshake nonces and ephemeral X25519 secrets are
   supplied by the caller (`connect` / `accept_hello`), keeping the crate
   deterministic, testable, and free of a `getrandom` dependency on
   `no_std`.
5. **Publish-only visibility.** A node advertises exactly the cells its
   `PublishPolicy` permits (kind mask + capability mask, deny-all by
   default). Remote peers can send IPC only to cells that are currently
   published. The advertised required-capability mask is metadata for the
   upstream authorization boundary; the fabric API does not yet accept a
   trusted per-caller capability token.

## Identity and node binding

- Long-term identity: Ed25519 keypair (`NodeIdentity::from_seed`).
- `node_id = SHA-256(public_key)[0..8]` (little-endian). The binding is
  verified on every signed frame, so a node cannot claim an id that does
  not match its key, and a known node cannot rotate keys without being
  rejected as a spoof.
- Trust policy: `Open` (TOFU, development only) or `Pinned` (explicit
  trusted-key table). Pinned mode rejects announcements and handshakes
  from unregistered keys.

## Wire protocol (version 1, little-endian)

Magic `AWCN` (`0x414E_4357` LE), `u16` version, `u8` type.

| Type | Name | Protection | Size |
|------|------|------------|------|
| 1 | Hello | Ed25519 signature | 192 B |
| 2 | HelloAck | Ed25519 signature | 216 B |
| 3 | Finished | AEAD (session) | 32 B hdr + 32 B payload + 16 B tag |
| 4 | Announce | Ed25519 signature | 520 B |
| 5 | Heartbeat | AEAD (session) | 32 B hdr + 8 B payload + 16 B tag |
| 7 | CellIpc | AEAD (session) | 32 B hdr + ≤536 B payload + 16 B tag |
| 9 | DirectorySync | AEAD (session) | 32 B hdr + ≤400 B payload + 16 B tag |

Signatures are always verified over the exact received byte range; frames
are never re-encoded before verification.

### Handshake (mutual authentication, 4 flights)

```text
Initiator                                Responder
   | -------- Hello (signed) --------------> |
   |  pubkey, node_id, x25519_pub_i,        |
   |  nonce_i, endpoint, tick               |
   | <----- HelloAck (signed) ------------- |
   |  pubkey, node_id, x25519_pub_r,        |
   |  nonce_r, nonce_i (echo), tick         |
   | -------- Finished (AEAD) ------------> |
   | <------- Finished (AEAD) ------------- |
```

- `transcript = SHA-256(hello_signed ‖ hello_ack_signed)`
- `shared = X25519(ephemeral_secret, peer_public)`; low-order
  (non-contributory) peer keys are rejected (`was_contributory`).
- Salt: `nonce_i ‖ nonce_r`; IKM: `shared`;
  info: `"AWE-CELLNET-SESSION-V1" ‖ transcript`.
- HKDF expands 72 bytes: `k_i2r ‖ k_r2i ‖ session_id`.
- Finished payload: `HMAC-SHA256(tx_key, "AWE-CELLNET-CONFIRM-V1" ‖
  transcript)` — explicit mutual key confirmation; the session enters
  `Secured` only after the peer's Finished verifies.

### Envelope

32-byte header (`magic, version, type, flags, session_id, sequence,
payload_len, reserved`) used as AEAD AAD; nonce = `session_id[0..4] ‖
sequence`. Sequence numbers are monotonic per session; receivers run an
IPsec-style 64-bit replay window. Decryption happens in place; plaintext
is released to the fabric only after tag verification, and delivered only
after the replay check.

### Discovery and anti-entropy

- `Announce` frames broadcast the signed offer set (≤ 8 offers, each
  48 bytes: cell id, kind, ABI version, required capabilities, interface
  hash, TTL). Per-node announcement sequences reject stale replays.
- On every session establishment, both peers owe a `DirectorySync`
  snapshot (authoritative for the sender). Local publish/withdraw marks
  the same debt. This is the partition-recovery mechanism: after a healed
  partition, re-handshake plus one snapshot restores a consistent
  directory without new announcements.
- Offers carry TTLs; entries expire without refreshes.

## Liveness and recovery state machine

```text
Offline --connect--> HandshakeInitiator --HelloAck--> AwaitingConfirm
Offline <--inspect-- (responder) ------accept_hello-> AwaitingConfirm
AwaitingConfirm --valid Finished--> Secured
Secured --silence ≥ suspect_after--> Suspect
Suspect --any valid frame--> Secured
Secured/Suspect --silence ≥ offline_after--> Offline (session scrubbed)
Offline --backoff expired + reconnect_due--> connect (fresh randomness)
```

On session drop: session keys and handshake secrets are scrubbed
(volatile writes + compiler fence), directory entries for the peer are
marked `Unreachable`, and an exponential backoff (`base << attempts`,
capped) gates reconnection. Supervisors poll `reconnect_due(now)` and
re-dial with fresh randomness.

## Remote cell IPC

`CellIpc` payload: `to_cell ‖ channel ‖ from_cell ‖ body` (body ≤
512 B). The sender must hold the destination in its directory; the
receiver delivers only to locally published cells, otherwise counts a
drop. Delivery is at-most-once (no retransmission in v1); reliability
above this layer is a documented future step.

## What v1 deliberately does not do

- No real socket/NIC integration yet: the `Transport` trait
  (`send_to`/`recv_from` over opaque `u64` addresses) is implemented by
  the caller. Production wiring belongs to `netd` once its UDP path is
  executable in QEMU.
- No multicast/broadcast addressing model beyond caller-chosen addresses.
- No retransmission, ordering, or fragmentation for `CellIpc`.
- No post-compromise security (per-session ephemeral keys only; no
  ratchet).
- No group cells or anycast routing.

## Integration contract

```rust
let mut fabric = CellFabric::new(identity_seed, config);
fabric.add_trusted_key(peer_key)?;           // Pinned mode
fabric.publish_offer(offer)?;                // subject to PublishPolicy
let announce = fabric.make_announcement(now);// broadcast via transport
fabric.connect(node_id, nonce, x25519_secret, now, &mut transport)?;
// per received datagram:
match fabric.process_datagram(source, &mut bytes, now, &mut transport)? {
    true  => fabric.accept_hello(&hello_bytes, nonce, secret, now, &mut transport)?,
    false => {}
}
fabric.tick(now, &mut transport);            // heartbeats, timeouts, sync
fabric.send_cell_ipc(request, payload, now, &mut transport)?;
while let Some(ipc) = fabric.next_incoming() { /* deliver to local cell */ }
while let Some(event) = fabric.next_event()  { /* supervise */ }
```
