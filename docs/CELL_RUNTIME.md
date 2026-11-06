# CellKernel Local Cell Runtime — ABI 1.0

**Status:** implemented on `feat/cell-runtime-core-2026-11-06`; CI validation pending.  
**Scope:** bounded, local, process-backed cell lifecycle and IPC. This document does not claim per-cell memory isolation or distributed execution.

## Runtime contract

The kernel module is `aweos_kernel::cell`. Its ABI constants are:

- `CELL_ABI_MAJOR = 1`
- `CELL_ABI_MINOR = 0`

A `CellId` and `CellChannelId` are non-zero monotonically allocated identifiers. Once the 64-bit identifier space is exhausted, allocation fails closed; identifiers are not reused when a registry slot is freed.

The runtime is fixed-capacity and allocation-free:

```rust
CellManager<const CELLS: usize,
            const CHANNELS: usize = 16,
            const QUEUE: usize = 8,
            const SENDERS: usize = 8>
```

Each `CellSpec` declares a `CellKind`, an already registered `ProcessId`, granted and required kernel capabilities, CPU/memory/IPC quotas, restart policy, bounded dependencies and versioned interfaces.

## Lifecycle and process binding

The supported lifecycle is:

```text
Declared -> Admitted -> Starting -> Running -> Stopping -> Stopped
                                  \-> Failed -> Admitted (fresh process)
                                             \-> Quarantined
```

- `register` requires an existing `ProcessManager` process in `Created` state, a sufficient process budget, and granted capabilities containing every required capability.
- Dependencies must already be registered. They must be admitted before a dependent cell is admitted and must be `Running` before the dependent is started. This registration order prevents cycles in this initial contract.
- `begin_start` calls `ProcessManager::make_runnable` first. The cell remains `Admitted` if scheduler admission fails.
- `confirm_running` publishes `Running` only after the bound process descriptor is `ProcessState::Running`.
- `begin_stop` exits the process and closes cell channels. `confirm_stopped` delegates reclamation to `ProcessManager::reap`; it therefore fails while the process is still current or queued.
- `record_failure` exits the bound process, tears down channels, increments a non-wrapping failure counter and quarantines after the configured restart limit.
- `rebind_process` accepts only a new `Created` process after the old process has been safely reaped. It never revives an old CPU context.

The architecture/scheduler integration must call the confirmation methods only after the corresponding execution transition has actually completed.

## Local IPC contract

Only a currently running cell with both `Security` and `Ipc` capabilities may create a channel. The caller's `ProcessId` is checked against the broker cell's bound process and must be the process currently marked `Running`.

A channel declares one receiver, a bounded explicit list of sender `CellId` values and a receiver interface ID. Every sender must have the same interface major version and a minor version at least as new as the receiver's declared minor version.

- `send` accepts a typed `CellSendRequest` and checks the caller process identity supplied by the trusted syscall/kernel dispatch path, sender allowlist, sender/receiver cell states, IPC capabilities, process state, non-zero request/opcode, bounded queue capacity and sender IPC quota before committing the message.
- `receive` checks the receiver cell and caller process identity supplied by the trusted kernel path and requires the receiver process to be `Running`.
- Messages contain only fixed-width metadata and four 64-bit payload words; they do not carry unchecked pointers.
- Stopping/failing a receiver destroys its channels. Stopping/failing a sender removes that sender and compacts queued messages so stale messages cannot be delivered.

The endpoint table and per-channel queue are fixed-capacity. Full queues and quota exhaustion return explicit errors; no message is silently overwritten.

## Resource accounting

`charge_cpu` and `charge_memory` provide checked accounting hooks and reject overflow/quota excess without modifying usage. `release_memory` rejects underflow and keeps outstanding memory charged across stop/failure until the allocator confirms the bytes were freed. Restart and cell removal are rejected while memory usage remains non-zero. These hooks are not yet automatically connected to timer ticks or the physical/virtual memory allocator. IPC message usage is charged by successful `send` operations only.

## Security and isolation limits

This milestone uses the existing kernel `CapabilitySet` bitset for coarse admission and IPC rights. The security broker and explicit sender allowlist are enforced by the local CellManager API, but this is not yet a complete object-capability distribution system or a substitute for syscall caller-context enforcement.

The module does **not** create per-process page tables, user-only mappings, guard pages, DMA/IOMMU restrictions or a Ring-3 execution environment. It therefore does not prove that one cell's memory is inaccessible to another. The boot path must continue to defer Ring 3 until those mechanisms are implemented and runtime-tested.

Remote cells, network discovery, authenticated peer IPC, replay protection, distributed quotas and partition recovery are out of scope until a versioned authenticated transport exists.

## Required validation

- Identity exhaustion and non-reuse.
- Process existence/state, capability and quota admission failures.
- Dependency admission/start ordering.
- Process-backed start/stop and deferred process reaping.
- Restart with a fresh process and quarantine threshold.
- IPC caller spoof rejection, sender allowlist, interface compatibility, queue backpressure, quota accounting and endpoint teardown.
- Workspace formatting, tests, Clippy, boot/QEMU and VirtIO workflows.

A green build is not proof of memory isolation, remote security, durable storage or physical-hardware support.
