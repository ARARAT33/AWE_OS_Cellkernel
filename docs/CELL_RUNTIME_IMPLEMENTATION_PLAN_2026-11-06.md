# AWE_OS CellKernel — Cell Runtime Implementation Plan (2026-11-06)

**Base:** `main` at `499ec2b4397c73ead52a0831c3ffc0e8379714b9`  
**Working branch:** `feat/cell-runtime-core-2026-11-06`  
**Scope:** first executable, bounded local Cell Runtime milestone. This is not a claim of process memory isolation or remote-cell support.

## Repository audit and current evidence

- The main workspace is Rust 2024 and contains a `no_std` kernel crate plus boot, service, application, tooling and product-core test crates.
- The kernel exposes `ProcessManager`, monotonic `ProcessId`, process state/resource budgets, a fixed-capacity scheduler, capability bitsets, and bounded IPC mailboxes.
- Existing `ServiceDescriptor`/`ServiceRegistry` are service metadata; they do not implement cell identity, dependency admission, runtime ownership or cell-level recovery.
- The current QEMU CI evidence covers boot and selected VirtIO/exception paths, but not a general cell runtime.
- The default branch already includes PR #57 and its post-merge CI record. Stale open PRs #55/#53/#52/#50/#48 have old base commits and are not reused as the implementation base.

## P0 — Implement a real bounded local Cell Runtime

1. Add a versioned cell ABI, non-zero non-reused `CellId`, explicit `CellKind`, lifecycle and health states, recovery policy, resource quota/usage accounting, versioned interfaces and bounded declared dependencies.
2. Add a fixed-capacity `CellManager` that admits only existing `Created` processes whose process budgets and granted kernel capabilities satisfy the cell contract.
3. Make `begin_start` enqueue the bound process through `ProcessManager`; publish `Running` only after the process manager reports the process as `Running`.
4. Make stop/failure paths transition the bound process through `ProcessManager`, defer reclamation until its scheduler/current-context safety checks pass, and never reuse process or cell identities.
5. Enforce local cell IPC with explicit sender allowlists, receiver identity, interface ABI compatibility, bounded queues and per-sender IPC quota accounting. Tear down endpoints/messages when a cell stops or fails.
6. Quarantine cells after the configured restart/failure threshold. A restart must bind a new, independently registered process; do not resurrect a stale process context.

## P0.1 — Dependency lifecycle integrity (hardening pass)

- Reject graceful provider shutdown while a dependent cell is Starting or Running.
- Recheck dependencies at the final Running confirmation boundary, not only when scheduling starts.
- When a provider fails unexpectedly, mark active dependents Degraded transitively through the acyclic dependency graph.
- Require dependencies to be both Running and Healthy before a dependent starts, confirms Running or returns to Healthy.
- Bind health updates to a still-existing, non-exited process descriptor.
- Add regression tests for shutdown ordering, transitive failure propagation/recovery, exited process health updates and the begin-start/confirm-running race.

## P1 — Regression and validation

- Unit tests for identity exhaustion, registration rejection, quotas, capability admission, dependency ordering, lifecycle/process-state synchronization, process reaping, restart limits, channel authorization, ABI mismatch, queue backpressure and endpoint teardown.
- Run rustfmt, kernel/workspace tests, Clippy, and boot/QEMU workflows through GitHub Actions. Fix only failures reproduced by the workflow logs.
- Update `docs/AI-DEVELOPMENT-LOG.md` with actual commit, changed files and real workflow outcomes.

## Explicit non-goals / limitations of this milestone

- No per-process page tables, Ring-3 isolation or proof of memory isolation is introduced here.
- No remote/distributed cells, discovery, network transport or cryptographic peer authentication is introduced here.
- Capability admission uses the existing kernel capability bitset; it is not a replacement for object-capability handles or syscall enforcement.
- CPU and memory usage counters are explicit accounting hooks; they are not yet automatically charged by scheduler ticks or page allocation. IPC sends are charged by the local cell channel.
- Cell state confirmation remains an orchestration boundary: the caller must report process execution/exit only after the architecture path has actually completed the relevant switch.

## Definition of done for this branch

A cell may be declared `Running` only when its bound process is actually marked `Running`; cell IPC must reject unauthorized or non-running peers; all fixed-capacity/identity/quota failures must fail closed; tests and CI results must be recorded honestly. The broader OS remains in development.

## Implementation checkpoint (2026-11-06)

The first local runtime implementation is now present in `kernel/src/cell.rs` and exported by `kernel/src/lib.rs`:

- [x] Versioned cell ABI, monotonic CellId/channel IDs, cell kinds/states/health, bounded dependencies/interfaces, recovery policy and resource usage.
- [x] Admission against an existing Created ProcessManager process, budget and required-capability subset.
- [x] ProcessManager-backed start confirmation, stop/exit, deferred reap, failure quarantine and fresh-process rebind.
- [x] Local fixed-capacity IPC channels with Security broker caller binding, sender CellId allowlists, interface compatibility, caller ProcessId checks, message quota and teardown.
- [x] Regression tests for identity exhaustion, admission failures, process lifecycle, dependency ordering, transitive health degradation/recovery, exited-process health rejection, quota accounting, caller spoofing, IPC authorization and failure teardown.
- [x] ABI and limitation documentation in `docs/CELL_RUNTIME.md`.

### Pending validation

The branch PR has been opened as draft PR #58. GitHub Actions are running. No Rust test, Clippy, boot or QEMU result is considered passed until the corresponding workflow/job completes and its logs are inspected.

### Known integration limits

- CPU and memory accounting APIs are explicit hooks; timer and allocator charging are not yet wired.
- CellManager is local-only; remote cells and authenticated network IPC are not implemented.
- Per-process page tables, Ring-3 entry and memory isolation remain separate P0 kernel work.
- Cell lifecycle confirmation relies on the architecture layer to report actual ProcessManager transitions; this module does not perform a context switch itself.
