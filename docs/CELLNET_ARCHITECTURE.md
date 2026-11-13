# AWE_OS CellNet — Network Cell Architecture (ABI 0.2)

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
   offer table (8), incoming queue (16), replay table (64 single-use
   capability grants).
2. **No invented cryptography.** All primitives live in `awe-securityd`
   (the Security Cell): Ed25519 (identity/signatures and capability-token
   signatures), X25519 (key agreement), HKDF-SHA256 (key schedule),
   ChaCha20-Poly1305 (AEAD), SHA-256/HMAC-SHA256 (transcript/confirmation).
3. **No trusted network.** Every control-plane frame is signed; every
   data-plane frame is encrypted and authenticated; a 64-bit sliding
   anti-replay window rejects duplicates.
4. **No hidden RNG.** Handshake nonces, ephemeral X25519 secrets, and
   capability-grant sequences are supplied by the caller, keeping the crate
   deterministic, testable, and free of a `getrandom` dependency on
   `no_std`.
5. **Publish-only visibility, capability-gated access.** A node advertises
   exactly the cells its `PublishPolicy` permits (kind mask + capability
   mask, deny-all by default). Remote peers can send IPC only to cells that
   are currently published. When `FabricConfig::require_ipc_caps` is set
   (enforced mode), every `CellIpc` must additionally carry a valid,
   unexpired, single-use Ed25519 capability token whose mask covers the
   destination cell's advertised `required_caps`; the grant-free API fails
   closed and forged tokens are dropped by the receiver.

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
| 7 | CellIpc | AEAD (session) | 32 B hdr + ≤649 B payload + 16 B tag |
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

## Capability authorization (ABI 0.2)

Remote `CellIpc` calls can be gated by signed capability tokens issued by
a **CapabilityAuthority** (in practice: `securityd`'s Ed25519 authority).
The token is a 49-byte signed payload plus a 64-byte Ed25519 signature
(113 bytes on the wire):

```text
version(1) ‖ grantee_node(8) ‖ cell_id(8) ‖ caps(8)
        ‖ not_before(8) ‖ not_after(8) ‖ sequence(8) ‖ signature(64)
```

Semantics:

- The grant names the **grantee node** and the **source cell**; a token
  minted for another node or cell is worthless.
- `caps` is the capability mask granted to that cell; the receiving
  fabric verifies `grant.caps ⊇ destination.required_caps` against the
  destination's currently published offer.
- The validity window (`not_before`/`not_after`) is checked against the
  sender's clock at send time; expired grants are refused before sealing.
- `sequence` is monotonic and non-wrapping per
  `(grantee_node, caller cell)`. Each stream keeps a 64-value sliding
  replay window: duplicates and values older than the window are rejected,
  while unique out-of-order values inside the window are accepted once.
  Up to 64 streams are tracked; when the stream table is full, new streams
  fail closed but existing streams continue to advance.
- Issuer keys are pinned per fabric (`set_capability_authority`); tokens
  from any other issuer fail signature verification.

Enforcement is bidirectional and fail-closed: the sender API
(`send_cell_ipc_authorized`) verifies the grant before spending it, the
legacy grant-free `send_cell_ipc` returns `CapabilityDenied` in enforced
mode, and the receiver independently re-verifies the token against its own
authority pin, the destination's `required_caps`, and its replay table.
A compromised sender cannot smuggle an IPC call through with a zero or
forged token. In development mode (`FabricConfig::development()`),
enforcement is off and the token slot is ignored for compatibility.

### Replay-state restart boundary

Replay windows currently live in the `CellFabric` instance's memory. They
protect against duplicate and stale grants during that instance's lifetime,
but are reset when `cellnetd` restarts. This is **not** cross-reboot replay
protection. Before production deployment, persist replay high-water/window
state atomically or bind grants to a fresh, unpredictable authority epoch
supplied by the platform's secure random source and reject prior epochs.
Until that integration exists, callers must use short-lived grants and must
not claim durable single-use semantics across restart.

## Remote cell IPC

`CellIpc` payload: `24 B header (to_cell ‖ channel ‖ from_cell) ‖
113 B capability token ‖ body` (body ≤ 512 B, total payload ≤ 649 B).
The sender must hold the destination in its directory; the receiver
delivers only to locally published cells and only with a valid token in
enforced mode, otherwise counts a drop. Delivery is at-most-once (no
retransmission in v1); reliability above this layer is a documented future
step.

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
- No distributed capability-delegation chains yet: grants are minted
  directly by the pinned authority; delegation/attenuation is future work.

## Integration contract

```rust
let mut fabric = CellFabric::new(identity_seed, config);
fabric.add_trusted_key(peer_key)?;           // Pinned mode
fabric.set_capability_authority(securityd_public_key);
fabric.publish_offer(offer)?;                // subject to PublishPolicy
let announce = fabric.make_announcement(now);// broadcast via transport
fabric.connect(node_id, nonce, x25519_secret, now, &mut transport)?;
// per received datagram:
match fabric.process_datagram(source, &mut bytes, now, &mut transport)? {
    true  => fabric.accept_hello(&hello_bytes, nonce, secret, now, &mut transport)?,
    false => {}
}
fabric.tick(now, &mut transport);            // heartbeats, timeouts, sync
// capability-gated remote IPC (enforced mode):
fabric.send_cell_ipc_authorized(request, &grant, payload, now, &mut transport)?;
while let Some(ipc) = fabric.next_incoming() { /* deliver to local cell */ }
while let Some(event) = fabric.next_event()  { /* supervise */ }
```
