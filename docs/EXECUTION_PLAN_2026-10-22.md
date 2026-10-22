# CellKernel Execution Plan — 2026-10-22

## Scope
Stabilize the privileged process lifecycle and scheduler queue before adding more kernel features. This plan is tracked on the repository's main branch.

## Prioritized tasks
1. **Queue/lifecycle consistency:** add a fixed-capacity scheduler queue removal operation that handles ring-buffer wraparound; remove processes from the ready queue when transitioning to Running or Exited.
2. **Failure atomicity:** transition a process to Runnable only after enqueue succeeds, so a full queue cannot leave registry state inconsistent.
3. **Regression tests:** cover queue removal in normal and wrapped layouts, Running transition cleanup, and ensuring an exited process cannot be dispatched.
4. **Validation:** run rustfmt, workspace check/tests, Clippy, and QEMU boot/runtime CI where available. Record only actual outcomes; do not infer boot or hardware success from compilation.
5. **Follow-up:** investigate any failures and fix them directly on main, preserving existing changes and never force-pushing.

## Evidence log
- Initial remote inspection: main branch head `8e6a9aa14cefa2c78d365802f3186a5dab64cb26`; repository metadata confirms push permission. Remote repository APIs do not expose a local working tree, so local uncommitted edits cannot be assessed here.
- Initial code review found that ProcessManager transitions state before enqueue and that exit does not remove a queued ID. These are the focused fixes for this slice.
- Validation status: pending implementation and CI; no claim of passing tests or boot is made yet.
