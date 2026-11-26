# AWE_OS CellKernel — Project Status

**Project track:** AWE development project #2  
**Status:** Active development / preserved for continued work  
**Date:** 2026-11-25  
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

## Current implementation evidence (2026-11-25)

- Run 19 (2026-11-25): added `kernel/src/memory/boot_direct_map.rs`, a no-allocation boot gate that fails closed with the new `KernelBootStatus::DirectMapWindowExceeded` when any `MEMORY_REGION_USABLE` range of the boot memory map exceeds the 64 TiB bounded direct-map window; `kernel_entry` now runs the gate before any allocator or page-table state exists. The `kernel-selftest` boot path gained `boot_kernel_root_probe`, which builds a real supervisor-only kernel-half root through the merged `KernelPageTableBuilder` over `TableArena => PhysicalTablePool` and validates it by readback, never publishing it (no CR3, no TLB, no Ring 3).

- PR #89 (`feat/boot-direct-map-gate-2026-11-25`) remains open against `main`. Its first CI attempts exposed compile errors and a QEMU probe ownership issue; follow-up commits corrected the imports/types, exhaustive boot-status handling, pool-capacity inference, and `OwnedTableFrames` release lifecycle. The latest exact head must still pass the full required CI set before merge; no CR3 activation, TLB switch, or Ring 3 is claimed.

- Run 17 (2026-11-24): added `kernel/src/memory/kernel_page_table_build.rs`, a supervisor-only kernel-half page-table builder over the validated `kernel_map` region contract (kernel-half-only installs, no USER bits, planned-region bounds, W^X), plus direct-map window fit validation. Host tests only; nothing is activated at boot, and no CR3 switch, TLB invalidation, or Ring 3 change is claimed.

- PR #78 (checked physical user-copy contract), PRs #79-#81 (reservation-aware frame allocation and boot-owned range reservation), and PR #84 (reserved `TableArena` backing a real `PhysicalTablePool`) are merged into `main`.
- PR #86 merge commit: `bd47eca67259945c044454dfe31b09212e8a90b5` (the current `main` history also includes the Run 16 status/log documentation commits).
- PR #85 (Run 14) was merged after its head `c1dc74ea4fea706d11fefc71773233adcea03a63` passed **16/16 GitHub check-runs**; PR #84 exact head `4f189c916b491a199ab0edef48d478c3ddbe6df4` also passed **16/16 check-runs** before that. The main-head workspace CI, Quality Gate, Bootable Image Pipeline, VirtIO Runtime, Cloud Security Gate, Multi-Architecture Images, AOSIN installers and scheduled branch-integration audit also completed successfully on 2026-11-22/23.
- The live `PhysicalFrameAllocator` reserves boot-owned ranges before its first allocation. The `kernel-selftest` x86_64 boot path can reserve a 16-frame table arena below the proven 1 GiB bootstrap identity map and exercise a real `PhysicalTablePool` over those pages: allocation, zeroing, table read/write, release, foreign-frame rejection and reuse/rezeroing. PR #85 also adds a non-allocating contiguous-run preview that skips reserved ranges, so the probe does not mistake the first boot-owned page for a usable arena start.
- Runtime evidence added in PR #85: the QEMU exception-injection self-test passed on code head `d4b65c1341129735516a05910c143e82b6fe0b48` with assertions for both `[cell:tables] probe result: PASS` and `foreign-rejected=1`. This validates the bounded self-test arena path only. The latest formatter follow-up `0ea2fbb` is formatting-only; the full CI suite must still pass on the latest PR head before merge.
- PR #86 merged into `main` at `bd47eca67259945c044454dfe31b09212e8a90b5` from exact PR head `32387a44460cab52215d5981a6fd0cd94d178a18` after **9/9 workflow families passed**. `kernel/src/memory/kernel_map.rs` now defines the bounded 64 TiB direct-map window, kernel text, heap and per-CPU virtual regions, canonical-address checks, half-open region lookup, a const pairwise validator, and a compile-time assertion over the shipped layout. It has 14 host tests.
- Run 16 resolved the initial `Option<VirtualAddress>` assertion trait-bound errors, removed a redundant `FrameReserver` test import, hardened `is_kernel_half()` against noncanonical addresses, and added real overlap, unordered, noncanonical, overflow and region-boundary regression tests. A Clippy `identity_op` finding in the overflow-test fixture was fixed without changing production logic.
- Exact-head CI evidence on `32387a44460cab52215d5981a6fd0cd94d178a18`: Quality Gate, CI Pipeline, Clippy, CI Diagnostics, Bootable Image Pipeline, VirtIO Runtime, Multi-Architecture Images, Cloud Security and AOSIN Cross-Platform Installers all concluded success. Diagnostics reported 443 `aweos-kernel` tests and 4 `kernel-bin` tests passing; workspace formatting/checks, QEMU exception-injection self-test, QEMU boot with a VirtIO block device, multi-architecture image builds and all four Linux/Windows GUI/CLI installer builds passed.
- **Scope limit:** the merged higher-half module is a validated layout contract only. It does not install higher-half page tables, write CR3, invalidate TLB entries, establish a production direct map, or prove Ring 3/process isolation. The QEMU VirtIO result proves the existing boot/transport smoke path, not durable block I/O or filesystem persistence.
- QEMU x86_64 evidence from the scheduler milestone remains: PIT/timer interrupts drive a three-kernel-thread preemptive scheduler. These are ring-0 threads sharing the kernel address space, not isolated userspace processes.
- Memory: early x86_64 paging and CR3 bootstrap exist. A bounded `AddressSpace` contract, four-level PML4 -> PDPT -> PD -> PT builder, ownership/rollback primitives, and a physical table pool over real frames exist. The pool's current boot integration is a validation probe, not a retained production allocator.
- The checked user-copy helper validates a complete user range before page-bounded access through a supplied `PhysicalAccess` backend and caps each request at 1 MiB. It is not connected to a verified live user-memory backend.
- Storage: bounded block-device, GPT, VFS, journaling and RAM-disk contracts/tests exist. QEMU VirtIO PCI presence and successful boot do not prove block request/completion, read-after-write, flush or durable persistence.
- Networking: protocol/transport contracts exist; a real NIC-backed network path and multi-node CellNet runtime are not proven.
- Architecture: current boot-image workflows build x86_64 BIOS/Multiboot and UEFI artifacts. ARM64 and RISC-V are not runtime-validated.

### Explicitly not complete

The live monotonic `PhysicalFrameAllocator` derives and batch-reserves the kernel image, BootInfo structure, memory-map descriptors, framebuffer and RSDP page before its first frame probe. PR #84 adds a reserved `TableArena` and a real `PhysicalTablePool` probe under the x86_64 `kernel-selftest` configuration; the probe is not retained as the production allocator. The normal boot path still does not install a permanent verified direct map or keep a reclaiming table-frame pool alive. Per-process CR3 switching, TLB invalidation/shootdown, integrated page-table lifetime/reclamation, Ring 3 entry and QEMU user/kernel isolation are not enabled. The scheduler remains single-core and kernel-thread-only. The checked user-copy helper is a contract over a supplied backend, not proof of live user-memory isolation. VirtIO PCI visibility and successful boot do not prove block I/O or durable storage. Do not describe the OS as having production process isolation, persistent VirtIO storage, a production network stack, or ARM64/RISC-V support.

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

1. Run 19 (2026-11-25) added the no-allocation boot direct-map gate (`kernel/src/memory/boot_direct_map.rs`, `KernelBootStatus::DirectMapWindowExceeded`) and the `kernel-selftest` QEMU kernel-half root probe (`boot_kernel_root_probe`). Still open: pre-activation validation of the full built root, the CR3-switch/TLB-invalidation contract with isolation tests, process-root integration, and the TableArena production lifetime.
2. Keep the current `TableArena` as a bounded self-test resource. Define a permanent, QEMU-proven direct-map window and allocator lifetime before retaining a production table-frame pool.
3. Resolve the lower-half bootstrap identity-map conflict with user mappings by defining the kernel's permanent higher-half mapping strategy.
4. Build each process root with supervisor-only kernel mappings preserved; define root/table ownership and teardown before activating a process address space.
5. Implement a controlled CR3 switch and matching local TLB invalidation policy; add failure-path and QEMU isolation tests before any Ring 3 entry.
6. Connect checked user-copy to a verified physical-access backend and test mapped/unmapped, read-only, cross-page and fault cases in QEMU.
7. Implement actual VirtIO block request/completion, read-after-write, flush and power-loss recovery tests.
8. Connect the network contracts to a real NIC/UDP transport and test two independent nodes, reconnect and partition behavior.
9. Add real ARM64 and RISC-V builds and runtime tests before claiming those ports.

## Long-term objective

Build AWE_OS incrementally into a real, reproducible, security-oriented operating system while keeping CellKernel small, modular, auditable, and suitable for continued development over multiple project cycles.
