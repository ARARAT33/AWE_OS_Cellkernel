# CellKernel Execution Plan — 2026-10-22

## Scope
Stabilize the privileged process lifecycle and scheduler queue before adding more kernel features. This plan is tracked on the repository's main branch.

## Prioritized tasks
1. **Queue/lifecycle consistency:** add a fixed-capacity scheduler queue removal operation that handles ring-buffer wraparound; remove processes from the ready queue when transitioning to Running or Exited.
2. **Failure atomicity:** transition a process to Runnable only after enqueue succeeds, so a full queue cannot leave registry state inconsistent.
3. **Regression tests:** cover queue removal in normal and wrapped layouts, Running transition cleanup, and ensuring an exited process cannot be dispatched.
4. **Validation:** run rustfmt, workspace check/tests, Clippy, and QEMU boot/runtime CI where available. Record only actual outcomes; do not infer boot or hardware success from compilation.
5. **Follow-up:** resume implementation once the repository code-write operation is available; keep all changes directly on main and never force-push.

## Evidence log
- Initial remote inspection: main branch head `8e6a9aa14cefa2c78d365802f3186a5dab64cb26`; repository metadata confirms push permission. Remote repository APIs do not expose a local working tree, so local uncommitted edits cannot be assessed here.
- Initial code review found that ProcessManager transitions state before enqueue and that exit does not remove a queued ID. These are the focused fixes for this slice.
- Plan commit: `d320fdf5a529f7fedf1dcf3fd9f1a1bc4db9a8cc` on main.
- GitHub Actions run `39809173034` (AWEOS VirtIO Runtime, 2026-10-22): formatting passed; kernel compilation completed; the product-core test command failed because `product_core::qemu_automated_boot_smoke_test` timed out after 31.03 seconds. Overall result: 9 tests passed, 1 failed. Subsequent kernel ELF validation, QEMU boot-image build and VirtIO boot steps were skipped. No artifacts were uploaded.
- Code update attempts for `kernel/src/process/scheduler.rs` and `kernel/src/process/mod.rs` were rejected by the GitHub tool's safety checks. No kernel source or test code was changed in this run. The blocker was recorded rather than bypassed through a different write route.
- Validation status: rustfmt passed in the remote workflow; kernel compilation completed there; product-core suite failed as noted. Workspace-wide tests, Clippy and a successful QEMU boot were not established.
