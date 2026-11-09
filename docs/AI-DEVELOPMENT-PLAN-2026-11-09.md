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
