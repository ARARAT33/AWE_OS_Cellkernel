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

## P2 — Integrate the cell runtime with the OS service architecture

1. Define a versioned cell syscall contract whose caller identity is derived from the trusted current-process dispatch context; do not accept a user-supplied PID as authority.
2. Integrate `initd`/AWOSA service supervision with CellIds through that syscall contract, avoiding a second independent service lifecycle state machine.
3. Connect CPU tick accounting to the scheduler and memory ownership/accounting to the physical/virtual allocator.
4. Add end-to-end supervisor tests for dependency failure, transitive degradation, fresh-process restart and ordered recovery.

## P3 — Continue the cross-subsystem audit

- Harden netd socket/rule ID exhaustion and validate socket owners against process identities.
- Add real NIC TX/RX and authenticated remote IPC only after local socket/driver contracts are executable.
- Harden storaged volume/snapshot ID exhaustion and validate every volume reference; then implement actual block I/O, persistence and power-loss recovery.
- Fix AYUI signed-coordinate/grid overflow with i64/checked arithmetic and boundary tests before integrating a hardware desktop.
- Reconcile the legacy implementation matrix against `PRODUCT_READINESS.md` and `PRODUCT_GAP_MATRIX.md`.

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

### Validation checkpoint — source commit `2b16188c8472412de3c73a21246d9dfde02ffdf9`

- [x] Workspace compile and `cargo test --workspace`: 439 passed, 0 failed.
- [x] `cargo fmt --all -- --check`.
- [x] Workspace Clippy and UEFI loader quality gate.
- [x] Cloud Security Gate and release-contract gate.
- [x] QEMU ISO boot smoke and dedicated exception-injection self-test (exactly 1 test executed and passed).
- [x] Multi-architecture image build and four AOSIN Linux/Windows GUI/CLI installer jobs.
- [x] VirtIO Runtime #357: formatting, 439 workspace tests, kernel/ELF build, QEMU VirtIO PCI visibility and boot with an attached VirtIO block device passed. This does not prove block read/write or durable filesystem I/O.
- [ ] Latest Quality Gate #1004 remains pending behind a UEFI-loader job in run #1003; the earlier Quality Gate #1001 passed on the same source-code commit.
- [ ] Physical hardware validation remains unavailable.

The final observed workflow evidence and the exact test-filter correction are recorded in `docs/AI-DEVELOPMENT-LOG.md`. Draft PR #58 remains open and unmerged.

### Known integration limits

- CPU and memory accounting APIs are explicit hooks; timer and allocator charging are not yet wired.
- CellManager is local-only; remote cells and authenticated network IPC are not implemented.
- Per-process page tables, Ring-3 entry and memory isolation remain separate P0 kernel work.
- Cell lifecycle confirmation relies on the architecture layer to report actual ProcessManager transitions; this module does not perform a context switch itself.

## P0.2 — Derive local IPC caller identity from the kernel's current process

**Status before this run:** open security gap. create_channel, send, and receive accepted a caller-supplied ProcessId. Equality checks prevented accidental mismatches, but a caller able to choose the value could claim the PID already bound to another cell.

1. Add a kernel-internal ProcessManager::current_process_id() accessor that returns an identity only when the manager's tracked current process still exists and is in Running state.
2. Make CellManager::create_channel, send, and receive derive authority from that accessor. Do not accept a caller PID in any IPC request/API argument.
3. Remove caller_process from CellSendRequest and advance the local cell ABI minor version to 1.
4. Add regression tests for a different current process, no current process, sender/receiver binding, and the existing channel authorization/quota/teardown paths.
5. Document the single-core limitation: before SMP is enabled, replace the single ProcessManager::current authority source with per-CPU current-task identity and interrupt-safe synchronization.
6. Keep syscall exposure deferred until the architecture syscall entry supplies and validates the same trusted current-task context. This change hardens the kernel API; it does not claim a completed userspace Cell syscall ABI.

**Acceptance evidence:** workspace format/check/tests/Clippy plus the existing boot, exception-injection, and VirtIO QEMU jobs on the updated PR head. No physical hardware claim.

## P0.2 implementation checkpoint — trusted local IPC caller identity

- [x] Added a kernel-internal ProcessManager accessor that exposes only the tracked process whose descriptor remains Running.
- [x] Removed caller_process from CellSendRequest and from create_channel/receive arguments; send/create/receive now derive caller identity from the ProcessManager.
- [x] Added regression coverage for non-current callers and for IPC attempted with no tracked current process.
- [x] Advanced the local cell ABI minor version to 1 and documented the API contract.
- [ ] Keep syscall dispatch integration, per-CPU/SMP identity, object-capability handles, and runtime context-switch verification as separate follow-up gates.

## P0.3 — Unify process authority with real x86_64 execution

Repository audit confirms that the current `ProcessManager::mark_running` has no production call site. `ExecutionCore` owns a separate scheduler/current-process model, `process::dispatch_runtime` only prepares a switch frame, and `SyscallContext` mutates a borrowed descriptor without a validated current-task/address-space context. These are separate foundations, not one integrated process runtime.

1. Choose one authoritative process/scheduler state model; do not maintain independent current-task state in `ProcessManager`, `ExecutionCore`, and `Dispatcher` without a defined synchronization contract.
2. Connect the x86_64 context-switch completion path to the authoritative model. Publish the new current process only after the hardware switch completes; clear current only after the switch to idle completes.
3. Bind CellManager lifecycle and IPC to that authoritative runtime and a trusted syscall dispatch context. Keep IPC fail-closed until this exists.
4. Implement per-process address spaces and validated user mappings before enabling Ring 3 or dereferencing syscall user pointers.
5. Add QEMU tests that execute at least two real process contexts, yield/block/exit/reap them, verify current-process identity across switches, and exercise cell IPC end-to-end.
6. Add timer-driven preemption only after context save/restore, interrupt masking, and per-CPU state are proven.

**Do not claim CellManager is boot-integrated or that user-space IPC is operational until these gates pass.**