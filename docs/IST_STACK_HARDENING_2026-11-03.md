# x86_64 IST Stack Hardening — 2026-11-03

## Context

This change resolves remaining risk #1 from the 2026-11-02 interrupt entry
hardening pass (see `INTERRUPT_ENTRY_HARDENING_2026-11-02.md`): the critical
vectors #NMI (2), #DF (8) and #MC (18) had no Interrupt Stack Table backing.
A fault or NMI arriving on a corrupted, exhausted or undefined kernel stack
escalated to a triple fault (silent CPU reset) instead of reaching a handler
able to print diagnostics.

## Implemented on branch `feat/x86_64-ist-critical-stacks`

- `kernel/src/arch/x86_64/gdt.rs`:
  - `Tss64::set_ist(index, stack_top)` — validated assignment of TSS IST
    slots. Rejects slot indices outside 1..=7, zero and non-16-byte-aligned
    stack tops (the CPU loads these values directly into RSP).
  - `Tss64::ist(index)` — read-back accessor; unassigned or invalid slots
    read 0.
- `kernel/src/arch/x86_64/idt.rs`:
  - `IdtEntry::ist_index()` — decodes the architectural IST field (bits
    0..2 of the options word; the previous code always wrote `0x8e00`, i.e.
    no IST).
  - `Idt::ist_index(vector)` / `Idt::set_ist(vector, ist)` — query and mark
    an already-installed vector. Fails closed when the vector has no present
    handler or the slot is out of range; never clobbers the present/type bits.
- `kernel/src/arch/x86_64/ist.rs` (new):
  - Critical-vector contract: NMI→IST1, #DF→IST2, #MC→IST3, each backed by a
    dedicated 16 KiB static 16-byte-aligned stack.
  - `configure_ist_stacks(&mut Tss64, &mut Idt)` — wires TSS and IDT
    consistently and fails closed if a critical handler is missing.
  - `ist_index_for_vector()` — single source of truth for the mapping.
- `kernel/src/arch/x86_64/isr.rs`: documentation of
  `interrupted_stack_pointer` updated — IST-backed vectors switch RSP before
  the frame is built, but the same-ring RSP/SS derivation stays valid.

## Test coverage added (host-runnable unit tests)

- TSS slot validation: valid tops accepted, invalid slot index / zero /
  misaligned tops rejected, unassigned slots read zero.
- IDT IST encoding round-trips; present/type bits preserved; plain gates
  report no IST; uninstalled vectors report none; out-of-range slots rejected.
- IST module: vector→slot mapping is total for the three critical vectors,
  distinct across them, and None elsewhere; stack tops are non-zero,
  16-byte aligned and pairwise distinct; end-to-end `configure_ist_stacks`
  leaves TSS and IDT consistent across all 256 vectors; empty-IDT
  configuration fails closed.

## Deliberately not claimed

- This is wiring plus contract validation. Correct behaviour under real
  NMI/#DF/#MC delivery, including IST switch correctness on hardware, is a
  QEMU/runtime validation task (next rung: QEMU tests that deliberately
  trigger #DE/#UD/#PF and a fault on a corrupted stack).
- `configure_ist_stacks` is not yet called from the boot path; wiring it
  into boot order (after IDT stub installation, before `lidt` or before the
  first `sti`) is the follow-up once CI validates this layer.
- APIC/IOAPIC runtime and FPU/XSAVE remain open (risks #2 and #3 of the
  2026-11-02 pass).
