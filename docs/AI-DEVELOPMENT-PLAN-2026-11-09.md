# CellKernel engineering plan — 2026-11-09

## Branch and baseline

- All changes in this session are committed directly to `main`; no feature branch or pull request is used.
- Repository: `ARARAT33/AWE_OS_Cellkernel`.
- Audited main head: `46c849f159c6cc8cbd8db6d1f8146bb9379999b8` (2026-11-08 21:30:55 UTC).
- The remote `main` ref is readable and unprotected. GitHub's repository API exposes committed state only; it cannot inspect uncommitted files in an external/local clone. No remote uncommitted state exists in Git's commit model, and this work will use non-force, parent-checked commits.
- The workspace has 28 listed Rust members across boot, kernel, services, apps, tools and product-core tests; the kernel crate is `no_std`.
- Existing architecture includes a bounded local CellManager, process/scheduler/context-switch primitives, capability tables, paging/boot guards, x86_64 interrupt/IST code, storage/network contracts, AWOSA service crates and AYUI.
- The CellManager is explicitly not a memory-isolation boundary. The general syscall dispatcher intentionally returns `ERR_NOT_IMPLEMENTED` until trusted task identity and validated user address spaces exist.

## Prioritized execution plan

### P0 — Fix x86_64 canonical-address validation consistently

The audit found a correctness defect repeated across the bootloader and kernel: checking only `address >> 48 == 0 || == 0xffff` accepts non-canonical addresses where bit 47 is not sign-extended (for example `0x0000_8000_0000_0000` and `0xffff_0000_0000_0000`). Such values can pass boot, paging, interrupt, IDT, IST or context validation and later fault when used by the CPU.

- Use the architectural 48-bit canonical rule: bits 63:48 must equal bit 47.
- Apply it to virtual-address validation, boot entry and stack validation, saved interrupt RIP/RSP, IDT handler addresses, IST stack tops, scheduler contexts, process CPU contexts and switch frames.
- Keep the documented four-level paging model explicit; do not silently claim LA57 support.
- Add boundary tests for the lowest/highest lower-half canonical values, both high-half boundaries, and both non-canonical sign-extension gaps.
- Keep process and kernel contexts compatible with canonical high-half kernel addresses; user-range policy remains a separate permission/address-space check.

### P1 — Validate the changed code

- Run `cargo fmt --all -- --check`.
- Run `cargo test --workspace --all-targets` and `cargo clippy --workspace --all-targets -- -D warnings -A dead_code -A unused_attributes -A bad_asm_style -A unused_imports`.
- Trigger and inspect the repository CI workflows on the resulting main commit where the GitHub integration permits it.
- Inspect actual QEMU boot/exception and VirtIO job steps/logs; distinguish successful image boot/device visibility from real block I/O or hardware support.

### P2 — Preserve the next kernel-integration gates

- Add an architecture-owned switch-completion path that acknowledges only the exact pending `DispatchTarget` after execution reaches the target stack.
- Unify `ProcessManager`, `ExecutionCore` and current-task identity rather than maintaining competing authorities.
- Implement per-process page-table roots, user/kernel mapping policy, safe user-copy primitives and capability-checked syscall integration before enabling Ring 3.
- Wire scheduler CPU accounting and allocator memory ownership to CellManager quotas only when the execution/allocator ownership path is explicit and testable.

## Architecture evidence and known limits

- Main's latest recorded Actions runs for CI, Quality Gate, Cloud Security, Bootable Image, VirtIO Runtime, Multi-Architecture Images and AOSIN installers all completed successfully on the audited head.
- The workspace CI log reports 323 kernel tests passing and 11 product-core tests passing; the dedicated boot workflow ran QEMU ISO smoke and exception-injection steps successfully.
- VirtIO CI proved that QEMU exposes a VirtIO PCI block device and that the kernel boots with it attached. It did not prove durable block reads/writes or filesystem power-loss recovery.
- No Ring-3 process isolation, per-process page tables, full network stack, remote authenticated cell IPC, production USB/GPU/Wi-Fi/audio support, or physical-hardware compatibility is claimed.
- Open PRs #55, #53, #52, #50 and #48 are stale relative to the audited main head; this session will not merge or alter them.

## Session completion record

This plan is committed before implementation. The completion record, exact files changed, test outcomes, CI run IDs and remaining blockers will be appended to `docs/AI-DEVELOPMENT-LOG.md` after validation.

---

## Continuation run — 2026-11-09 (x86_64 dispatch completion)

### Baseline and repository safety

- Audited the repository default branch main at 6025b264aa80efd605d6eb5517c9ccb3f6dfc92b (2026-11-09 16:18:18 UTC). The root tree contains 447 entries and 28 Rust workspace members.
- main is the default branch. Existing branch chore/ci-verify-main-2026-11-08 and open PRs #55, #53, #52, #50 and #48 are preserved and will not be checked out, rebased, merged, or deleted.
- All writes in this run are scoped explicitly to main; no feature branch or PR will be created. GitHub's API exposes committed refs only, so it cannot inspect an unrelated local clone's uncommitted worktree. Each update uses the current blob SHA and branch ref; if either changes unexpectedly, stop and re-audit.
- The current main head's latest Actions runs (2026-11-09 16:18 UTC) report successful workspace CI, Quality Gate, Cloud Security, boot image/QEMU, VirtIO QEMU, multi-architecture image builds, and AOSIN installers.

### Prioritized implementation plan

1. **P0 — Architecture-owned dispatch completion.** Extend the x86_64 switch-frame ABI with an optional, validated target-stack completion callback. Add a wrapper that accepts only the exact pending DispatchTarget, invokes ProcessManager::confirm_dispatch after the CPU has loaded the target stack, and fails closed if acknowledgement cannot be committed. Keep the contract explicitly single-core and kernel-address-space-only.
2. **P1 — Real QEMU context-switch regression.** Add a feature-gated two-stack round-trip self-test: boot context → fresh process stack → completion acknowledgement → return to boot stack. Verify the process becomes Running only after the target-stack callback, then exit, acknowledge idle, and reap it. This is not Ring-3 or SMP validation.
3. **P2 — Validate.** Run workspace format/tests/Clippy and the dedicated QEMU boot/exception-injection workflow. Inspect job steps/logs and report each result separately. Do not infer physical-hardware, durable storage, or NIC functionality from these checks.
4. **P3 — Record evidence.** Update docs/AI-DEVELOPMENT-LOG.md with exact commit/run IDs, changed files, failures, limitations, and next priorities.

### Audit findings driving P0

- ProcessManager now retains a single pending DispatchTarget, checks exact target equality, and rejects premature reaping.
- The x86_64 assembly currently saves/restores the callee-saved register frame and jumps to the target RIP, but it has no target-stack callback. No production path calls confirm_dispatch; the kernel boot path explicitly states that a scheduler is not yet running.
- The existing QEMU test validates exception/IST handling, not a real task-to-task context switch. A passing ISO smoke test proves early boot reaches its serial marker only; it does not prove scheduler execution.

### Hard limits

- The completion callback is a kernel-only, single-core mechanism. It must not be presented as an address-space isolation boundary or SMP-safe scheduler.
- Ring 3 remains disabled until per-process page-table roots, supervisor-only kernel mappings, checked user-copy primitives, and capability-checked syscalls exist.
- Storage persistence, network TX/RX, authenticated remote cell IPC, and physical-hardware support remain separate unvalidated gates.