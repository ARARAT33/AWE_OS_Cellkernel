# AWE_OS CellKernel — Project Status

**Project track:** AWE development project #2  
**Status:** Active development / preserved for continued work  
**Date:** 2026-11-16  
**Primary branch:** `main`  
**Preserved snapshot:** `archive/pre-restructure-2026-08-27`

## Purpose

This repository is a long-term development workspace for AWE_OS and its CellKernel foundation. It is intentionally kept as a living project rather than presented as a finished operating system.

The repository should remain buildable, testable, and easy to resume after periods of inactivity. New work should extend the existing architecture instead of replacing the project with an unrelated prototype.

## Current direction

The immediate focus is **engineering the bootable and testable AWE_OS core**, with emphasis on:

1. x86_64 boot and deterministic kernel entry
2. memory, paging, interrupts and CPU execution
3. processes, scheduling, syscalls, IPC and capabilities
4. hardware/service boundaries and VirtIO/QEMU execution
5. storage and networking runtime integration
6. AWOSA native runtime and package trust
7. AYUI and first-party userspace
8. recovery, validation and release evidence

## Current implementation evidence (2026-11-16)

- PR #71 (per-process address-space contract and x86_64 four-level user page-table builder) is merged into `main` as `991473ee1bd41d7578b5de9df66666a4f19a2730`.
- The latest validated code head passed **564 tests across 57 test binaries** with zero failures. The nine substantive PR workflows passed; the final main-branch CI, Quality Gate, Cloud Security Gate, boot image, VirtIO QEMU, multi-architecture-image and installer workflows also passed after documentation updates.
- QEMU x86_64 evidence: boot protocol validated; kernel reaches `RUNNING`; PIT/timer interrupts drive a 3-kernel-thread preemptive scheduler. The captured run reported `switches=44` and 15 dispatches per thread. This is kernel-thread scheduling evidence, not userspace process isolation.
- Memory: early x86_64 paging and CR3 bootstrap exist. A bounded `AddressSpace` contract and four-level PML4 → PDPT → PD → PT builder now exist, but the builder is currently exercised through a test-only `TableMemory` backend.
- Storage: bounded block-device, GPT, VFS, journaling and RAM-disk contracts/tests exist. QEMU exposes VirtIO PCI, but actual VirtIO block request/completion, read-after-write and durable persistence are not yet proven.
- Networking: protocol/transport contracts exist; a real NIC-backed network path and multi-node CellNet runtime are not yet proven.
- Architecture: current boot-image workflows build `x86_64-unknown-none` and `x86_64-unknown-uefi` only. ARM64 and RISC-V are not yet validated.

### Explicitly not complete

No live frame-backed `TableMemory` backend, per-process CR3 switching, TLB invalidation/shootdown, page-table teardown/reference counting, physical checked user-copy, Ring 3 entry or QEMU user/kernel isolation runtime is enabled. The scheduler remains single-core and kernel-thread-only. Do not describe the OS as having production process isolation, persistent VirtIO storage, a production network stack, or ARM64/RISC-V support.

## Project-state rules

- `main` is the active development line.
- Historical or superseded documentation must not be treated as current implementation evidence.
- Planned features must be labeled as planned, experimental, or pending validation.
- A passing static build is not equivalent to a bootable or production-ready OS.
- Runtime evidence should be attached to the milestone it validates.
- Security boundaries and ABI contracts must remain explicit and versioned.

## Documentation source of truth

Use this file for the high-level project state. Use `PRODUCT_READINESS.md` for release-readiness gates and `docs/AWE_OS_100_PERCENT_MASTER_PLAN.md` for the long-range implementation plan.

Older progress/evidence documents are retained as engineering history. Their dates and scope must be respected; they do not automatically describe the current state of `main`.

## Preservation

Before the documentation/project-state restructuring on 2026-08-27, the `main` tree was preserved as:

`archive/pre-restructure-2026-08-27`

That branch is the recovery point for the previous repository state and should not be deleted while the new development direction is being established.

## Next development cycle

Prioritize the next missing runtime boundary, not additional feature labels:

1. Wire `TableMemory` to owned physical frames and a verified physical-to-virtual mapping; define rollback, lifetime and teardown.
2. Ensure each process root receives supervisor-only kernel mappings, then implement safe CR3 switching and TLB invalidation.
3. Implement physical checked user-copy and QEMU user/kernel isolation tests before any Ring 3 entry.
4. Implement actual VirtIO block request/completion, read-after-write, flush and power-loss recovery tests.
5. Connect the network contracts to a real NIC/UDP transport and test two independent nodes, reconnect and partition behavior.
6. Add real ARM64 and RISC-V builds and runtime tests before claiming those ports.

## Long-term objective

Build AWE_OS incrementally into a real, reproducible, security-oriented operating system while keeping CellKernel small, modular, auditable, and suitable for continued development over multiple project cycles.
