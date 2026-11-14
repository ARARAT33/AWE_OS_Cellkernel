# CellKernel Local Cell Runtime — ABI 1.1

**Status:** bounded local CellManager runtime is merged on `main`; latest full validation is recorded in `docs/AI-DEVELOPMENT-LOG.md`.  
**Scope:** bounded, local, process-backed cell lifecycle and IPC. This document does not claim per-cell memory isolation or distributed execution.

## Runtime contract

The kernel module is `aweos_kernel::cell`. Its ABI constants are:

- `CELL_ABI_MAJOR = 1`
- `CELL_ABI_MINOR = 1`

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
- Dependencies must already be registered. They must be admitted before a dependent cell is admitted and must be both `Running` and `Healthy` before a dependent starts or is confirmed Running. This registration order prevents cycles in this initial contract.
- `begin_start` calls `ProcessManager::make_runnable` first. The cell remains `Admitted` if scheduler admission fails.
- `confirm_running` publishes `Running` only after the bound process descriptor is `ProcessState::Running`.
- `begin_stop` can cancel a `Starting` cell or stop a `Running` cell, but rejects a graceful provider shutdown with `DependencyInUse` while any dependent is `Starting` or `Running`. It exits the process and closes cell channels. `confirm_stopped` delegates reclamation to `ProcessManager::reap`; it therefore fails while the process is still current or queued.
- `record_failure` exits the bound process, tears down channels, increments a non-wrapping failure counter and quarantines after the configured restart limit. An unexpected provider failure marks all active dependents `Degraded` transitively through the dependency DAG; their supervisors must stop or recover them.
- `begin_stop` rejects a graceful provider shutdown with `DependencyInUse` while any dependent is `Starting` or `Running`. Stop dependents first, then their providers.
- `confirm_running` revalidates dependency state and health after the process has actually entered `Running`, closing the race where a provider fails between `begin_start` and confirmation. A rejected confirmation leaves the cell in `Starting`; the supervisor can cancel it with `begin_stop` or record failure.
- `set_health` verifies that the bound process has not exited. A dependent can return to `Healthy` only after every declared dependency is both `Running` and `Healthy`, in dependency order.
- `rebind_process` accepts only a new `Created` process after the old process has been safely reaped. It never revives an old CPU context.

The architecture/scheduler integration must acknowledge only the exact `DispatchTarget` returned by `ProcessManager::next_dispatch`, and only after execution has reached that target's stack. A failed switch must use `cancel_dispatch`; a second dequeue is rejected while one target is pending. An idle acknowledgement must identify the exact current process and is accepted only after that process is Exited.

## Local IPC contract

Only a currently running cell with both `Security` and `Ipc` capabilities may create a channel. Caller identity is not accepted as a user-supplied `ProcessId`: `create_channel`, `send`, and `receive` derive it from `ProcessManager::current_process_id()`, which returns only the tracked process whose descriptor remains `Running`. The architecture/context-switch path must call `confirm_dispatch(target)` only after the selected context is actually executing; the target must exactly match the single-use token returned by `next_dispatch`. No userspace PID can select its own authority. This manager currently tracks one current process (single-core); SMP must use per-CPU current-task identity and appropriate synchronization before multi-core execution.

A channel declares one receiver, a bounded explicit list of sender `CellId` values and a receiver interface ID. Every sender must have the same interface major version and a minor version at least as new as the receiver's declared minor version.

- `send` accepts a typed `CellSendRequest` and derives caller identity from the manager's tracked current process; no caller PID is accepted in the request. It also checks the sender allowlist, sender/receiver cell states, IPC capabilities, process state, non-zero request/opcode, bounded queue capacity and sender IPC quota before committing the message.
- `receive` checks the receiver cell against the manager's tracked current process and requires the receiver process to be `Running`; no caller PID is accepted in the call.
- Messages contain only fixed-width metadata and four 64-bit payload words; they do not carry unchecked pointers.
- Stopping/failing a receiver destroys its channels. Stopping/failing a sender removes that sender and compacts queued messages so stale messages cannot be delivered.

The endpoint table and per-channel queue are fixed-capacity. Full queues and quota exhaustion return explicit errors; no message is silently overwritten.

## Resource accounting

`charge_cpu` and `charge_memory` provide checked accounting hooks and reject overflow/quota excess without modifying usage. `release_memory` rejects underflow and keeps outstanding memory charged across stop/failure until the allocator confirms the bytes were freed. Restart and cell removal are rejected while memory usage remains non-zero. These hooks are not yet automatically connected to timer ticks or the physical/virtual memory allocator. IPC message usage is charged by successful `send` operations only.

## Security and isolation limits

This milestone uses the existing kernel `CapabilitySet` bitset for coarse admission and IPC rights. The security broker and explicit sender allowlist are enforced by the local CellManager API, and IPC caller identity is derived from `ProcessManager`'s tracked current process rather than a caller-supplied PID. This still depends on the architecture path maintaining the current-process field correctly; the early syscall dispatcher is not yet wired to CellManager and must pass through a trusted kernel context before any user-facing cell syscall is exposed. This is not yet a complete object-capability distribution system.

The module does **not** create per-process page tables, user-only mappings, guard pages, DMA/IOMMU restrictions or a Ring-3 execution environment. It therefore does not prove that one cell's memory is inaccessible to another. The boot path must continue to defer Ring 3 until those mechanisms are implemented and runtime-tested.

Remote cells, network discovery, authenticated peer IPC, replay protection, distributed quotas and partition recovery are out of scope until a versioned authenticated transport exists.

## Required validation

- Identity exhaustion and non-reuse.
- Process existence/state, capability and quota admission failures.
- Dependency admission/start ordering, graceful shutdown ordering, transitive failure degradation and ordered recovery.
- Health updates reject an exited bound process.
- Process-backed start/stop and deferred process reaping.
- Restart with a fresh process and quarantine threshold.
- current-process identity enforcement, non-current-process rejection, idle-context rejection, sender allowlist, interface compatibility, queue backpressure, quota accounting and endpoint teardown.
- Workspace formatting, tests, Clippy, boot/QEMU and VirtIO workflows.

A green build is not proof of memory isolation, remote security, durable storage or physical-hardware support.

## Runtime integration boundary (updated 2026-11-11)

The 2026-11-11 process-runtime refactor makes `ExecutionCore` a compatibility facade over the existing `ProcessManager`. It no longer owns a second process registry, run queue, scheduler or current-task field. Process descriptors, CPU contexts, queue membership, pending-dispatch tokens and acknowledged current-process identity are owned by `ProcessManager`.

Process creation now requires an explicit validated `CpuContext`. A dispatch target remains pending until the exact `DispatchTarget` is acknowledged. `ExecutionCore` and `AcRuntime` expose the outgoing-frame persistence operation so the dispatch loop can save the switched-away registers into the same authoritative context table before that process is selected again. Yield, block, unblock and CPU-budget accounting are delegated to `ProcessManager`.

This is a software-state unification, not a claim that the normal boot path now runs a production scheduler. The `AcRuntime` type is explicitly a bounded contract harness: its `UserRange` entries are metadata only, its local FIFO IPC queue is not authenticated/capability-enforced IPC, and its product-core test manually acknowledges dispatch tokens. That test is not hardware context-switch evidence.

The architecture-owned x86_64 path remains the trusted switch-completion mechanism. The feature-gated QEMU two-stack round-robin self-test exercises that path directly through `ProcessManager`, including outgoing-frame persistence and resume. No normal boot-path call site currently starts a preemptive scheduler, and no general Ring-3 process isolation is claimed.

The current `SyscallContext` remains fail-closed and is not wired to CellManager. No user-facing cell syscall should be exposed until the syscall entry receives trusted current-task identity, a validated per-process address space, safe user-copy operations and capability-checked operations.

Before enabling SMP, replace the single-current-process identity with per-CPU current-task state and synchronization. Before exposing cell operations to Ring 3, add per-process address spaces, user-pointer validation and a versioned syscall contract.

## General syscall dispatcher status (2026-11-07)

The numeric syscall IDs above are currently reserved ABI identifiers, not a
claim that the corresponding operations are available to userspace.
`SyscallContext::dispatch` validates whether a number is known and returns
`ERR_NOT_IMPLEMENTED` for every known general syscall. It does not change
process state, charge quotas, return fabricated IPC/mapping results, or access
caller-provided memory.

This fail-closed behavior is intentional. The existing dispatcher does not
yet receive an authoritative current-task token, a process-manager operation
interface, capability-checked IPC endpoints, or a validated per-process
address space. In particular, no Read/Write pointer is dereferenced and no
Map/Unmap request is treated as successful. Unknown syscall numbers return
`ERR_INVALID_ARGUMENT`.

The x86_64 `BOOTSTRAP_HELLO_SYSCALL` remains a separate early boot smoke-test
handshake; it is not the general syscall ABI and must not be treated as a
production userspace service interface. General Ring-3 applications remain
disabled until context-switch completion, current-task authority, user-memory
validation and syscall-level capability checks are implemented and tested.


## x86_64 target-stack dispatch acknowledgement

The x86_64 SwitchFrame keeps its original eight register/stack/RIP words at offsets 0–56 and adds a kernel-only completion triplet at offsets 64, 72 and 80. When the callback is present, the assembly backend loads the target stack first, calls the non-unwinding callback with its opaque context and argument, then jumps to the target RIP. The frame validator requires a canonical callback/context and a System V AMD64 function-entry stack alignment (RSP % 16 == 0 or 8; the backend pushes one alignment pad in the 16n case).

process::switch_to_dispatch accepts only the exact pending DispatchTarget and uses that callback to commit ProcessManager::confirm_dispatch on the target stack. A callback failure halts the kernel because publishing an unacknowledged task as current would violate a security-sensitive invariant. The API is unsafe and currently single-core: the manager and both stacks must remain pinned and mapped in the shared kernel address space, and the caller must keep interrupts disabled. It does not enable Ring 3, per-process address spaces, SMP, or general preemption.

The feature-gated QEMU exception self-test now includes a two-stack round trip through this path and checks the process lifecycle through exit, idle acknowledgement, and reap.

## Outgoing-context persistence and round-robin resume

The dispatch loop must persist the switched-away process's outgoing frame through `ProcessManager::persist_outgoing_frame` once the scheduler regains control. The x86_64 backend writes the outgoing registers into the frame storage that was current at switch time; without this persistence step a re-dispatched process would resume from its original entry context instead of its actual execution state.

`persist_outgoing_frame` validates the saved RIP/RSP as canonical, clears stale completion metadata (completion hooks belong to the dispatch path, not to a persisted context), applies the saved registers to the stored `ProcessContext`, and rejects processes that cannot be dispatched again (Created/Exited) as well as unknown processes and malformed frames. `ProcessManager::process_context` returns a copy of the stored context for verification.

The feature-gated QEMU self-test now runs a two-process round-robin on separate fresh stacks: each process yields once, the driver persists its outgoing frame (RIP must have advanced past the process entry and RSP must stay inside that process's own stack), and each process must then resume from the persisted context before exiting, acknowledging the switch to idle and being reaped. This is runtime evidence for the persist/resume path; it is still single-core, kernel-address-space-only and does not enable Ring 3, preemption or per-process address spaces.


## Per-process address-space roots and checked user copy (2026-11-14)

`kernel/src/memory/address_space.rs` adds the software contract layer for
per-process address spaces: an `AddressSpace` owns a validated, page-aligned
PML4 root frame and a bounded registry of user mappings. Every user mapping is
validated (page alignment, USER+PRESENT flags via `valid_user_mapping`, user-half
containment, overlap rejection, bounded table). `checked_user_range` and
`check_user_access` are the gates a future kernel-to-user copy must pass:
nonzero, canonical user half, no wraparound, and every touched page mapped
USER+PRESENT plus WRITABLE for writes. This layer does not install page tables,
touch CR3, or perform the physical copy; hardware activation and the Ring-3
entry path remain explicitly unimplemented until isolation tests exist.
