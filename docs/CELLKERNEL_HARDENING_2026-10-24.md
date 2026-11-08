# CellKernel hardening cycle — 2026-10-26

This plan is based on the inspected `main` head at `f4ad1a67122cbd22ff950c434cc186e1d62d9306`. All implementation work, tests, documentation, and commits stay directly on `main`; do not create branches or pull requests.

## Prioritized work

1. **P0 — Process lifecycle and capacity recovery.** Inspect how `ProcessManager` retains exited processes and contexts. Add an explicit, safe reclamation operation only if it can prove the process is exited and is no longer queued/current; ensure it releases the fixed-capacity slot without changing other processes.
2. **P1 — Regression coverage.** Test that live/runnable processes cannot be reclaimed, exited processes can be reclaimed exactly once, IDs cannot be reclaimed while still scheduled, and a freed slot can be reused without stale context or duplicate registry entries.
3. **P1 — Cross-layer consistency.** Review `ExecutionCore` and scheduler behavior for equivalent lifecycle leaks or stale queue membership; make only changes supported by the current implementation and tests.
4. **P1 — Validation evidence.** Run available formatting, workspace tests, Clippy, and QEMU boot/VirtIO checks. Record actual results and do not infer success from earlier CI runs.

## Acceptance criteria

- Reclamation is explicit and fail-closed; a process is not freed while runnable, running, or still queued.
- Reclamation clears descriptor and context together and updates capacity accounting exactly once.
- A reused registry slot contains no stale context or scheduling entry.
- All writes are direct commits on the current `main` branch, with no branch/PR creation and no force update.
- If repository writes or test execution are blocked, preserve current state and report the blocker precisely.
