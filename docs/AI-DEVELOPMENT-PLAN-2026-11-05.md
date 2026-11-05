# AWE_OS CellKernel — prioritized engineering plan (2026-11-05)

This plan is based on the current repository tree, the latest `main`
commit (`6b77c7b`), the current project/readiness documents, the previous
AI development log, and a review of the open IST boot-wiring change.

## P0 — Make critical interrupt entry correct before enabling IST

1. Wire the configured NMI, double-fault and machine-check IST stacks into
   the real x86_64 boot path, after all IDT handlers are installed and before
   `lidt`; halt with a diagnostic if configuration fails.
2. Correct `InterruptFrame` interpretation using Intel SDM Vol. 3A
   §6.14.2: in 64-bit mode the CPU unconditionally saves SS:RSP for every
   interrupt/exception, including same-CPL events with IST=0. Long mode
   aligns the new interrupt frame, not the saved interrupted RSP, so do not
   reject a valid byte-unaligned old RSP. Use the hardware-saved fields for
   every vector and test same-ring, IST, Ring 3 and unaligned saved-RSP cases.
3. Make IST configuration preflight all required IDT gates and stack tops
   before mutating TSS/IDT state.

## P0 — Capability handles must never be reused

1. Replace wrapping capability-ID allocation with a checked, exhausted state.
2. Preserve the final `u64::MAX` ID exactly once; subsequent inserts must
   fail with an explicit error rather than returning an old handle.
3. Add a regression test that verifies existing capabilities remain
   addressable and no stale ID is reissued after exhaustion.

## P0 — Do not expose the kernel through the bootstrap user map

1. Audit both bootstrap page-table builders (the 32-bit entry assembly and
   the Rust x86_64 identity-map helper).
2. Clear U/S on every bootstrap paging level so the early identity map is
   supervisor-only.
3. Do not enter Ring 3 until dedicated user code/stack mappings and a
   per-process address-space contract exist. Keep the syscall path dormant
   rather than pretending a shared identity map is process isolation.
4. Add tests that pin the supervisor-only bootstrap flags and document the
   deliberate Ring 3 deferral.

## P1 — Keep boot output evidence-based

Remove fabricated subsystem/readiness claims from the early boot banner.
Report only initialization steps that actually ran. Explicitly state that
timer/PIT programming is not a scheduler.

## P1 — Repository and validation discipline

- Preserve the existing workspace and user work.
- Work on the existing safe branch for PR #57; do not create a new branch,
  force-push, delete branches, or merge unverified changes.
- Update `docs/AI-DEVELOPMENT-LOG.md` with files, real test outcomes,
  unresolved risks, and next priorities.
- Run Rust formatting, workspace check/tests, Clippy, boot-image and QEMU
  jobs when the execution environment permits. Never infer runtime success
  from a successful compile or from a workflow that has not actually run.

## Follow-up priorities (not claimed complete in this pass)

1. QEMU exception injection for #DE/#UD/#PF and dedicated IST delivery for
   #NMI/#DF/#MC.
2. A real per-process address-space builder with supervisor/user page
   permission tests before enabling Ring 3.
3. APIC/IOAPIC EOI and routing runtime; current boot still uses PIC/PIT.
4. Timer-driven context switching and FPU/XSAVE ownership.
5. End-to-end cell lifecycle, IPC/capability isolation, storage/network
   persistence and userspace service supervision.

No release-readiness percentage is used. A subsystem advances only with
executable evidence.


## P0 — Validate the physical memory-map trust boundary

1. Bound the version-1 BootInfo memory-region count and document the memory-map contract.
2. Reject null/misaligned/overflowing region arrays, zero-length or overflowing descriptors, and unsorted/overlapping ranges before the kernel reads them into an allocator.
3. Replace saturating address arithmetic in the boot frame cursor with checked arithmetic; a malformed region must never expose a frame outside its declared interval.
4. Harden the bitmap allocators against unaligned bases/frames, range overflow, capacity mismatch and double-free.
5. Make kernel entry distinguish an invalid memory map from a valid map with no usable frame; add regression tests for every rejected case.
6. Keep the boot cursor explicitly documented as an early-boot probe, not a fully integrated physical allocator; do not claim persistent frame allocation until it is retained and wired into VM/page-table allocation.

Validation must include workspace tests, formatting, Clippy, UEFI loader checks and the existing QEMU boot/VirtIO jobs. A passing unit test is not evidence of durable physical-memory ownership or hardware validation.


## P1 — Turn service metadata into a real Cell runtime

The repository search on `main` found no kernel `CellId`, `CellState` or `CellRuntime` implementation. The existing `ServiceDescriptor`/`ServiceRegistry` are bounded service metadata, and `AuthorizedMailbox` is a local IPC primitive; neither alone constitutes a cell runtime. Do not relabel these structures as completed cells.

1. Define a versioned cell contract: non-reused `CellId`, `CellKind`, lifecycle state, capability set, CPU/memory/IPC quotas, interface ABI versions, declared dependencies, communication endpoints, health state and recovery policy.
2. Implement a bounded `CellManager<const N>` with duplicate/identity checks, explicit transition validation, dependency admission, capability validation, resource admission and quarantine on repeated failure.
3. Bind each admitted cell to an actual kernel `ProcessId` and the existing process/scheduler lifecycle. A cell must not be reported Running until its process has been admitted and the runtime confirms the transition.
4. Bind local cell channels to the existing capability-aware IPC boundary. Reject unauthorized senders and tear down queued messages when a cell exits.
5. Add tests for ID exhaustion/non-reuse, lifecycle transition matrix, duplicate registration, quota exhaustion, dependency failure, unauthorized IPC, teardown, crash/restart and quarantine.
6. Keep remote cells out of scope until a versioned authenticated transport and replay/identity policy exist. Local IPC and remote IPC must share message semantics, not trust assumptions.
7. Do not claim memory isolation until per-process page tables, user-only mappings, guard pages and a real Ring-3 entry/exit path are exercised.

## Next milestone order

1. Complete review of PR #57; keep it unmerged until required checks and the missing exception/IST runtime evidence are addressed.
2. Add QEMU exercises for #DE/#UD/#PF and controlled NMI/#DF/#MC IST delivery, with serial assertions that distinguish handled events from triple-fault resets.
3. Integrate a persistent physical-frame allocator with page-table/VM allocation and reserve kernel, boot-info, framebuffer and firmware ranges explicitly.
4. Implement the bounded CellManager described above and integrate it with process, scheduler, capability and IPC lifecycles.
5. Implement APIC/IOAPIC EOI/routing, timer-driven scheduling and per-task FPU/XSAVE ownership.
