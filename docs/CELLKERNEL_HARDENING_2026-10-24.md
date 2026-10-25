# CellKernel hardening cycle — 2026-10-25

This plan is based on the current `main` tree at `f7037500f41dcdb172694cb981e28bdb6c27095c`. All implementation work stays directly on `main`; do not create feature branches or pull requests.

## Prioritized work

1. **P0 — Prevent process-ID reuse on exhaustion.** The atomic `ProcessTable::allocate_id` previously wrapped through reserved ID 0; it now uses a compare/exchange allocator with explicit exhaustion. Apply the same rule to `ExecutionCore::create_process`, whose `saturating_add` could repeatedly issue `u64::MAX`.
2. **P1 — Preserve lifecycle/queue invariants.** Review exit/reap behavior and ensure a queue-capacity failure cannot leave a process marked runnable but absent from the run queue. Preserve observable `Exited` state until explicit safe reclamation.
3. **P1 — Regression tests.** Exercise the final valid IDs, repeated allocation after exhaustion, and lifecycle behavior after failed scheduling operations.
4. **P1 — Validate evidence.** Run formatting, workspace tests, Clippy, and available QEMU boot/VirtIO workflows after changes. Distinguish current-run evidence from earlier CI runs.

## Acceptance criteria

- ID 0 remains reserved and is never returned as a valid allocated ID.
- Every successful allocation is unique, including the final `u64::MAX` ID; exhaustion is explicit and does not wrap or saturate into duplicate IDs.
- Queue-capacity errors do not leave process state inconsistent with scheduler membership.
- All changes and commits are made directly on `main`, preserving its current head as parent.
- If a write or CI action is blocked, report the blocker instead of bypassing it.
