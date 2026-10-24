# CellKernel hardening cycle — 2026-10-24

This plan is based on the current `main` tree at `bed1b2d17848624db69f58fbb0d4d6a61dc9104e`. All implementation work stays on `main`; no feature branches or pull requests.

## Prioritized work

1. **P0 — Remove fabricated scheduler contexts.** `ProcessManager::context_snapshot` currently fills empty slots with a synthetic context using `ProcessId(u64::MAX)`. Because process IDs are plain `u64`, that sentinel can collide with a real process and let dispatch select a fake context. Pass optional contexts through scheduler dispatch and only match registered entries.
2. **P0 — Enforce process admission invariants.** Registration should accept only a fresh `Created` descriptor with a valid, non-reserved ID and a valid matching CPU context. Failed registration must not consume a slot or change the manager count.
3. **P1 — Add regression tests.** Cover sparse context arrays, a process whose ID is `u64::MAX`, invalid initial states, reserved IDs, and existing FIFO/exit behavior.
4. **P1 — Validate evidence.** Run the available CI suite after the code commits; distinguish unit/static checks from QEMU runtime evidence. Do not claim tests that were not executed.

## Acceptance criteria

- Empty slots cannot impersonate a real process context.
- Invalid registrations leave the process table unchanged.
- Scheduler only dispatches an explicitly registered, valid context matching the queued process ID.
- Changes and commits are made directly on `main`, preserving the current branch head as parent.
- If the GitHub write or CI path blocks a step, record the blocker rather than bypassing it.
