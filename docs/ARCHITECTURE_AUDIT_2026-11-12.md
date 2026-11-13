# CellKernel Architecture Audit — 2026-11-12

**Method:** full-tree clone of `main` (head `0d33de0`), local build and
test execution on the pinned nightly toolchain, bare-metal target build,
CI workflow reading. Percentages in historical documents were ignored as
instructed; only compiling code, passing tests and CI evidence count.

## Verified baseline (executed locally, 2026-11-12)

- `cargo check --workspace`: clean.
- `cargo test --workspace --no-fail-fast`: **507 tests, 0 failures**
  (481 pre-existing + 26 added in this session).
- `cargo build --release -p aweos-kernel-bin --target x86_64-unknown-none`:
  clean; produces a Multiboot1/2 + Xen PVH kernel image.
- `cargo check --manifest-path boot/aweloader/Cargo.toml --target
  x86_64-unknown-uefi`: clean.
- `cargo clippy --workspace --all-targets -- -D warnings
  -A dead_code -A unused_attributes -A bad_asm_style -A unused_imports`:
  clean (CI gate flags).
- CI history (`docs/AI-DEVELOPMENT-LOG.md`): 16/16 check-runs green on the
  PR #67 head, including the QEMU ISO smoke test
  ("AWEOS: kernel is alive"), exception-injection self-test, VirtIO
  runtime and multi-arch image builds.

## Real vs skeleton — honest map

| Subsystem | State | Evidence / gap |
|---|---|---|
| Boot (GRUB/Multiboot2, PVH note, bootstrap paging) | **Real, boot-proven in CI** | QEMU smoke asserts serial banner |
| x86_64 kernel entry, GDT/IDT/IST, exception self-test | **Real** | CI exception-injection job |
| Memory (paging, frame allocator, boot mapper) | **Kernel-space real** | Per-process page tables + user copy still missing (documented next priority) |
| Process/scheduler (ProcessManager, ExecutionCore, x86_64 context switch) | **Real, unified (PR #67)** | Not yet driven by a live timer-interrupt dispatch loop |
| Local cell runtime (`kernel::cell`, ABI 1.1) | **Real** | Local only; no per-cell memory isolation |
| IPC (kernel + service transport) | **Contract-real** | Bounded local rings; no Ring-3 consumers yet |
| Capabilities/security model | **Contract-real** | Enforcement end-to-end pending userspace |
| Network stack (ARP/IPv4/UDP-TCP headers, sockets, firewall) | **Model-real** | No NIC TX/RX on hardware/QEMU yet |
| **Distributed cells (this directive's §V)** | **Was absent; this session adds the first real layer** | `awe-cellnetd` (see below) |
| Storage (GPT/VFS/journal/ramdisk) | **Model-real** | No device I/O, no persistence (storaged docs admit this) |
| VirtIO/block | **Contract + CI exercise** | Real request/completion persistence pending |
| Services (nexus, netd, storaged, securityd, driverd, initd, identityd, appd, update, platformd, ayui) | **Logic-real, host-tested** | All are bounded `no_std` libs; none are running as booted userspace processes yet |
| `.asd` / `.awos` package admission | **Real parsers + validators + tests** | Signature trust-root wiring pending |
| AYUI | **Compositor/shell model** | No framebuffer/input path on real boot yet |
| ARM64 / RISC-V | **Skeleton boot stubs** | As docs say; no runtime claims |
| Compat layers (Linux/Windows/Android/Wlin) | **Header/dispatch models** | Far from execution |
| Continuum (cross-device mesh) | **Toy** | 16-node clipboard sync; superseded conceptually by cellnetd |

## What this session added (directive §V, first executable slice)

- `awe-securityd::crypto`: X25519 (with contributory check), HKDF-SHA256,
  ChaCha20-Poly1305 AEAD — validated against RFC 7748 / RFC 5869 /
  RFC 8439 vectors.
- `awe-cellnetd` (new service crate): signed cell announcements,
  pinned/TOFU trust, mutually authenticated 4-flight handshake
  (Ed25519 + X25519 + HKDF), ChaCha20-Poly1305 session envelopes with
  anti-replay window, remote cell IPC gated by publish policy,
  suspect/offline liveness, exponential-backoff reconnect, and
  DirectorySync anti-entropy for partition recovery. 22 integration/unit
  tests including tamper, replay, spoofing, low-order-key, stale-entry
  and partition-heal scenarios. Spec: `docs/CELLNET_ARCHITECTURE.md`.

## Recommended priority order (evidence-first)

1. **Per-process page-table roots + checked user copy** (already the
   documented next item) — the gate for any Ring-3 execution.
2. **Live dispatch loop on the timer interrupt** driving the unified
   ProcessManager, with CPU accounting feeding cell quotas.
3. **VirtIO block request/completion + read-after-write persistence** in
   QEMU, then GPT/VFS durability on top.
4. **netd UDP path in QEMU**, then bind `cellnetd`'s `Transport` to it
   (loopback-only today) and run two QEMU instances as a 2-node mesh.
5. **Init/service manager boot integration**: start storaged/netd/
   securityd/cellnetd as real cells under AWOSA supervision.
6. Package trust roots: wire `.asd`/`.awos` signature verification to
   securityd keys (pinned trust policy exists in cellnetd; reuse it).
7. AYUI framebuffer + PS/2 input path; only then desktop claims.
8. ARM64/RISC-V: keep as stubs until x86_64 userspace exists.

## Anti-goals reaffirmed

- No readiness percentages; gates are runtime evidence.
- "Cell" is not a folder name: cells have identity, lifecycle, quotas,
  capabilities and (now, remotely) authenticated channels.
- No invented cryptography anywhere in the mesh layer.
