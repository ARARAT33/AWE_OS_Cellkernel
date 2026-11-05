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


---

## Session 2026-11-04 — Wire critical IST stacks into the boot path

### Repository audit performed first

- Confirmed PR #56 (IST stacks for #NMI/#DF/#MC) was merged to `main`
  (`6b77c7b`, merge commit 2026-11-04), restoring the CI-green state the
  previous session achieved.
- Re-read the carried-forward unresolved issues: `configure_ist_stacks`
  was still not called from any boot path, and early boot serial output in
  `kernel-bin/src/main.rs` printed fabricated readiness claims
  ("100% Ready", "APIC initialized", "Preemptive Scheduler running",
  "System Core Status = 100% COMPLETE") before any of those subsystems ran,
  which violates the no-false-readiness-claims rule.
- Verified the correct wiring point in `kernel_entry`
  (`kernel/src/entry.rs`): after `init_idt_stubs` (so the critical
  vectors have handlers) and before `Idt::load` (so the IST-marked entries
  are what the CPU sees). TSS mutation after `ltr` is safe: the CPU reads
  the TSS from memory at interrupt delivery.

### Chosen task (top priority from the previous session log)

1. Wire `configure_ist_stacks` into `kernel_entry` with a fail-closed
   fatal path, between IDT stub installation and `Idt::load`.
2. Add a host-runnable boot-order regression test pinning the contract
   ("configure fails before stubs are installed, succeeds after").
3. Remove the fabricated readiness claims from the early boot banner and
   the false "Preemptive Scheduler initialized" line; keep the real
   subsystem status lines that report actual initialization.

### Work done (branch `feat/x86_64-ist-boot-wiring`)

- `kernel/src/entry.rs`: after `init_idt_stubs`, the boot path now calls
  `ist::configure_ist_stacks(&mut TSS, &mut IDT)`; on failure it prints a
  FATAL diagnostic and halts instead of booting without IST protection.
  Success is reported as "IDT & critical IST stacks initialized". The
  false "Preemptive Scheduler initialized" line was replaced with an
  accurate "timer programmed (no scheduler yet)" message.
- `kernel/src/arch/x86_64/ist.rs`: new
  `boot_order_contract_configure_after_stub_install` test — fails closed
  on an empty IDT, succeeds after handlers are installed, and verifies
  every critical vector is IST-marked afterwards.
- `kernel-bin/src/main.rs`: the fabricated banner (fake 100%/50%
  percentages, phantom APIC/IPC/VFS/scheduler claims) was replaced with a
  factual early-boot banner; real subsystem status is printed only by the
  code that actually initializes each subsystem.
- `docs/AI-DEVELOPMENT-LOG.md`: this entry.

### Test results

- The new host-runnable test plus the 9 existing IST unit tests and the
  GDT/IDT/entry/interrupts unit suites are exercised by the workspace
  `cargo test` job in CI on this PR head.
- The QEMU boot smoke and VirtIO runtime jobs exercise the real boot
  image built from the modified `kernel_entry`; IST delivery under real
  #NMI/#DF/#MC injection remains a separate QEMU exception-exercise test
  that does not exist yet (carried forward, not claimed).
- Exact CI results for this PR head are recorded in the PR checks; the
  PR is left open for review per the no-unverified-merge rule.

### Unresolved issues

- QEMU exception-exercise test (#DE/#UD/#PF plus timer interrupt counts)
  still missing — next session's primary target.
- APIC/IOAPIC runtime (EOI backend, IOAPIC routing) still absent; the
  system still boots with the legacy 8259 PIC only.
- FPU/XSAVE per-task ownership not designed yet.
- Stale open PRs (#55, #53, #52, #50, #48) still need re-evaluation
  against current `main`.
- IST behaviour under real NMI/#DF/#MC delivery still needs QEMU/hardware
  evidence (unit + boot-order tests cover the wiring contract only).

### Next session priorities

1. Verify this PR's CI is green and merge it after review.
2. Build the QEMU exception-exercise boot test (risk #4) using the
   existing `build-and-smoke-test-image` pipeline.
3. Begin APIC/IOAPIC runtime design (EOI backend first, then IOAPIC
   routing), replacing the legacy PIC path in a follow-up PR.

---

## 2026-11-05 — IST boot wiring, long-mode frame correctness and bootstrap privilege boundary

### Repository audit and plan
- Audited the current `main` snapshot at `6b77c7bb2d5d2b27878811107935ce346b8d8148`, enumerated 447 tracked tree entries, 29 Cargo manifests and 9 workflow files, and reviewed the boot, memory, interrupt, process/scheduler, IPC, capability, driver, storage, network, AWOSA and AYUI boundaries.
- Added `docs/AI-DEVELOPMENT-PLAN-2026-11-05.md` and `docs/CELLKERNEL_ARCHITECTURE_AUDIT_2026-11-05.md`. The audit distinguishes implemented primitives/contracts from end-to-end runtime functionality; it does not assign a completion percentage.
- Work is on the existing safe branch `feat/x86_64-ist-boot-wiring` / PR #57. No branch was created or deleted, no force-push was used, and no unverified changes were merged.

### Critical findings and fixes
1. **IST boot integration**
   - `kernel/src/entry.rs` configures NMI, double-fault and machine-check IST stacks after all IDT stubs are installed and before `lidt`; configuration failure prints a fatal diagnostic and halts.
   - `kernel/src/arch/x86_64/ist.rs` now preflights all required gates and canonical/aligned stack tops before mutating TSS/IDT state. The empty-IDT test also verifies no partial state is left behind.

2. **Correct 64-bit interrupt frame**
   - The previous same-CPL accessor incorrectly derived RSP and treated SS as absent.
   - Intel SDM Vol. 3A §6.14.2 and AMD64 APM Vol. 2 §8.9.3 specify that long mode unconditionally saves SS:RSP for every interrupt/exception, even with no CPL change and no IST.
   - `InterruptFrame` now reads the hardware-saved RSP/SS for all vectors; diagnostics print SS unconditionally. Tests cover same-CPL/no-IST, same-CPL/IST and Ring 3 frames.
   - Earlier historical notes in `docs/INTERRUPT_ENTRY_HARDENING_2026-11-02.md` and `docs/IST_STACK_HARDENING_2026-11-03.md` were corrected with an explicit superseding note.

3. **Bootstrap privilege boundary**
   - The original bootstrap PML4/PDPT/2 MiB leaf flags included U/S=1, and the Rust identity-map helper also granted USER. A Ring 3 transition using that shared 1 GiB identity map would expose supervisor memory.
   - Both bootstrap page-table builders now use shared supervisor-only permission constants. Tests verify U/S is clear at table and leaf levels.
   - The boot path no longer enters the bootstrap Ring 3 stub. Ring 3 is deliberately deferred until dedicated per-process user code/data/stack mappings exist. The unsafe helper remains documented with its mapping preconditions, but is not called by boot.

4. **Host test linkability**
   - Changed the x86_64 context-switch resume-label address to RIP-relative form, fixing the PIE test-linker relocation error observed on the earlier PR head.

### Files changed in this pass
- `kernel/src/entry.rs`
- `kernel/src/arch/x86_64/ist.rs`
- `kernel/src/arch/x86_64/isr.rs`
- `kernel/src/arch/x86_64/isr_stubs.rs`
- `kernel/src/memory/x86_64.rs`
- `kernel-bin/src/main.rs`
- `kernel/src/process/x86_64_switch.S`
- `docs/AI-DEVELOPMENT-PLAN-2026-11-05.md`
- `docs/CELLKERNEL_ARCHITECTURE_AUDIT_2026-11-05.md`
- `docs/INTERRUPT_ENTRY_HARDENING_2026-11-02.md`
- `docs/IST_STACK_HARDENING_2026-11-03.md`
- this development log

### Validation evidence for PR head `9507d193538f5d9e5d3d1d1ef1f3695fb3b3b8d8`
- AWEOS CI Pipeline: workspace compilation, workspace tests, formatting, boot-image build and artifact verification — **passed**.
- AWEOS Quality Gate: formatting, workspace check/tests, product-core evidence, Clippy and UEFI loader check — **passed**.
- Clippy PR diagnostics — **passed**.
- Bootable Image Pipeline: QEMU ISO smoke test — **passed**.
- Cloud Security Gate — **passed**.
- Multi-Architecture image build/artifact verification — **passed**.
- AOSIN cross-platform installer matrix — **passed**.
- VirtIO Runtime (including QEMU VirtIO block boot) — **still in progress at log time; not claimed as passed**.
- Local `cargo`, `rustc` and QEMU were unavailable in this automation container; the Rust and QEMU evidence above is from GitHub Actions, not a local run.
- No physical hardware validation or injected NMI/#DF/#MC runtime test was performed.

Earlier PR-head failures were investigated: stale flag identifiers, Clippy constant-assertion warnings, a formatting mismatch, and an x86_64 PIE relocation. They were corrected; the current CI/Quality Gate/Clippy/boot-image checks above pass on the later head.

### Remaining risks / next priorities
1. Add QEMU exception-injection tests for #DE/#UD/#PF and actual NMI/#DF/#MC IST delivery.
2. Fix capability ID exhaustion: `CapabilityTable::insert` currently wraps its `u64` ID counter back to 1 and can reuse stale handle IDs.
3. Implement per-process page tables and user-only code/data/stack mappings before any Ring 3 re-enable.
4. Implement APIC/IOAPIC EOI/routing and timer-driven context switching; current timer programming is not a preemptive scheduler.
5. Prove persistent block-device I/O/recovery, real network-device I/O and AWOSA service supervision end-to-end.
6. Add guard pages for IST stacks, FPU/XSAVE ownership, and fuzz/stress tests for capability, IPC, storage and network boundaries.

PR #57 remains open and unmerged pending the remaining VirtIO check and review.
