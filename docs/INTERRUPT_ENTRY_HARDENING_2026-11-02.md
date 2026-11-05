# x86_64 Interrupt Entry Hardening — 2026-11-02

> **2026-11-05 alignment correction:** Long mode aligns the new interrupt
> frame to a 16-byte boundary before pushing it, but the old/interrupted RSP
> is saved unconditionally and may be byte-unaligned. The frame validator
> checks that saved RSP is nonzero and canonical; it must not require
> eight-byte alignment. This follows Intel SDM Vol. 3A §6.14.2 and AMD64
> APM Vol. 2 §8.9.3.

> **2026-11-05 architecture correction:** The original same-ring stack-frame
> statement below was incorrect for 64-bit mode. Intel SDM Vol. 3A
> §6.14.2 states that long mode unconditionally pushes the interrupted
> SS:RSP for every interrupt/exception, including same-CPL events with
> IST=0. The current `InterruptFrame` implementation therefore reads the
> hardware-saved `rsp` and `ss` fields for all vectors. The earlier
> same-ring derivation described in this historical plan is superseded.

## Main-branch inspection

- Repository: `ARARAT33/AWE_OS_Cellkernel`
- Target branch: `main` (the repository default branch)
- Inspected head before this work: `6b0b3d2ac58b6720338d571325529c8abe4218bb`
- The latest change adds 256 ISR symbols and expands exception diagnostics. This plan preserves that work and hardens the interrupt-entry contract rather than replacing it.

## Prioritized execution plan

1. **Interrupt ABI correctness (critical):** align the stack to the System V AMD64 call boundary before calling Rust from the common ISR; restore the original saved-register frame exactly; clear DF for Rust code while allowing `iretq` to restore the interrupted RFLAGS.
2. **Exception frame correctness (critical):** use the unconditional SS:RSP saved by 64-bit mode for every vector, including same-CPL interrupts with IST=0. Validate the actual interrupted stack pointer from the hardware frame.
3. **Error-code stub correctness (critical):** keep the assembly stubs synchronized with architectural error-code vectors (#DF, #TS, #NP, #SS, #GP, #PF, #AC, #CP, #VC, #SX); keep #VE (vector 20) as no-error-code.
4. **Safe exception diagnostics:** dispatch #DE (vector 0) as divide error, and do not dereference an arbitrary faulting RIP while handling #UD.
5. **Regression coverage:** test error-code classification, ring-0/ring-3 stack-frame interpretation, and that all 256 IDT entries are populated.
6. **Validation:** run the repository's formatting, workspace tests/build, Clippy and available boot/QEMU workflows; record exact results and leave any unsupported hardware claims unmade.

## Known limitations deliberately not hidden by this change

- APIC vector handling currently lacks an initialized local-APIC EOI backend; this remains a separate hardware-integration task. The present boot path initializes the legacy PIC/PIT, not a complete APIC/IOAPIC runtime.
- The interrupt path does not yet save/restore extended SIMD/FPU state for preempted tasks. That must be designed with per-task XSAVE/FPU ownership before enabling SIMD-heavy user workloads.
- This remote GitHub workflow can update committed repository content on `main`, but it cannot inspect uncommitted files in a developer's local working tree.

## Completion rule

A change is considered validated only when the corresponding GitHub Actions run for the resulting `main` commit has completed. A successful compile or unit test does not by itself certify hardware interrupt behavior.

## Implemented on `main`

- `kernel/src/arch/x86_64/isr.rs`: interrupted RSP/SS accessors read the hardware-saved long-mode fields for all vectors. The 2026-11-05 correction supersedes the earlier same-ring derivation. Error-code classification covers #DF, #TS, #NP, #SS, #GP, #PF, #AC, #CP, #VC and #SX; #VE remains no-error-code.
- `kernel/src/arch/x86_64/isr_stubs.rs`: the common ISR aligns RSP to 16 bytes before the Rust ABI call, clears DF for Rust, and restores the original saved-register frame before `iretq`. #DE now has its own diagnostic path; #UD no longer dereferences an arbitrary faulting RIP. #CP/#VC stubs now preserve the CPU-pushed error-code layout.
- The APIC path now fails visibly for vectors 48-254 when no EOI backend is initialized; vector 255 is treated as the APIC spurious vector and returns without EOI. This is fail-closed behavior, **not** APIC runtime support.
- Added regression tests for ring-0/ring-3 stack interpretation, exception error-code classification, assembly stub/error-code agreement, all 256 IDT entries, and common ISR ABI alignment/restoration.
- Corrected dated change/progress and hardware-matrix documentation so it no longer claims APIC/IOAPIC runtime support or physical interrupt certification.

## Commits from this hardening pass

- `dab418b` — plan
- `36bd25a` — ring-aware interrupt frame
- `e11a6bc` — ISR ABI and exception stubs
- `731ae4c` — interrupt diagnostics and APIC fail-closed behavior
- `9331cbe` — interrupt frame layout documentation
- `206a550` — ISR ABI regression test
- `d92a575`, `b6e270b`, `1208f15` — APIC readiness documentation corrections

## Validation status for this pass

- Source and workflow definitions were inspected through GitHub.
- This environment has no `cargo`, `rustc`, or QEMU executable available locally.
- The repository's push workflows are configured for `main`, but the GitHub connector did not return check/status records for the new commit at the time of this report. Therefore Rust tests, Clippy, formatting and QEMU results for these exact changes remain **pending**, not passed.

## Remaining high-priority risks

1. **IST stacks:** #DF/NMI/machine-check still need dedicated TSS IST stacks and IDT IST selectors. The current RSP/SS accessor explicitly assumes no IST switch.
2. **APIC/IOAPIC:** initialize and map the local APIC, implement EOI, configure IOAPIC routing, and test timer/IPI delivery before enabling APIC vectors.
3. **FPU/SIMD state:** define per-task XSAVE/FPU ownership and interrupt save/restore before enabling SIMD-heavy user workloads.
4. **Runtime validation:** add QEMU tests that deliberately exercise #DE/#UD/#PF and a real timer interrupt counter, then validate on physical hardware separately.
