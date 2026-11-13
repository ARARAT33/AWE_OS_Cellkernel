# CellKernel Advanced Runtime Architecture

**Document type:** Architecture proposal and staged implementation specification  
**Status:** Proposed; not implemented unless separately evidenced by code and tests.  
**Target:** AWE_OS CellKernel  
**Related documents:** `CELL_RUNTIME.md`, `PRODUCT_READINESS.md`, `PRODUCT_GAP_MATRIX.md`, CellNet Fabric architecture in AWENET.

## 1. Vision

CellKernel is intended to manage an operating system as a collection of small, independently governed cells. A cell is a bounded unit of execution with an identity, lifecycle, health state, dependencies, resource budget, versioned interfaces and explicit capabilities.

The long-term direction is not merely to rename processes as cells. The cell abstraction should provide a consistent security and lifecycle boundary for kernel services, drivers, system services, applications and—only after a separate remote-execution security design—distributed work.

The current local Cell Runtime is a bounded process-backed manager. Its existing documentation explicitly states that it does not yet provide per-process page tables, Ring-3 isolation, complete object-capability handles, automatic CPU/memory charging or remote cells. This proposal preserves those limits and describes how to evolve safely.

## 2. Core invariants

1. **Kernel authority is local:** only trusted kernel code may grant or revoke capabilities.
2. **No caller-supplied identity:** syscall authority derives from the current process context, never a PID supplied by userspace.
3. **No false Running state:** a cell is Running only after the scheduler/architecture layer confirms execution of its bound process.
4. **No stale context revival:** a restart uses a fresh process and CPU context.
5. **No identity reuse:** cell, process, channel, capability-handle and request identifiers fail closed on exhaustion.
6. **Bounded resources:** every queue, registry, message, dependency graph, restart loop and telemetry buffer has a configured maximum.
7. **Isolation before exposure:** Ring 3 and user-facing syscalls remain disabled until address-space isolation and checked user-copy are implemented and tested.
8. **Dependency integrity:** providers cannot stop gracefully while active dependents rely on them; provider failure degrades dependents transitively.
9. **Fail closed:** unknown ABI versions, invalid handles, overflow, quota exhaustion and malformed messages return explicit errors.
10. **Remote work is untrusted:** network requests are proposals. Local CellKernel policy decides whether to admit them.

## 3. Cell model

A cell descriptor should evolve toward the following logical structure. This is a design sketch, not a claim that all fields exist in the current Rust type.

```rust
struct CellDescriptor {
    id: CellId,
    generation: u32,
    kind: CellKind,
    lifecycle: CellState,
    health: CellHealth,
    process: ProcessId,
    abi: CellAbi,
    dependencies: BoundedVec<CellDependency>,
    interfaces: BoundedVec<InterfaceVersion>,
    capabilities: CapabilitySet,
    resource_limits: ResourceLimits,
    resource_usage: ResourceUsage,
    restart: RestartPolicy,
    isolation: IsolationProfile,
    provenance: CellProvenance,
}
```

The generation field is useful for handles and stale-reference detection. It must not be used to recycle the underlying global identity. A removed slot may be reused internally only if every external handle includes and validates the generation.

### 3.1 Cell kinds

Suggested initial kinds:
- **Kernel service:** privileged service with narrowly defined kernel interfaces.
- **Driver cell:** hardware-facing code behind a driver-service boundary.
- **System service:** userspace or supervisor-managed service.
- **Application cell:** untrusted application workload.
- **Worker cell:** constrained task execution unit for local or future AWENET tasks.
- **Recovery cell:** isolated diagnostics and recovery tooling with minimal authority.

Kind is descriptive metadata; it must never grant authority by itself. Capabilities and isolation profiles determine what the cell can do.

### 3.2 Lifecycle

The current local lifecycle remains authoritative. Future additions should not introduce parallel, conflicting lifecycle state machines.

```text
Declared -> Admitted -> Starting -> Running -> Stopping -> Stopped
                         |             |
                         v             v
                       Failed       Degraded
                         |
                  RestartBackoff
                         |
                    Admitted
                         |
                    Quarantined
```

Any extension to the state machine must define:
- allowed source and destination states;
- the component authorized to request the transition;
- the event that confirms it;
- rollback behavior;
- persistence requirements;
- observability event and test.

## 4. Living Cells — self-organizing local services

Living Cells describes controlled adaptation of a cell's resource profile and dependency relationships. It does not mean arbitrary self-modification or self-granted privileges.

A cell may:
- report demand and saturation;
- request a resource-budget adjustment;
- advertise optional interfaces;
- request a compatible provider;
- enter degraded mode when an optional dependency disappears;
- restart from a known-good version after failure.

A cell may not:
- grant itself capabilities;
- modify kernel policy;
- silently replace its trusted executable;
- expand its resource ceiling;
- rewrite dependency authority;
- bypass supervisor approval.

### 4.1 Resource negotiation

Each cell reports bounded telemetry over a fixed interval: CPU ticks, resident/committed memory, IPC queue pressure, I/O bytes and health. The supervisor compares observed demand with configured limits and system-wide reserves.

A proposed budget change passes through:
1. request;
2. policy validation;
3. system reserve check;
4. admission-control decision;
5. atomic update at a safe point;
6. audit event.

The budget cannot exceed global policy, and the kernel must enforce the resulting limit. A user-space dashboard is not an enforcement boundary.

## 5. Cell Continuity Capsule

### 5.1 Purpose

Cell Continuity Capsule (CCC) is a versioned checkpoint format intended to restore a cell after a controlled stop, crash or host restart. The first implementation should restore logical application state, not a raw CPU register dump or arbitrary live process image.

A checkpoint is a recovery artifact, not proof that a process can be transparently migrated between CPUs or architectures.

### 5.2 Capsule contents

Proposed logical schema:

```text
CellCapsule {
  format_version,
  cell_id,
  cell_generation,
  executable_digest,
  cell_abi,
  state_schema_id,
  state_schema_version,
  checkpoint_sequence,
  created_at,
  dependency_digests[],
  granted_capability_descriptors[],
  resource_policy_snapshot,
  application_state_blob,
  pending_operation_records[],
  open_channel_recovery_records[],
  integrity_digest,
  signer_key_id,
  signature
}
```

Do not serialize raw kernel pointers, physical addresses, page-table roots, file descriptors as naked integers, kernel locks, interrupt state or architecture-specific stack layouts. These values are invalid after restart and can become security vulnerabilities.

### 5.3 Checkpoint protocol

1. Supervisor asks the cell to prepare a checkpoint.
2. Cell stops accepting new state-changing requests or records them in a bounded journal.
3. Cell reaches a declared safe point.
4. Cell serializes only its versioned logical state.
5. Kernel validates size, schema, ownership and quota.
6. Capsule is written to durable storage using a temporary object and atomic commit protocol.
7. Digest and signature are recorded.
8. Cell resumes or proceeds to shutdown.

If a cell cannot reach a safe point before timeout, the checkpoint is aborted. The kernel must not freeze indefinitely.

### 5.4 Restore protocol

1. Verify capsule signature, digest, format and expiry policy.
2. Verify executable digest and compatible ABI.
3. Validate dependency versions and capabilities against current policy.
4. Allocate a fresh process, fresh address space and fresh kernel objects.
5. Deserialize state into a bounded staging area.
6. Recreate channels only through explicit reauthorization; never trust stale endpoint handles.
7. Reconcile pending operations against durable journals and idempotency keys.
8. Start the process and confirm actual scheduler dispatch.
9. Mark the cell Running only after all required checks pass.

If the executable or state schema is incompatible, use an explicit migration function with a versioned input/output contract, or reject restore. Never guess a migration.

### 5.5 Exactly-once caution

A checkpoint cannot by itself guarantee exactly-once external side effects. A crash can occur after a payment, file write or network request succeeds but before the result is checkpointed. External effects need idempotency keys, transactional APIs or reconciliation. Otherwise the correct guarantee is at-least-once or at-most-once, stated per operation.

## 6. Cell Immune System

### 6.1 Purpose

Cell Immune System is a kernel-supervisor response framework for detecting and containing abnormal cell behavior. It is not a machine-learning agent with authority to rewrite policy. Detection may be heuristic; containment must be deterministic.

### 6.2 Signals

Potential bounded signals:
- repeated invalid syscalls or capability-handle failures;
- resource quota violations;
- abnormal restart frequency;
- IPC queue flooding;
- unexpected dependency access;
- executable digest mismatch;
- memory-safety fault or kernel exception attributed to a cell;
- repeated protocol violations;
- policy-denied filesystem/network/device operations.

Signals have severity, source, timestamp, evidence reference and confidence. A noisy signal must not automatically destroy user data.

### 6.3 Response levels

| Level | Response | Example |
| --- | --- | --- |
| Observe | Record and increase sampling | Unusual but permitted resource demand |
| Throttle | Apply existing quota ceiling | Repeated IPC pressure |
| Restrict | Revoke a scoped capability or deny an operation | Repeated unauthorized device access |
| Isolate | Stop IPC and detach external endpoints | Suspected compromise |
| Quarantine | Prevent restart until review or explicit policy action | Executable integrity failure |
| Recover | Restore from a verified capsule or known-good image | Recoverable service crash |

The kernel performs only actions supported by trusted policy. A cell cannot vote itself out of quarantine. Recovery must use a known-good executable and a fresh process context.

### 6.4 False-positive safety

- Do not infer malicious intent from one resource spike.
- Keep user data separate from executable state.
- Provide an auditable reason code for every containment action.
- Allow policy-defined recovery for non-security failures.
- Security-critical quarantine requires explicit review and must not be auto-cleared by a score.
- The response path itself must be bounded and allocation-safe in kernel context.

## 7. Virtual Hardware Fabric

### 7.1 Purpose

Virtual Hardware Fabric (VHF) provides a capability-checked resource interface so cells request abstract compute or I/O capabilities rather than directly assuming a specific physical device.

A request might ask for:
- CPU time budget;
- memory pages or a bounded memory pool;
- a GPU/accelerator queue;
- block storage operations;
- network socket capability;
- timer service;
- cryptographic accelerator.

VHF is not a promise that any device exists. The kernel or trusted device manager returns a typed resource handle or a clear unavailable error.

### 7.2 Resource handles

Resource handles must be unforgeable within the kernel's object model, scoped to one cell, revocable, generation-checked and limited by operation rights. A raw device index or pointer is not an authority token.

### 7.3 Scheduling

The first implementation should remain local and conservative:
- reserve kernel and recovery capacity;
- enforce per-cell CPU quotas using scheduler accounting;
- enforce memory ownership in the allocator/page tables;
- apply bounded queues and backpressure;
- prevent one cell from starving interrupt, driver or recovery paths.

Cross-device and cross-node scheduling is not part of the local VHF milestone. AWENET can later request work through the Worker Cell contract, but it cannot directly allocate local hardware resources.

## 8. Capability and isolation evolution

The current capability bitset is useful for coarse admission but is not sufficient as a complete object-capability system.

Proposed evolution:
1. Keep the current admission checks as an initial gate.
2. Introduce typed kernel object handles with owner cell, rights, generation and revocation state.
3. Ensure handles cannot be guessed or forged from user input.
4. Validate every syscall against the current process and handle table.
5. Add checked user-copy routines before exposing pointer-bearing syscalls.
6. Implement per-process page-table roots and user/kernel mapping separation.
7. Add guard pages and validate stack, heap and mapped regions.
8. Add DMA/IOMMU policy before untrusted cells can access devices.
9. Add Ring-3 transition and return-path tests only after the above controls exist.

The security boundary must be tested under malformed pointers, integer overflow, stale handles, concurrent revocation, process exit and context-switch races.

## 9. Local-to-remote cell boundary

CellKernel remains the final authority for local admission. AWENET may discover a remote task or propose a worker lease, but the local supervisor must:
- verify the remote sender and manifest signature;
- check local owner opt-in;
- map the requested task class to an allowed local Worker Cell profile;
- reject capabilities outside the profile;
- enforce local resource limits;
- isolate input artifacts and output channels;
- record task evidence without exposing unrelated user identity.

Remote tasks must not become privileged system cells. They must not access kernel services, device control, personal files or secrets unless a specific, narrow capability has been granted by the owner and enforced by the kernel.

A network disconnect does not grant extra local authority. A lease expiry stops or checkpoints the worker according to its task policy.

## 10. Supervision and dependency management

The supervisor owns lifecycle transitions and dependency policy. It should support:
- dependency DAG validation;
- start ordering;
- provider shutdown protection;
- transitive degraded-state propagation;
- ordered recovery;
- restart backoff with jitter;
- restart budgets and quarantine;
- health probes with bounded execution;
- versioned interface compatibility.

A health probe must not be allowed to block the scheduler or hold a kernel lock indefinitely. Probe timeouts and response sizes are fixed.

## 11. Observability and audit

Each cell should expose a bounded status record:
- identity and generation;
- kind and ABI;
- lifecycle and health;
- executable digest;
- dependency summary;
- resource budget and observed usage;
- restart/failure counters;
- capability summary (never secret material);
- checkpoint sequence;
- quarantine reason, if any.

Audit events should be structured, bounded and append-only within the chosen storage model. Log overflow must have an explicit policy and must not block the kernel. High-volume telemetry may be sampled; security events must preserve reason and sequence metadata.

## 12. Rust module boundaries (proposal)

Possible future module decomposition:

```text
kernel/src/cell/
  mod.rs
  descriptor.rs
  lifecycle.rs
  supervisor.rs
  capability.rs
  handles.rs
  resource.rs
  checkpoint.rs
  quarantine.rs
  ipc.rs

services/cell-supervisor/
  policy.rs
  recovery.rs
  health.rs
  persistence.rs

services/worker-runtime/
  manifest.rs
  lease.rs
  sandbox.rs
  evidence.rs
```

Do not split the existing `kernel/src/cell.rs` until there is a measured maintenance or ownership benefit. This is a proposed future layout, not an instruction to create empty modules.

## 13. Implementation roadmap

### P0 — Preserve and verify current runtime
- Keep the existing process-backed CellManager as the source of truth.
- Reconcile current docs with the actual merged implementation.
- Preserve exact dispatch-token acknowledgement semantics.
- Add tests for every lifecycle and dependency invariant.
- Do not claim memory isolation or remote execution.

### P1 — Complete local enforcement
- Connect CPU accounting to scheduler ticks.
- Connect memory accounting to allocator/page ownership.
- Implement checked user-copy and per-process page-table roots.
- Keep Ring 3 disabled until isolation tests pass.
- Define typed object-capability handles and revocation.
- Test context-switch failure, stale handles and concurrent process exit.

### P2 — Supervisor and Living Cells
- Implement resource-demand telemetry with bounded buffers.
- Add supervisor-mediated resource negotiation.
- Add dependency-aware health and restart backoff.
- Ensure adaptive policy cannot expand capabilities or exceed global budgets.
- Provide deterministic simulation tests.

### P3 — Cell Continuity Capsule
- Define canonical capsule encoding and schema versioning.
- Implement safe-point checkpoint hooks for one simple service.
- Persist capsules atomically with digest verification.
- Restore into a fresh process and fresh address space.
- Add crash-injection tests around every persistence boundary.
- Add idempotency/reconciliation for external operations.

### P4 — Cell Immune System
- Add deterministic violation counters and reason codes.
- Add throttle, isolate and quarantine transitions.
- Add safe recovery from a verified image/capsule.
- Fuzz the signal parser and containment state machine.
- Audit false-positive and denial-of-service behavior.

### P5 — Virtual Hardware Fabric
- Define typed resource handles and a local resource broker.
- Integrate CPU, memory, storage, timer and network services one at a time.
- Add capability revocation and quota enforcement.
- Add device-specific isolation only with DMA/IOMMU support where required.

### P6 — AWENET Worker Cell integration
- Implement only after local isolation, syscall enforcement and resource accounting are proven.
- Accept versioned Task DNA manifests through an unprivileged ingress service.
- Map remote task requests to a fixed Worker Cell profile.
- Require explicit local opt-in and bounded leases.
- Validate results before reporting completion.
- Keep economic settlement disabled until independent evidence is reliable.

## 14. Required test matrix

### Identity and lifecycle
- ID exhaustion and non-reuse;
- stale generation handles;
- invalid state transitions;
- process exits during start/stop;
- exact pending-dispatch acknowledgement;
- restart with fresh process and fresh CPU context.

### Capabilities and isolation
- forged, stale and revoked handles;
- unauthorized syscall;
- invalid user pointers and overflow;
- cross-cell memory access;
- guard-page violation;
- concurrent revocation and IPC;
- DMA isolation when device access is introduced.

### Continuity Capsule
- invalid signature and digest;
- oversized and malformed capsule;
- unknown mandatory schema;
- incompatible executable digest;
- incompatible state version;
- truncated storage write;
- crash before/after atomic commit;
- duplicate external operation reconciliation;
- stale IPC endpoint restoration rejection.

### Immune System
- isolated violation;
- repeated violation threshold;
- false-positive spike;
- queue flood;
- executable digest mismatch;
- quarantine persistence;
- recovery only from approved image;
- response path remains bounded under memory pressure.

### Virtual Hardware Fabric
- unavailable device;
- quota exhaustion;
- stale resource handle;
- handle revocation;
- resource starvation;
- kernel reserve preservation;
- device removal during operation.

### AWENET boundary
- unsigned manifest;
- expired lease;
- capability escalation attempt;
- replayed task unit;
- worker disconnect;
- data policy mismatch;
- output digest mismatch;
- remote request cannot invoke privileged cell APIs.

## 15. Performance and safety budgets

Every future feature must define:
- maximum registry entries;
- maximum dependency count and graph depth;
- maximum capsule size and serialization time;
- maximum checkpoint pause;
- maximum message size and queue depth;
- maximum supervisor work per tick;
- maximum recovery attempts and backoff;
- maximum memory reserved for audit and telemetry.

Kernel-path code should avoid unbounded allocation, blocking I/O, unbounded recursion and untrusted parsing. Large serialization and policy evaluation belong in a supervisor/service context, with the kernel enforcing the final bounded result.

## 16. Non-goals

This proposal does not claim:
- transparent migration of arbitrary running processes;
- safe serialization of raw kernel execution state;
- automatic self-modifying kernel behavior;
- machine-learning-controlled privilege grants;
- complete memory isolation in the current milestone;
- remote execution in the current local runtime;
- universal hardware abstraction that makes all devices interchangeable;
- exactly-once external side effects without transactional support;
- security against a fully compromised host kernel.

## 17. Definition of done

A feature is complete only when its code, tests, CI evidence and limitations are recorded. A design document is not implementation evidence. A green unit-test suite is not proof of physical hardware support or memory isolation. A simulated distributed task is not proof of a secure multi-node deployment.

The central rule is: **cells may adapt their work, but they may never expand their own authority.** The kernel enforces local truth; the supervisor coordinates lifecycle; AWENET coordinates remote cooperation under explicit leases and evidence.
