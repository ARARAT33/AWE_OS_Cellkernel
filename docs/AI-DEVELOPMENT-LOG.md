# AI Development Log

Running log of autonomous development sessions on AWE_OS_Cellkernel.
Each entry records what was done, what changed, real test results, open
issues and the next session's priorities. Percentages and unverified
readiness claims are deliberately not used.

---

## Session 2026-11-03 — IST stacks for critical x86_64 vectors + CI repair

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
- **Critical CI finding:** `main` itself has been red since the 2026-11-02
  pass — AWEOS Quality Gate, AWEOS CI Pipeline, AWEOS Cloud Security Gate
  and AWEOS VirtIO Runtime all fail at their `cargo fmt --check` step on
  `20b66d0` (pre-existing rustfmt violations in
  `kernel/src/arch/x86_64/isr_stubs.rs`: whitespace-only lines and
  single-line `if` statements introduced before this session).
- Noted open PRs #55, #53, #52, #50, #48 (stale, pre-restructure bases) —
  left untouched, nothing deleted, no force push.

### Chosen task (highest priority, realistic)

Resolve risk #1: dedicated IST stacks for #NMI (vector 2), #DF (vector 8)
and #MC (vector 18), and repair the broken CI so main becomes green again.

### Work done (branch `feat/x86_64-ist-critical-stacks`, PR #56)

Implementation commits:
- `5bd63a2` — IST feature: `gdt.rs` validated `Tss64::set_ist`/`ist`;
  `idt.rs` IST encoding (`set_ist`, `ist_index`); new `ist.rs` module
  wiring NMI→IST1, #DF→IST2, #MC→IST3 on dedicated 16 KiB aligned static
  stacks with fail-closed configuration; `isr.rs` IST-aware frame docs;
  9 host-runnable unit tests including an end-to-end TSS/IDT consistency
  check across all 256 vectors.
- `72dc300` — docs: `docs/IST_STACK_HARDENING_2026-11-03.md` and this log.

CI repair commits:
- `4fe8abf`, `a2a0ef0` — manual rustfmt-normalization of the new IST code
  and of the pre-existing `isr_stubs.rs` violations on main (removed 21
  whitespace-only lines, split single-line `if`s, broke an over-long
  if-else assignment).
- `f1d69bf`/`3bb302f` — added `.github/workflows/rustfmt-fix.yml`: a bot
  that runs real `cargo fmt --all` on feature branches and commits the
  result back, so rustfmt diffs are visible as commits instead of
  inaccessible job logs, and formatting can never silently block CI.
- `e3b6111` — the rustfmt-fix bot's own commit applying canonical rustfmt
  to `isr.rs`, `isr_stubs.rs` and `ist.rs` (+41/−19 lines, formatting only).

### Real test results (GitHub Actions evidence)

- First CI run (head `5bd63a2`/`72dc300`): UEFI loader, bootable-image
  smoke test, multi-arch images and AOSIN installers green; **workspace
  `cargo check` and `cargo test` passed** (ci.yml reached its formatting
  step), but all four fmt-checking jobs failed at
  `cargo fmt --all -- --check`.
- Root cause analysis: pre-existing fmt violations inherited from
  `main` plus minor style deviations in the new IST code.
- After the manual fixes (`4fe8abf`/`a2a0ef0`) fmt still failed, so the
  exact canonical rustfmt output was produced by the new bot in CI
  (`e3b6111`) — this is real `cargo fmt` output, not a guess.
- Validation of the fully formatted tree: complete, see "Final CI
  status" below.



### Clippy repair (same session, continued)

- `753b197` — added `.github/workflows/clippy-report.yml`: on clippy
  failure the job comments the exact compiler diagnostics (last 250 lines)
  on the PR, giving log-free access to precise lint errors.
- Two pre-existing clippy errors in `kernel/src/arch/x86_64/isr_stubs.rs`
  (introduced by the 2026-11-02 pass) were fixed: `unnecessary_cast`
  (`frame.error_code as u64` → `frame.error_code`) and
  `manual_range_contains` (`vector >= 32 && vector < 48` →
  `(32..48).contains(&vector)`).
- Three clippy `assertions_on_constants` errors in the new `ist.rs`
  tests were fixed by moving the constant IST-slot distinctness checks into
  a compile-time `const {}` block.

### Final CI status (PR #56 head, 2026-11-03)

All 14 checks on the PR head completed **green**:

- Rust workspace: success
- Workspace Build & Validation: success
- Workspace clippy with PR diagnostics: success
- cloud-security: success
- UEFI loader: success
- build-and-smoke-test-image: success
- Build Multi-Arch Boot Images: success
- virtio-block-contract-and-qemu: success (real QEMU boot + VirtIO
  block-contract runtime)
- Build AOSIN Installers (ubuntu/windows, gui/cli): success (4/4)
- Apply rustfmt and commit fixes: success
- cloud-release-contract: success

This is the first fully green CI on any branch since 2026-11-02.
The PR is left open for human review per the no-unverified-merge rule;
merging it also restores `main` to green.
### Unresolved issues

- `configure_ist_stacks` is not yet called from the boot path; the call
  site must be ordered after IDT stub installation and before interrupts
  are enabled.
- IST switch behaviour under real NMI/#DF/#MC delivery needs QEMU/hardware
  evidence, not only unit tests.
- `main` remains red until PR #56 merges; merging before green CI is
  forbidden.
- Open risks carried forward: APIC/IOAPIC runtime (EOI backend, IOAPIC
  routing), FPU/XSAVE per-task ownership, QEMU exception-exercise tests.
- Stale open PRs (#55, #53, #52, #50, #48) predate the 2026-08-27
  restructure and need re-evaluation against current `main`.

### Next session priorities

1. Review and merge PR #56 (all 14 checks green) to restore `main`
   CI; after merge, confirm `main` itself runs green.
2. Wire `configure_ist_stacks` into the kernel boot sequence with a
   boot-order regression test.
3. Start risk #4: a QEMU boot test that deliberately exercises #DE/#UD/#PF
   and counts real timer interrupts.
4. Consider extending the rustfmt-fix bot to `main` pushes.
5. Re-evaluate stale open PRs against current `main`.
