# AWE_OS CellKernel — prioritized engineering plan (2026-11-05)

This plan is based on the current repository tree, the latest `main`
commit (`6b77c7b`), the current project/readiness documents, the previous
AI development log, and a review of the open IST boot-wiring change.

## P0 — Make critical interrupt entry correct before enabling IST

1. Wire the configured NMI, double-fault and machine-check IST stacks into
   the real x86_64 boot path, after all IDT handlers are installed and before
   `lidt`; halt with a diagnostic if configuration fails.
2. Correct `InterruptFrame` interpretation: an IST stack switch saves the
   interrupted RSP/SS even when CPL does not change. Preserve the existing
   same-ring/no-IST derivation and add regression tests for both cases.
3. Make IST configuration preflight all required IDT gates and stack tops
   before mutating TSS/IDT state.

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
