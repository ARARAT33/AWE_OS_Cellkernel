# CellKernel hardening plan — 2026-10-23

This plan is being executed directly on `main`; no feature branch or pull request is used.

## Priority 1 — process/context identity integrity
- Reject registration when a process descriptor ID does not match its CPU context ID.
- Ensure a rejected registration does not consume a process-table slot or alter the process count.
- Add regression coverage for the mismatch and the subsequent valid registration.

## Priority 2 — scheduler lifecycle invariants
- Keep the existing run-queue removal and FIFO wraparound tests.
- Inspect future failures for stale queue entries and process-state/queue disagreement before changing scheduler policy.

## Priority 3 — reproducible validation
- Run the repository's CI workflows triggered by the main-branch commit.
- Report formatting, build, tests, security checks, and QEMU/boot results separately; do not equate CI success with hardware validation.

## Scope and safety
- Preserve all existing files and user changes.
- Commit only the focused code guard, regression test, and this plan.
- If any validation cannot run, state that explicitly and retain the failing or unverified milestone as open.
