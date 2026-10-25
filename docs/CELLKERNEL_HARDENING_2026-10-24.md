# CellKernel hardening cycle — 2026-10-25

This plan is based on the current `main` tree at `d55cda8a5c2beb9c30c7f5ef96191efdb0d4c9ca`. All implementation work stays directly on `main`; do not create feature branches or pull requests.

## Prioritized work

1. **P0 — Prevent process-ID allocator wraparound.** `ProcessTable::allocate_id` currently uses `AtomicU64::fetch_add`; at exhaustion it wraps through the reserved ID 0 and can eventually issue duplicate IDs. Replace this with a non-wrapping compare/exchange allocator that returns an explicit exhaustion result, and add boundary regression tests.
2. **P1 — Verify call-site compatibility.** Search the repository for allocator consumers before changing its return type; update every real call site and tests as needed.
3. **P1 — Process lifecycle and slot reclamation.** Review the manager's exit/reap behavior and fixed-capacity slot reuse. Only introduce reclamation if it can preserve the intended observable `Exited` state and cannot free a live context.
4. **P1 — Validate evidence.** Run formatting, workspace tests, Clippy, and available QEMU boot/ VirtIO workflows after the code change. Clearly distinguish current-run evidence from prior successful CI.

## Acceptance criteria

- ID 0 remains reserved and is never returned as a valid allocated ID.
- Every successful allocation is unique, including the final `u64::MAX` ID.
- Exhaustion is explicit and does not wrap back to an earlier ID.
- All changes and commits are made directly on `main`, preserving its current head as parent.
- If a write or CI action is blocked, report the blocker instead of bypassing it.
