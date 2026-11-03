# AI Development Log

Running log of autonomous development sessions on AWE_OS_Cellkernel.
Each entry records what was done, what changed, real test results, open
issues and the next session's priorities. Percentages and unverified
readiness claims are deliberately not used.

---

## Session 2026-11-03 — IST stacks for critical x86_64 vectors

### Repository audit performed first

- Inspected `main` @ `20b66d0` (2026-11-02): interrupt entry hardening pass
  (ISR ABI alignment, ring-aware frames, fail-closed APIC vectors) plus
  documentation corrections removing APIC runtime claims.
- Confirmed the declared open risks from
  `docs/INTERRUPT_ENTRY_HARDENING_2026-11-02.md`: (1) no IST stacks for
  #NMI/#DF/#MC, (2) no APIC/IOAPIC runtime, (3) no FPU/XSAVE design,
  (4) no QEMU exception-exercise tests.
- Verified in code: `Tss64` exposed `ist1..ist7` but nothing assigned them;
  `IdtEntry::new` always wrote `0x8e00` options (no IST encoding);
  `docs/AI-DEVELOPMENT-LOG.md` did not exist.
- Noted open PRs #55, #53, #52, #50, #48 (stale, pre-restructure bases) —
  left untouched, nothing deleted, no force push.

### Chosen task (highest priority, realistic)

Resolve risk #1: dedicated IST stacks for #NMI (vector 2), #DF (vector 8)
and #MC (vector 18). Without them, a fault on a corrupted kernel stack
escalates to a triple fault with zero diagnostics.

### Work done (branch `feat/x86_64-ist-critical-stacks`, commit `5bd63a2`)

Changed files:
- `kernel/src/arch/x86_64/gdt.rs` — validated `Tss64::set_ist`/`ist`
- `kernel/src/arch/x86_64/idt.rs` — IST encoding, `Idt::set_ist`, `ist_index`
- `kernel/src/arch/x86_64/ist.rs` — new IST wiring module (NMI→IST1,
  #DF→IST2, #MC→IST3, dedicated 16 KiB aligned stacks, fail-closed
  configuration)
- `kernel/src/arch/x86_64/mod.rs` — register `ist` module
- `kernel/src/arch/x86_64/isr.rs` — IST-aware frame documentation
- `docs/IST_STACK_HARDENING_2026-11-03.md` — change record

Added capabilities: architectural IST slot validation; IST options encoding
in IDT entries without clobbering present/type bits; consistent TSS+IDT
critical-vector wiring; 9 new host-runnable unit tests including an
end-to-end consistency check across all 256 IDT vectors.

### Real test results

- This environment cannot execute `cargo`/`rustc`/QEMU. No test was run
  locally and no local result is claimed.
- A pull request was opened so the repository's CI (formatting, workspace
  check, `cargo test --workspace`, Clippy `-D warnings`) validates the
  change. Until that run completes on the PR head, validation status is
  **pending, not passed**.

### Unresolved issues

- `configure_ist_stacks` is not yet called from the boot path; the call site
  must be ordered after IDT stub installation and before interrupts are
  enabled.
- IST switch behaviour under real NMI/#DF/#MC delivery needs QEMU/hardware
  evidence, not only unit tests.
- Open risks carried forward: APIC/IOAPIC runtime (EOI backend, IOAPIC
  routing), FPU/XSAVE per-task ownership, QEMU exception-exercise tests.
- Stale open PRs (#55, #53, #52, #50, #48) predate the 2026-08-27
  restructure and need re-evaluation against current `main`.

### Next session priorities

1. Check CI results for the IST PR; fix any failures; only then consider
   merging (no unverified merges into `main`).
2. Wire `configure_ist_stacks` into the kernel boot sequence with a
   boot-order regression test.
3. Start risk #4: a QEMU boot test that deliberately exercises #DE/#UD/#PF
   and counts real timer interrupts.
4. Re-evaluate stale open PRs against current `main`.
