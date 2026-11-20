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

## Current implementation evidence (2026-11-20)

- PR #78 (checked physical user-copy layer) is merged into `main`; the current main head is `d781f94344b700021ac5ce9bf2ce13f84426b1f2`.
- On that exact main head, the seven latest push workflows completed successfully: Quality Gate, Bootable Image Pipeline, AOSIN installers, Multi-Architecture Images, Cloud Security Gate, CI Pipeline and VirtIO Runtime. The Quality Gate's formatting, workspace check, workspace tests, Clippy and UEFI-loader steps passed.
- QEMU x86_64 evidence remains: boot protocol validated; kernel reaches `RUNNING`; PIT/timer interrupts drive a 3-kernel-thread preemptive scheduler. The captured run reported `switches=44` and 15 dispatches per thread. This is kernel-thread scheduling evidence, not userspace process isolation.
- Memory: early x86_64 paging and CR3 bootstrap exist. A bounded `AddressSpace` contract, four-level PML4 → PDPT → PD → PT builder, ownership/rollback layer and `PhysicalTablePool` over real physical frames exist. The physical pool is host-tested but is not instantiated by the live boot path.
- The checked physical user-copy helper now exists: it validates the full user range before invoking a page-bounded `PhysicalAccess` backend and caps a request at 1 MiB. It is not yet connected to a live boot direct map or a production physical-access backend.
- Storage: bounded block-device, GPT, VFS, journaling and RAM-disk contracts/tests exist. QEMU exposes VirtIO PCI and boots with the device attached, but actual VirtIO block request/completion, read-after-write and durable persistence are not proven.
- Networking: protocol/transport contracts exist; a real NIC-backed network path and multi-node CellNet runtime are not proven.
- Architecture: current boot-image workflows build `x86_64-unknown-none` and `x86_64-unknown-uefi` only. ARM64 and RISC-V are not runtime-validated.

### Explicitly not complete

The live boot path still does not construct a reservation-aware frame allocator from the boot memory map, install a verified direct-map window for the physical table pool, or instantiate that pool. Per-process CR3 switching, TLB invalidation/shootdown, integrated page-table lifetime/reclamation, Ring 3 entry and QEMU user/kernel isolation are not enabled. The scheduler remains single-core and kernel-thread-only. The checked user-copy helper is a contract over a supplied backend, not proof of live user-memory isolation. VirtIO device presence and successful boot do not prove block I/O or durable storage. Do not describe the OS as having production process isolation, persistent VirtIO storage, a production network stack, or ARM64/RISC-V support.

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

1. Complete PR #79 validation for the reservation-aware `BitmapFrameAllocator`; merge only after the exact final head passes the full PR suite.
2. Derive reservations from the validated boot handoff and linker symbols: kernel image, loader-owned pages, `BootInfo`, memory-map descriptors, bootstrap PML4/PDPT/PD and boot stacks. Keep unknown ownership reserved; never assume a region is free solely because it is addressable.
3. Construct a `DirectMapWindow` only over physical pages proven reachable in the active page tables, then instantiate `PhysicalTablePool` over the reservation-aware allocator. Add QEMU evidence that allocated table frames are real, zeroed, writable and safely reclaimable.
4. Build each process root with the supervisor-only kernel mappings preserved; define root/table ownership and teardown before activating a process address space.
5. Implement a controlled CR3 switch and the matching local TLB invalidation policy; add failure-path and QEMU isolation tests before any Ring 3 entry.
6. Connect the checked user-copy layer to a verified physical-access backend and test mapped/unmapped, read-only, cross-page and fault cases in QEMU.
7. Implement actual VirtIO block request/completion, read-after-write, flush and power-loss recovery tests.
8. Connect the network contracts to a real NIC/UDP transport and test two independent nodes, reconnect and partition behavior.
9. Add real ARM64 and RISC-V builds and runtime tests before claiming those ports.

## Long-term objective

Build AWE_OS incrementally into a real, reproducible, security-oriented operating system while keeping CellKernel small, modular, auditable, and suitable for continued development over multiple project cycles.
