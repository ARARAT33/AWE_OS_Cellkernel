# x86_64 Interrupt Entry Hardening — 2026-11-02

## Main-branch inspection

- Repository: `ARARAT33/AWE_OS_Cellkernel`
- Target branch: `main` (the repository default branch)
- Inspected head before this work: `6b0b3d2ac58b6720338d571325529c8abe4218bb`
- The latest change adds 256 ISR symbols and expands exception diagnostics. This plan preserves that work and hardens the interrupt-entry contract rather than replacing it.

## Prioritized execution plan

1. **Interrupt ABI correctness (critical):** align the stack to the System V AMD64 call boundary before calling Rust from the common ISR; restore the original saved-register frame exactly; clear DF for Rust code while allowing `iretq` to restore the interrupted RFLAGS.
2. **Exception frame correctness (critical):** stop treating RSP/SS as hardware-pushed fields for same-privilege interrupts. Add ring-aware accessors and validate the actual interrupted stack pointer.
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
