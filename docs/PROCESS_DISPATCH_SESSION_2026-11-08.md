# ProcessManager dispatch-acknowledgement session record — 2026-11-08

This file temporarily carries the 2026-11-08 ProcessManager dispatch-acknowledgement
session entry for `docs/AI-DEVELOPMENT-LOG.md`. Because PR #60 (storaged hardening)
appended its session entry to the same end-of-file region while this PR was in
review, the development log on this branch is kept byte-identical to `main` so the
pull request merges cleanly. The session entry below will be re-appended to the
development log on `main` immediately after this PR merges (as a documentation-only
commit), preserving the single chronological session history.

---

## Session 2026-11-08 — ProcessManager dispatch acknowledgement hardening

### Repository audit

- Started from `main` at `1710e4e9c78ab6869afc64dd66d2ab2e60f92d6f`; PR #60 (`fix(storaged): harden metadata IDs and owner consistency`) is separate and remains open/draft.
- PR #60 head `73e325259890529d331c010f3470bacb3f5368b9` had 16/16 GitHub check-runs successful on 2026-11-08. Workspace CI, Quality Gate/Clippy/UEFI, Cloud Security, VirtIO/QEMU, ISO boot, exception-injection, multi-architecture image build and all four installer jobs passed. These are PR #60 results, not validation of this new branch.
- Focused code audit found three ProcessManager state-boundary gaps: `next_dispatch` dequeued a target without retaining a pending token; `mark_running(ProcessId)` accepted any runnable ID rather than the exact selected frame; and `confirm_switched_to_idle()` cleared current identity without checking which process had exited.
- The x86_64 assembly switch routine saves/restores a frame and jumps to its saved RIP. Its current caller path still does not invoke a Rust completion acknowledgement on the target stack.

### Changes on branch `fix/process-dispatch-ack-2026-11-08`

- `kernel/src/process/mod.rs`
  - Added a single pending `DispatchTarget` slot. A second dequeue fails with `DispatchPending` until the target is confirmed or cancelled.
  - Replaced arbitrary-ID `mark_running` with `confirm_dispatch(target)`, which accepts only the exact target/frame returned by the preceding `next_dispatch` and consumes the acknowledgement once.
  - Added `cancel_dispatch(target)` so a rejected architecture switch returns the runnable target to the queue before clearing the pending token.
  - Prevented exit/reap from silently invalidating a pending target.
  - Made idle acknowledgement require the expected current ProcessId and an Exited descriptor; stale, wrong-process and premature acknowledgements fail closed.
  - Added regression tests for exact-target matching, single-use acknowledgement, pending-dequeue rejection, cancellation, pending exit/reap rejection and idle-state validation.
- `kernel/src/cell.rs`
  - Updated lifecycle/IPC tests to drive ProcessManager through its actual bounded round-robin dequeue/acknowledgement path instead of directly setting arbitrary process IDs to Running.
  - Updated idle transitions to identify the exact exited process; the idle-identity test now exits the current process before acknowledging idle.
- `docs/CELL_RUNTIME.md`
  - Documented the pending dispatch token and exact idle acknowledgement, including the remaining architecture integration boundary.

### Validation

- Source changes and regression tests are committed on this branch.
- No local Rust/QEMU result is claimed in this run.
- GitHub Actions validation is pending for this branch. Do not merge until the full applicable CI/QEMU/security matrix passes on the final PR head.
- PR #60's green checks are recorded above only as a separate prerequisite/result; they do not validate this branch.

### Remaining limitations

- `ProcessManager::confirm_dispatch` is not yet called by a production x86_64 switch-completion trampoline. The new token contract is not proof of hardware execution.
- `ExecutionCore` and its scheduler remain a separate state model; no authoritative unified process runtime is claimed.
- No per-process page-table roots, validated user-memory copy path, general Ring-3 execution, APIC/IOAPIC preemption or timer-driven scheduler integration is introduced here.
- The single-current-process model remains single-core only. SMP requires per-CPU current-task identity and synchronization.
- No block-device I/O, durable filesystem, NIC TX/RX, authenticated remote IPC or physical-hardware validation is claimed.

### Next priorities

1. Wire an architecture-owned x86_64 switch-completion trampoline that acknowledges the exact pending target on the target stack; prove two real contexts in QEMU before using current-task identity for security decisions.
2. Unify `ProcessManager` and `ExecutionCore` without maintaining duplicate process/scheduler state.
3. Implement per-process page-table roots and checked user-memory copy operations before enabling Ring 3.
4. Bind CellManager and service supervision to the authoritative process lifecycle.
5. Continue real VirtIO block read/write, flush, persistence and recovery work; then NIC TX/RX and authenticated remote IPC.


### CI feedback and correction — 2026-11-08

- First PR-head run on `748d86f81e0b3e0f9723e2659e9475f4d5bb10ee` compiled the workspace but the kernel test target reported **321 passed, 2 failed**. The failures were test-expectation/transition-model issues, not compiler errors:
  1. The cancellation test incorrectly expected a cancelled target to jump ahead of an already queued process. The scheduler is FIFO, so cancellation correctly re-enqueues the target at the tail; the test now verifies the intervening runnable process is dispatched first.
  2. The graceful dependency-stop test tried to acknowledge an idle switch for a provider that was Runnable/non-current. That process is removed from the run queue on exit and can be reaped without a current-to-idle acknowledgement; the unnecessary acknowledgement was removed.
- The workspace test step and dependent Cloud Security/VirtIO test steps failed because those two kernel tests failed. Clippy, UEFI loader, ISO boot, exception-injection, multi-architecture image build and installer jobs passed on that first PR head. VirtIO QEMU runtime was not considered validated because its test gate failed before the QEMU step.
- Corrections are committed separately on the same PR branch. A new full CI run is required; no passing result is claimed until it completes on the corrected head.


### Final CI validation after corrections — 2026-11-08

**Validated PR-head commit:** `f7fd9702a30fe89d9dfdb544c94fd561fee62174`
**Workflow run window:** 2026-11-08 15:36–15:37 UTC
**Result:** 15/15 commit check-runs completed successfully.

- AWEOS CI Pipeline: passed. `cargo check --workspace` and `cargo test --workspace` completed successfully; the `aweos-kernel` library test suite reported **323 passed, 0 failed**.
- AWEOS Quality Gate: passed; Workspace Clippy and UEFI loader jobs passed.
- AWEOS Cloud Security Gate and cloud release-contract gate: passed.
- AWEOS Bootable Image Pipeline: ISO QEMU smoke reported **ISO Boot Successful**; the dedicated exception-injection self-test executed exactly one test and passed.
- AWEOS VirtIO Runtime: QEMU exposed `virtio-blk-pci` and the kernel emitted `AWEOS: kernel is alive` with the device attached. This proves boot with the VirtIO block device present, not block read/write, flush or durable filesystem persistence.
- AWEOS Multi-Architecture Images: build workflow passed. This is image-build evidence, not ARM64/RISC-V runtime proof.
- AOSIN Cross-Platform Installers: all four Linux/Windows GUI/CLI build jobs passed.
- Rustfmt-fix and Clippy diagnostics jobs passed.

The initial PR-head run `748d86f` had two failing tests; both expectations were corrected and the complete workflow matrix was rerun successfully on `f7fd9702a30fe89d9dfdb544c94fd561fee62174`. The PR remains draft/unmerged pending review. The next required engineering step is still a real x86_64 target-stack switch-completion trampoline; the passing unit/QEMU boot checks do not establish a working multi-process context-switch runtime.
