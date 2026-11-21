# AWE_OS CellKernel — Project Status

**Project track:** AWE development project #2  
**Status:** Active development / preserved for continued work  
**Date:** 2026-11-21  
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

## Current implementation evidence (2026-11-21)

- PR #78 (checked physical user-copy layer) is merged into `main`.
- PR #79 (reservation-aware bitmap frame allocation) and PR #80 (boot-owned range derivation and all-or-nothing reservation) are both merged into `main`. The current `main` head is `b16abe9c55135240393b9689d0cab54504de0681`.
- PR #81, `feat/live-boot-memory-reservations-2026-11-21`, is still a draft and is **not merged**. Its exact head `17b54cc64eb07bec95aa3922597c4f934808efc0` passed **15/15 check-runs** (0 failures), including workspace check/tests/format, Clippy, UEFI loader, Multi-Architecture Images, Bootable Image Pipeline, exception-injection QEMU self-test, VirtIO Runtime, Cloud Security/Release and all four AOSIN installer jobs.
- The PR #81 workspace test output explicitly passed the new physical-reservation tests: intersecting-frame exclusion, atomic capacity failure, rejection of late reservations, live-allocator boot-owned reservation, and checked kernel-image bounds.
- QEMU evidence on the PR #81 head: the bootable ISO smoke test reached `AWEOS: kernel is alive`; the exception-injection self-test passed. The VirtIO runtime workflow also passed its test suite and confirmed a VirtIO PCI device is visible while the kernel reaches RUNNING. That does **not** prove block read/write, flush or durable persistence.
- QEMU x86_64 evidence from the previous scheduler milestone remains: the kernel reaches RUNNING and PIT/timer interrupts drive a 3-kernel-thread preemptive scheduler. This is kernel-thread scheduling evidence, not userspace process isolation.
- Memory: early x86_64 paging and CR3 bootstrap exist. A bounded `AddressSpace` contract, four-level PML4 → PDPT → PD → PT builder, ownership/rollback layer and `PhysicalTablePool` over real physical frames exist. The physical pool is host-tested but is not instantiated by the live boot path.
- PR #81 adds a bounded reservation ledger to the live monotonic `PhysicalFrameAllocator`, wires boot-owned range reservation into `kernel_entry` before its first frame probe, and obtains the kernel image bounds from linker symbols. The allocator is still local/monotonic and is not yet connected to `PhysicalTablePool`.
- The checked physical user-copy helper exists: it validates the full user range before invoking a page-bounded `PhysicalAccess` backend and caps a request at 1 MiB. It is not yet connected to a live boot direct map or a production physical-access backend.
- Storage: bounded block-device, GPT, VFS, journaling and RAM-disk contracts/tests exist. QEMU exposes VirtIO PCI and boots with the device attached, but actual VirtIO block request/completion, read-after-write and durable persistence are not proven.
- Networking: protocol/transport contracts exist; a real NIC-backed network path and multi-node CellNet runtime are not proven.
- Architecture: current boot-image workflows build x86_64 BIOS/Multiboot and UEFI artifacts. ARM64 and RISC-V are not runtime-validated.

### Explicitly not complete

PR #81 now constructs the live monotonic `PhysicalFrameAllocator`, derives and batch-reserves the kernel image, BootInfo structure, memory-map descriptors, framebuffer and RSDP page, and performs that reservation before the first frame probe. The x86_64 Multiboot/PVH kernel image range comes from linker-defined bounds. This path passed the PR QEMU boot smoke test. The allocator is still discarded after the probe: it has no frame release/reclamation and is not yet the global allocator or the backing store for `PhysicalTablePool`. No verified direct-map window is installed for the pool. Per-process CR3 switching, TLB invalidation/shootdown, integrated page-table lifetime/reclamation, Ring 3 entry and QEMU user/kernel isolation are not enabled. The scheduler remains single-core and kernel-thread-only. The checked user-copy helper is a contract over a supplied backend, not proof of live user-memory isolation. VirtIO PCI visibility and successful boot do not prove block I/O or durable storage. Do not describe the OS as having production process isolation, persistent VirtIO storage, a production network stack, or ARM64/RISC-V support.

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

1. Review PR #81 before merge; its current draft head passed 15/15 checks, but it remains unmerged and must not be treated as part of `main`.
2. Design a `DirectMapWindow` only over physical pages proven reachable in the active page tables. Do not assume that an identity mapping or an addressable physical range is a permanent direct map.
3. Instantiate `PhysicalTablePool` using the reservation-aware allocator; prove that table frames are allocated, zeroed, mapped, owned and safely released, with rollback tests.
4. Build each process root with supervisor-only kernel mappings preserved; define root/table ownership and teardown before activating a process address space.
5. Implement a controlled CR3 switch and the matching local TLB invalidation policy; add failure-path and QEMU isolation tests before any Ring 3 entry.
6. Connect the checked user-copy layer to a verified physical-access backend and test mapped/unmapped, read-only, cross-page and fault cases in QEMU.
7. Implement actual VirtIO block request/completion, read-after-write, flush and power-loss recovery tests.
8. Connect the network contracts to a real NIC/UDP transport and test two independent nodes, reconnect and partition behavior.
9. Add real ARM64 and RISC-V builds and runtime tests before claiming those ports.

## Long-term objective

Build AWE_OS incrementally into a real, reproducible, security-oriented operating system while keeping CellKernel small, modular, auditable, and suitable for continued development over multiple project cycles.
