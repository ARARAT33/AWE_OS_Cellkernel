# AWE_OS CellKernel — prioritized development plan (2026-11-07)

**Base:** `main` at `2f76706592bd19ff43f9a08e5ab788eb191afe0c`
**Working branch:** `main` only. All source, test and documentation commits in this run are made directly to `main`; no feature branch or PR is created.
**Scope:** security-first hardening of the existing syscall boundary. This is not a claim that userspace, process isolation or a complete OS is operational.

## Repository audit checkpoint

- Repository default branch is `main`; the current remote head is `2f767065`. The branch list and latest commits were inspected before changes. The repository is not archived and the linked GitHub account has push/admin permission.
- The latest source integration is the 2026-11-06 merge of PR #58: bounded local CellManager runtime and Cell ABI 1.1 trusted local IPC caller identity.
- The latest seven main-branch workflows on this head completed successfully: CI Pipeline, Quality Gate, Cloud Security Gate, VirtIO Runtime, Bootable Image Pipeline, Multi-Architecture Images and AOSIN installers. The VirtIO workflow proves QEMU device visibility and kernel boot with the device attached, not block read/write or filesystem durability. The boot workflow's QEMU ISO smoke and exception-injection self-test both passed.
- Open PRs #55, #53, #52, #50, #48, #36 and #34 are stale relative to the current architecture; they are left untouched.
- Source inspection confirms a serious syscall-boundary gap: `SyscallContext` directly mutates a borrowed process descriptor, reports fake IPC/map results, returns a hard-coded initrd marker, and dereferences caller-supplied Read/Write pointers without a validated user address space. The syscall entry itself is still a bootstrap-only handshake and is not wired to this dispatcher.
- The local CellManager remains bounded and test-backed, but `ProcessManager`, `ExecutionCore` and the architecture switch path are not yet one authoritative execution runtime. No production `mark_running` call site was found.

## P0 — Remove unsafe placeholder syscall behavior (this run)

1. Add an explicit `ERR_NOT_IMPLEMENTED` result to the versioned syscall ABI.
2. Make every recognized general syscall fail closed until the kernel has a trusted current-task context, a real process/scheduler integration, and validated user address spaces.
3. Remove all raw user-pointer reads/writes and fake IPC, mapping, initrd and process-state side effects from `SyscallContext::dispatch`.
4. Add regression tests proving every known syscall returns the not-implemented result without mutating process state or resource budgets, including invalid/unmapped pointer values.
5. Document that the dispatcher is deliberately unavailable to Ring 3; do not enable a general userspace syscall path as part of this change.

## P1 — Unify execution authority

1. Select one authoritative current-task/process model across `ProcessManager`, `ExecutionCore` and `Dispatcher`.
2. Publish current-process identity only after the architecture context switch has completed; clear it only after switching away.
3. Add QEMU coverage for at least two real contexts, yield/block/exit/reap and identity changes.
4. Keep CellManager IPC and cell syscalls fail-closed until this integration is proven.

## P1 — Establish safe address-space boundaries

1. Implement per-process page-table roots and explicit supervisor/user mapping contracts.
2. Validate user ranges with checked arithmetic and mapping/permission checks before any copy.
3. Add guard-page, permission, overlap, overflow and cross-address-space tests.
4. Do not enter Ring 3 for general applications until the above is exercised in QEMU.

## P2 — Continue subsystem integration after P0/P1

- Wire scheduler timer accounting and process quotas.
- Add APIC/IOAPIC EOI/routing and per-CPU state before SMP.
- Implement actual VirtIO block read/write and persistence/recovery tests.
- Implement real NIC TX/RX before authenticated remote-cell IPC.
- Harden netd/storaged ID exhaustion and resource ownership; fix AYUI coordinate/grid overflow.
- Reconcile implementation matrices against executable evidence.

## Validation policy

Run the repository's real CI on `main`: workspace check/tests, rustfmt, Clippy, UEFI loader, boot-image/QEMU smoke, exception self-test, VirtIO QEMU boot, security and multi-architecture image jobs. Report exact outcomes. A compile or image build is not proof of hardware support, process isolation, network I/O or storage durability. No physical hardware validation is claimed.


## Implementation checkpoint — 2026-11-07

- [x] P0.1: added `ERR_NOT_IMPLEMENTED` and made all nine recognized general syscall IDs fail closed.
- [x] P0.2: removed unchecked Read/Write pointer access and fake IPC, mapping, initrd and process-state side effects.
- [x] P0.3: added regression coverage for hostile pointer/length inputs and verified that process state and all resource budgets remain unchanged.
- [x] P0.4: documented the bootstrap/general syscall distinction and the Ring-3 integration gate.
- [x] P0.5: committed directly to `main` at `576d35dc2415132caf9b85f189918f582062b8f9`.
- [x] CI validation: all seven main-push workflows passed (CI #379, Quality #1019, Cloud Security #964, VirtIO #372, Bootable Image #990, Multi-Arch #293, AOSIN #331). Exact evidence and scope limits are recorded in `docs/AI-DEVELOPMENT-LOG.md`.
- [ ] P1 remains open: unify ProcessManager/ExecutionCore and connect authoritative identity to completed x86_64 context switches.
- [ ] P1 remains open: per-process address spaces and validated user-memory access before enabling general Ring-3 applications.

This checkpoint does not claim a complete syscall runtime, process isolation, durable storage, network I/O, ARM64/RISC-V runtime support or physical-hardware validation.
