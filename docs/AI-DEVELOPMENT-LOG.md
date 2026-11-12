
---

## Session 2026-11-08 (post-merge CI verification and stale-PR triage)

### Repository audit performed first

- Continued from the merge-coordination session's stopping point: main head `7187bf0` (docs-only, on merge `81575b9` = PR #61 and `6883904` = PR #60).
- Confirmed via the Actions page (branch main) that no workflow runs were triggered for heads `81575b9` or `7187bf0`: the newest main runs were still those for `6883904` (all workflows green). Automation-token pushes to main do not trigger Actions, and no workflow-dispatch tool is available through the automation interface, so main-head CI could not be obtained by pushing again.

### CI verification of the current main tree (PR #62)

- Created branch `chore/ci-verify-main-2026-11-08` from main head `7187bf0` with a single new file `docs/CI_VERIFICATION_2026-11-08.md`, and opened draft PR #62 ("CI verification of current main tree"). PR events do trigger workflows, so this PR provided the missing CI evidence for the current main content: the PR tree differs from main by one documentation file only.
- All 15 check-runs on PR #62 completed **success** (Actions run IDs recorded from check-run URLs):
  - `cloud-release-contract` and `cloud-security` — run 42520373128
  - `Rust workspace`, `UEFI loader` — run 42520373122
  - `Workspace Build & Validation` — run 42520373023
  - `Workspace clippy with PR diagnostics` — run 42520373117
  - `cargo failure diagnostics` — run 42520373002
  - `exception-injection-selftest`, `build-and-smoke-test-image` — run 42520373011
  - `virtio-block-contract-and-qemu` — run 42520373272
  - `Build Multi-Arch Boot Images` — run 42520373262
  - `Build AOSIN Installers` (ubuntu/windows, gui/cli, 4/4) — run 42520373111
- PR #62 was marked ready for review and merged to main as `969d298` (merge method). This gives real, recorded CI evidence that the current main tree (including the merged storaged hardening of PR #60 and the ProcessManager dispatch-acknowledgement work of PR #61) passes the full check suite. Caveat kept honest: the runs execute the PR head commit (main + one docs file), and the main branch itself still has no direct Actions run on `969d298` because token pushes do not trigger workflows.

### Stale PR re-evaluation started

- Posted an automated re-evaluation comment on stale PRs #52, #53 and #55: both hardening PRs #60/#61 have been merged; each stale branch should be rebased onto current main and its check suite re-run, or closed in favour of the merged hardening work. No branches or commits were deleted; nothing was force-pushed.
- PR #61's branch and PR #60's branch remain preserved after merging.

### Files changed in this session

- `docs/CI_VERIFICATION_2026-11-08.md` — new, via PR #62 (explains why the verification PR exists).
- `docs/AI-DEVELOPMENT-LOG.md` — this entry.
- Comments on PRs #52, #53, #55.
- No Rust source files were modified by this session.

### Honest validation state

- Current main tree validated green via PR #62's 15/15 check-runs (run IDs above); main-head Actions runs remain untriggerable by this automation token.
- The ProcessManager dispatch-acknowledgement contract remains unit-test-backed only; no production x86_64 switch-completion trampoline calls `confirm_dispatch` yet. No Ring-3 execution, per-process address spaces, block I/O, NIC TX/RX or physical-hardware behaviour is claimed.
- Stale PRs #52/#53/#55 (and #50/#48 if still open) remain un-rebased; they must not be merged without a fresh green check suite on a rebased head.

### Next session priorities

1. Wire the architecture-owned x86_64 switch-completion trampoline that acknowledges the exact pending dispatch target on the target stack; prove two real contexts in QEMU.
2. Unify `ProcessManager` and `ExecutionCore` behind one authoritative current-task model.
3. Implement per-process page-table roots and checked user-memory copy operations before enabling Ring 3.
4. Follow up on the re-evaluation comments: rebase or close stale PRs #55/#53/#52 (and check #50/#48) against current main `969d298`.

## 2026-11-09 — x86_64 target-stack dispatch acknowledgement

### Baseline and branch discipline

- Baseline audit began at main commit 6025b264aa80efd605d6eb5517c9ccb3f6dfc92b. The plan was updated first in commit f14a7db4f860193d38941a86374c64f039341fd8.
- Implementation commit: 1e27e61e609437560ab4cfd40ed52813abf7afef. Formatting-only correction: b2d212ca2a7558b9c044383abc7ed8d1341ac144.
- All implementation and documentation commits were written directly to main with a parent-SHA compare-and-swap update. No branch or PR was created; the existing chore/ci-verify-main-2026-11-08 branch and open PRs #55, #53, #52, #50 and #48 were left untouched.
- The GitHub API exposes committed repository state only. No local clone/worktree was available to inspect uncommitted local edits or run commands in a local checkout.
- Workspace inventory: 29 Cargo workspace members across boot (4), kernel/kernel-bin (2), services (11), apps (8), tools (3), and product-core tests (1); the recursive Git tree contains 447 entries.

### Implemented

- kernel/src/process/context_switch.rs: extended the kernel-private x86_64 SwitchFrame from 64 to 88 bytes; added a 24-byte completion triplet at fixed offsets 64/72/80 with compile-time layout assertions. Completion metadata is validated for canonical code/context addresses and System V AMD64 target-stack alignment. Added tests for aligned and misaligned callback frames and orphan metadata.
- kernel/src/process/x86_64_switch.S: loads all callback metadata before changing stacks; after installing the target RSP it calls the optional completion hook on that target stack, preserves the target RIP, then jumps to the target RIP.
- kernel/src/process/mod.rs: added switch_to_dispatch, which rejects null/noncanonical/misaligned pointers and accepts only the exact pending DispatchTarget. It acknowledges the pending process through ProcessManager on the target stack, cancels a pending dispatch if frame/backend validation fails, and halts fail-closed if target-stack acknowledgement cannot be committed.
- kernel/src/arch/x86_64/selftest.rs: added a feature-gated fresh 4 KiB process stack test. It switches boot stack → process stack, verifies the callback published Running before process entry, exits, switches back, acknowledges idle, and reaps the process.
- docs/CELL_RUNTIME.md: documented the callback ABI, validation, single-core/shared-kernel-address-space assumptions, and explicit non-goals.
- The first implementation commit passed compilation/tests but failed rustfmt checks. The formatting was corrected in b2d212c; the final validation matrix below is for that corrected commit.

### Final validation on main commit b2d212ca2a7558b9c044383abc7ed8d1341ac144

- AWEOS CI Pipeline run 42655918770 — SUCCESS: cargo check --workspace --all-targets; cargo test --workspace (337 passed, 0 failed); cargo fmt --all --check; product-core tests (11 passed); cargo clippy --workspace --all-targets with -D warnings and the repository's documented lint allowances.
- AWEOS Quality Gate run 42655918945 — SUCCESS: workspace formatting/check/tests (337 passed); product-core integration tests (11 passed); targeted persistent-storage round-trip/recovery test (1 passed); full AWE Nexus services/compatibility integration test (1 passed); workspace Clippy.
- AWEOS Bootable Image Pipeline run 42655918803 — SUCCESS: kernel-selftest Clippy; x86_64-unknown-none selftest-feature check; QEMU exception-injection self-test (1 passed); release kernel build; GRUB Multiboot2 header validation; ISO creation; QEMU serial smoke test reached 'AWEOS: kernel is alive'. The exception self-test's phase sequence cannot reach the NMI-wait marker if the new two-stack test returns failure.
- AWEOS VirtIO Runtime run 42655919123 — SUCCESS: contract tests and QEMU launch. QEMU exposed virtio-blk-pci; serial evidence showed boot protocol validated, kernel state RUNNING, and kernel alive. This run did not perform a verified block-device read/write or persistence test.
- AWEOS Cloud Security Gate run 42655919237 — SUCCESS.
- AWEOS Multi-Architecture Images run 42655918899 — SUCCESS for configured image builds. This is build evidence, not ARM64/RISC-V runtime boot evidence.
- AOSIN Cross-Platform Installers run 42655919007 — SUCCESS for the configured Linux and Windows GUI/CLI installer jobs.

### Remaining risks and honest scope

- The new path is an architecture-owned dispatch acknowledgement primitive and a feature-gated QEMU regression, not a production scheduler. The normal kernel boot path still does not start a preemptive process scheduler.
- ProcessManager's stored ProcessContext is not yet updated from the outgoing SwitchFrame by this wrapper; context persistence across multiple real processes and integration with ExecutionCore remain the next critical work.
- The mechanism is single-core, assumes interrupts remain disabled during the switch, and requires the manager, callback context, and both stacks to stay pinned and mapped in the shared kernel address space. It is not SMP-safe and does not provide address-space isolation.
- Ring 3, per-process page tables, supervisor-only kernel mappings, checked user-copy, capability-checked syscalls, timer preemption, and user-mode isolation remain unimplemented or not validated.
- QEMU validation used the repository's Multiboot2/GRUB path. The UEFI loader build/check passed in the Quality Gate, but no full OVMF/UEFI firmware boot was performed in this run. No physical hardware was tested.
- VirtIO evidence proves QEMU device exposure and kernel boot, not functional block I/O. Network TX/RX, authenticated remote cell IPC, durable native filesystem recovery, and USB/GPU/audio/Wi-Fi hardware drivers remain separate unvalidated gates.

### Next priorities

1. Persist the outgoing SwitchFrame into ProcessManager's ProcessContext on every dispatch and test two runnable processes round-robin on separate QEMU stacks.
2. Unify ProcessManager and ExecutionCore so there is one authoritative current-task/lifecycle model; connect scheduler tick, interrupt return, and the architecture switch path.
3. Add guarded page-table roots and checked kernel/user mappings before any Ring-3 entry; keep user mode disabled until isolation tests exist.
4. Continue VirtIO block I/O with real request/completion and read-after-write tests, then persistent storage/crash-recovery tests.
5. Add explicit QEMU self-test serial milestones for the two-stack path and expand regression/fuzz/stress coverage.

## 2026-11-09 — outgoing-context persistence and two-process round-robin

### Baseline and branch discipline

- Continued from the 2026-11-09 dispatch-acknowledgement session (main head f885c19). Read the development log, PROJECT_STATUS.md, the process/context-switch sources, the x86_64 self-test, boot-image.yml and the latest commits before changing anything.
- All work was done on branch `feat/process-context-persistence-2026-11-09` (cut from main f885c19) as commits 4f276b7, fb171e7 and fedf73e. Nothing was force-pushed; open PRs #55/#53/#52/#50/#48 and all other branches were left untouched.

### Implemented

- kernel/src/process/mod.rs: new `ProcessManager::persist_outgoing_frame` (validates canonical RIP/RSP, clears stale completion metadata, applies the outgoing registers to the stored ProcessContext, rejects Created/Exited/unknown processes and malformed frames) and the `process_context` getter; the persistence contract is documented on `switch_to_dispatch`; three new unit tests cover register updates, stale-completion normalization and rejection paths.
- kernel/src/process/context_switch.rs + x86_64_switch.S: completion-carrying frames now accept saved target RSP of 16n as well as 16n+8; the assembly pushes one alignment pad in the 16n case so the target-stack callback always enters with RSP % 16 == 8 (r9 is used as the caller-saved pad flag). This is required because a persisted resume point has no fixed ABI parity at the switch call site. Unit test updated for both accepted alignments.
- kernel/src/arch/x86_64/selftest.rs: the two-stack self-test is now a two-process round-robin on separate fresh 4 KiB stacks. Each process yields once; the driver persists the outgoing frame and requires RIP advanced past entry plus RSP inside that process's own stack; each process must resume from the persisted context, exit, acknowledge idle and be reaped. All previous serial milestones are preserved for the CI smoke contracts.

### Validation

- First PR check suite (head before the fix commit) failed on three real, diagnosable errors: static-mut mutable references (as_mut_ptr on the selftest stacks), unsafe-op-in-unsafe-fn bodies, and an assert_eq requiring Debug on ProcessContext. Fixed in a440d80 (addr_of_mut!, explicit unsafe blocks, is_none()); the rustfmt bot then applied formatting as e1f12fc.
- Second failure, diagnosed from the real QEMU serial log in the `exception-injection-selftest` job: the two-process round-robin halted with "process A failed to yield back to the boot stack". Root cause: the round-robin driver left each process's SwitchFrame storage empty until the first yield, so `switch_checked` rejected the outgoing frame (InvalidCurrent) before the backend could write the real outgoing registers. The proven single-process path always copied `target.frame` into the task frame storage before dispatching. Fixed in 4561e67: `dispatch_process` now publishes the dispatch target frame (entry context on first dispatch, persisted resume context on re-dispatch) into the process's frame storage before switching.
- Final full-suite validation on PR #63 head 4561e67: all 16 check-runs completed **success** (Actions run IDs from the check-run URLs): `cloud-security` + `cloud-release-contract` — run 42700430843; `Rust workspace` + `UEFI loader` — run 42700430760; `Workspace Build & Validation` — run 42700431007; `Workspace clippy with PR diagnostics` — run 42700430716; `cargo failure diagnostics` — run 42700430953; `exception-injection-selftest` + `build-and-smoke-test-image` — run 42700431073; `virtio-block-contract-and-qemu` — run 42700430930; `Build Multi-Arch Boot Images` — run 42700430739; `Build AOSIN Installers` (ubuntu/windows, gui/cli, 4/4) — run 42700431117; `Apply rustfmt and commit fixes` — run 42700417531.
- The decisive runtime evidence is the green `exception-injection-selftest` on head 4561e67: the QEMU self-test ran the two-process round-robin (A and B on separate fresh 4 KiB stacks, each yielding once, outgoing frames persisted, each resumed from its persisted context, exited, acknowledged idle and was reaped), then completed the NMI/IST1 phase and printed "ALL PHASES PASSED".

### Remaining risks and honest scope

- The persist/resume path is still single-core, shared-kernel-address-space, interrupts-disabled; no Ring 3, per-process page tables, preemptive scheduler tick or SMP.
- `ExecutionCore` and `ProcessManager` remain separate current-task models; unification is still pending.
- VirtIO block I/O, network, storage persistence and AYUI are untouched by this session.

### Files changed in this session (branch feat/process-context-persistence-2026-11-09, PR #63)

- kernel/src/process/mod.rs — persist_outgoing_frame, process_context getter, dispatch-loop persistence contract, unit tests.
- kernel/src/process/context_switch.rs + kernel/src/process/x86_64_switch.S — dual target-RSP alignment acceptance and the assembly alignment pad.
- kernel/src/arch/x86_64/selftest.rs — two-process round-robin; 4561e67 publishes the dispatch target frame before each switch (the QEMU-proven fix).
- docs/CELL_RUNTIME.md, docs/AI-DEVELOPMENT-LOG.md — contract documentation and this honest record.

### Next priorities

1. DONE: PR #63 merged into main as d6d7480 (merge method, expected-head-SHA compare against 445085f). The green validation above ran on PR head 4561e67 (+ docs commit 445085f, log-only); token pushes to main do not trigger Actions, so the merged main head has no direct Actions run of its own — the standard caveat recorded on 2026-11-08 applies. All work on feat/process-context-persistence-2026-11-09 is preserved; nothing was force-pushed or deleted.
2. Unify `ProcessManager` and `ExecutionCore` behind one authoritative current-task model.
3. Add guarded page-table roots and checked kernel/user mappings before any Ring-3 entry.
4. Continue VirtIO block I/O with real request/completion and read-after-write tests.

## 2026-11-10 — stale-PR triage: resolve all open PRs against main

### What was requested and what was done

- Request: merge all previously created PRs into main. Before acting, each open PR was audited against current main (check-run status, file-by-file comparison, real diffs). Result of the audit: every stale PR branch conflicted with main (update_pull_request_branch reported merge conflicts for #55, #53, #52, #50, #48), and their content is superseded by main's current state.

### Per-PR resolutions (honest reasoning recorded in each PR's closing comment)

- #55 (workspace manifests + workflows + docs): all five workflows and all docs already exist on main in newer form. The single change never applied — removing the unused `awe-platformd` and `awe-script` workspace dependency entries (cargo reports both as unused on every CI run) — was ported to PR #65. Closed as superseded; branch preserved.
- #53 (kernel CpuContext validation + EndUserRuntime error mapping): verified superseded by direct file comparison — main already has `is_canonical_x86_64_48` validation with both-halves/noncanonical tests, the dedicated `EndUserRuntimeError::CapabilityDenied` variant, and `Starting`-state acceptance. Closed as superseded; branch preserved.
- #52 (2026-11-08 end-user runtime attempt): superseded by main's contract-tested route (`runtime/end_user.rs`, fail-closed syscall dispatcher, Ring 3 explicitly deferred). The branch fails 7/13 checks on its stale head and would delete main's newer work (entry.rs -246, storaged -265, appd/awos.rs -540). Genuinely new pieces (PS/2 driver, input.rs, graphics.rs, desktop.rs) remain in the branch for a future standalone port. Closed as superseded; branch preserved.
- #50 (rflags + syscall pointer validation): rflags 0x0202 already on main; `is_valid_user_ptr` superseded by main's strictly fail-closed dispatcher that never interprets arguments as pointers. Closed as superseded; branch preserved.
- #48 (userspace_entry logging refactor): main deliberately removed `userspace_entry` (it claimed active Ring 3 with no isolated user address space) and already carries the 0x0202 fix. Merging would reintroduce removed unverified code. Closed as superseded; branch preserved.
- PR #64 (initd restart policies, created by an external bot, 16/16 green) is a draft and not authored by this automation; it was left untouched.

### Validation

- PR #65 (branch chore/workspace-deps-cleanup-2026-11-10, port of #55's surviving change on top of main): all 15 check-runs completed **success** (Actions run IDs from the check-run URLs): `cloud-security` + `cloud-release-contract` — run 42887720665; `build-and-smoke-test-image` + `exception-injection-selftest` — run 42887720608; `virtio-block-contract-and-qemu` — run 42887720623; `Build Multi-Arch Boot Images` — run 42887720602; `Rust workspace` + `UEFI loader` — run 42887720834; `Workspace Build & Validation` — run 42887720817; `Workspace clippy with PR diagnostics` — run 42887720703; `cargo failure diagnostics` — run 42887720537; `Build AOSIN Installers` (ubuntu/windows, gui/cli, 4/4) — run 42887720729.
- PR #65 merged into main as fd58c95 (merge method, expected-head-SHA compare against fa953a8). Standard caveat: token pushes and merges do not trigger Actions on main itself; the green evidence is the PR head run.
- No failing suite was merged at any point; no branch was deleted; nothing was force-pushed.

### Files changed in this session

- Cargo.toml (via PR #65) — two unused workspace dependency entries removed.
- docs/AI-DEVELOPMENT-LOG.md — this entry.
- Closing comments on PRs #55, #53, #52, #50, #48.

### Next priorities

1. Unify `ProcessManager` and `ExecutionCore` behind one authoritative current-task model.
2. Add guarded page-table roots and checked kernel/user mappings before any Ring-3 entry.
3. Continue VirtIO block I/O with real request/completion and read-after-write tests.
4. Optional: carefully port the PS/2 keyboard driver from the preserved #52 branch as a standalone PR with a green suite.
## 2026-11-10 — PS/2 driver port from the preserved #52 branch

### Baseline and branch discipline

- Continued from the stale-PR triage session (main head e7a12a5, PR #65 merged as fd58c95). Read the development log first and audited the current state before changing anything: main branch, open PRs, recent commits, drivers/ and arch/ module trees. External draft PR #64 (initd restart policies) was left untouched.
- Chose the recorded next-priority item 4 (port the PS/2 driver from the preserved feat/aweos-runtime-30-direct branch as a standalone PR) because priorities 1-3 (ProcessManager/ExecutionCore unification, page-table roots, VirtIO block I/O) are each multi-session refactors, while the PS/2 port is a self-contained, fully testable increment toward the AYUI input stack.
- All work was done on branch feat/ps2-driver-port-2026-11-10 (cut from main e7a12a5) as commit b5bf780. Nothing was force-pushed; no branch was deleted.

### Audit of the ported file (the branch version was broken)

- kernel/src/drivers/ps2.rs in the preserved #52 branch was never compiled or tested: the file was not wired into the module tree and the branch CI failed 7/13.
- Reading the branch code against its own tests exposed two real mouse-decoder defects:
  1. Movement bytes were decoded with overflow-bit arithmetic (dx = x - 256 only when the X-overflow bit was set), so every ordinary negative delta mis-decoded (0xFE -> +254). The branch's own test expected -2 and would have failed.
  2. A first mouse byte with overflow bits 6/7 set was stored instead of being rejected immediately, so the branch's malformed-packet test (which expected Err on the first byte) also would have failed.

### Implemented

- kernel/src/drivers/ps2.rs (new): PS/2 (i8042) driver with a set-2 scancode KeyboardDecoder (0xE0 extended / 0xF0 break prefixes, arrows, Enter/Tab/Escape/Backspace/Space), a MouseDecoder with corrected nine-bit two's-complement sign handling (sign-extension through i8), immediate InvalidPacket rejection of first bytes with overflow bits, always-one-bit-3 stream resynchronization, a bounded 128-slot FIFO EventQueue (fail-closed QueueFull), and a polled Controller whose x86_64 path uses the architecture-owned io_in8/io_out8 primitives with a bounded drain-and-enable init; all other architectures fail closed with Unsupported. All public APIs are documented.
- kernel/src/drivers/mod.rs: registered pub mod ps2 (one line, alphabetical).
- Eight unit tests: press/release/extended scancode sequences, extended-break prefix state reset, signed motion in both directions at the sign boundary, invalid-packet rejection with clean resync, desync drop, and queue FIFO/bounds/wraparound.

### Validation (real CI results)

- PR #66 (head b5bf780, unchanged by the rustfmt bot): all 15 check-runs completed **success** (Actions run IDs from the check-run URLs): Apply rustfmt and commit fixes — run 42892494782; Build AOSIN Installers — run 42892526134; cargo failure diagnostics — run 42892526161; cloud-security — run 42892526263; Workspace Build & Validation — run 42892526485; Rust workspace + UEFI loader — run 42892526490; Workspace clippy with PR diagnostics — run 42892526575; Build Multi-Arch Boot Images — run 42892526620; build-and-smoke-test-image + exception-injection-selftest — run 42892526795; virtio-block-contract-and-qemu — run 42892526787; cloud-release-contract — run 42892526795 job family.
- The new unit tests ran inside the green Workspace Build & Validation / CI pipeline suites on the PR head; the rustfmt check completed success with no correction push needed.
- PR #66 merged into main as 4a3492d (merge method, expected-head-SHA compare against b5bf780). Standard caveat: token pushes/merges do not trigger Actions on main itself; the green evidence is the PR head run.

### Honest scope

- No interrupt-driven input, no IRQ wiring, and no real-hardware or QEMU input validation is claimed: the controller is an explicitly polled primitive and the unit tests exercise the decoders and the queue, not hardware.
- Controller::init/poll are unsafe, compile their port-I/O path only on x86_64, and are not yet called from any boot path, service or AYUI component.
- ProcessManager/ExecutionCore unification (priority 1), per-process page-table roots (priority 2) and VirtIO block I/O request/completion (priority 3) remain untouched and still open.

### Files changed in this session

- kernel/src/drivers/ps2.rs — new, via PR #66.
- kernel/src/drivers/mod.rs — one-line module registration, via PR #66.
- docs/AI-DEVELOPMENT-LOG.md — this entry.

### Next priorities

1. Unify ProcessManager and ExecutionCore behind one authoritative current-task model (start with a bounded, contract-tested integration step).
2. Add guarded page-table roots and checked kernel/user mappings before any Ring-3 entry.
3. Continue VirtIO block I/O with real request/completion and read-after-write tests.
4. Integrate the PS/2 controller with the kernel IRQ path or a driverd input service before any AYUI input claims.


## 2026-11-11 — ProcessManager / ExecutionCore unification (PR #67)

### Baseline and branch safety

- Audited the current `main` head `312b4327f8c0f064723056aeb38d9ee1b2693500`, the workspace tree, recent commits, current status/readiness documents, CellManager integration notes, workflow runs, and open pull requests.
- The previous log's top priority was to unify `ProcessManager` and `ExecutionCore` behind one current-task authority. Implemented the work on `feat/unified-process-runtime-2026-11-11`.
- PR #67 is a **draft** targeting `main`. No main-branch merge, force-push, branch deletion, or change to external draft PR #64 was performed.

### Implemented

- `kernel/src/execution_core.rs`: replaced the duplicate process registry, run queue, scheduler and current-task field with a compatibility facade over `ProcessManager`. The facade now delegates process descriptors, contexts, queue state, pending dispatch, current identity, exit/reap, CPU accounting, and dispatch acknowledgement to the same manager.
- Process admission now requires an explicit validated `CpuContext`; metadata-only processes without an executable context are rejected.
- `kernel/src/process/mod.rs`: added bounded yield, block, unblock and current-task CPU-budget operations, an exact pending-dispatch accessor, and a blocked-to-idle acknowledgement. Blocking a queued process fails closed if queue removal cannot be confirmed.
- Dispatch identity is not published as current until the exact pending `DispatchTarget` is acknowledged. Outgoing register frames can be persisted through the same manager before the process is selected again.
- `kernel/src/ac_runtime.rs` and `kernel/src/ac_boot_gate.rs`: migrated the A-C contract path to explicit CPU contexts and the shared manager. The API documentation now states that AcRuntime's user ranges are metadata-only and its local FIFO IPC is not authenticated or capability-enforced userspace IPC; AcBootGate validates supplied metadata and is not a hardware boot sequence.
- `tests/product-core/src/lib.rs`: updated the contract test to acknowledge dispatch explicitly and persist the outgoing frame before rescheduling.
- `docs/CELL_RUNTIME.md`: replaced the stale statement that ExecutionCore still has an independent scheduler/current task with the new authority model and its remaining boot/Ring-3 limits.

### Validation — exact PR-head evidence

Validated code head: `58c2dfd596c708c0b4e3426c37c77471aa4aa67f`.

- AWEOS CI Pipeline run **43053697038**, workspace job **130637395526**: success. The kernel test binary reported **347 passed, 0 failed**; the product-core suite reported **11 passed, 0 failed**. The workspace formatting check completed successfully.
- AWEOS Quality Gate run **43053697452**: Rust workspace and UEFI loader jobs succeeded.
- Clippy report run **43053697027**: success.
- Cloud Security Gate run **43053697604**: both cloud-security and cloud-release-contract jobs succeeded.
- CI Diagnostics run **43053697388**: success.
- AOSIN installer workflow run **43053696963**: all four Linux/Windows GUI/CLI installer jobs succeeded.
- Rustfmt automation run **43053687964**: success.
- Final outcome (verified in the next scheduled run): Bootable Image Pipeline run **43053697042**, VirtIO Runtime run **43053697468** and Multi-Architecture Images run **43053697934** all ended **cancelled** on head 58c2dfd: their jobs exceeded the configured GitHub Actions maximum execution times (build-and-smoke-test-image 20m; exception-injection-selftest 25m; virtio-block-contract-and-qemu 25m; Build Multi-Arch Boot Images 25m) and were killed. No QEMU/image/device evidence exists for head 58c2dfd; the PR stayed a draft and was not merged.

### Corrections made during validation

- The first PR CI attempt found a non-const accessor incorrectly declared const; corrected.
- A subsequent test compile found an assertion requiring Debug on ProcessContext; changed the test to compare the persisted CPU fields.
- The outgoing-frame contract tests then exposed rustfmt differences; the repository rustfmt workflow corrected them. The validated code head above passed the workspace formatting check.

### Honest scope and remaining limits

- This is a **software-state unification**. It does not start a production preemptive scheduler from the normal boot path.
- The architecture-owned x86_64 switch path and QEMU two-stack round-robin self-test remain the separate hardware-switch evidence; the product-core test's acknowledgement is explicitly manual and is not a hardware context-switch test.
- No Ring 3, per-process page-table roots, supervisor/user mapping policy, safe user-copy path, SMP scheduling, or production syscall integration is enabled.
- AcRuntime remains a contract harness, not a production address-space or authenticated IPC implementation.
- No filesystem durability, real VirtIO block read/write, NIC TX/RX, or physical-hardware compatibility is inferred from the workspace test pass.

### Next priorities

1. Add guarded per-process page-table roots, explicit kernel/user mapping permissions, and checked user-copy primitives before any Ring-3 entry.
2. Inspect the still-running Bootable Image, VirtIO Runtime, and Multi-Architecture workflows; continue VirtIO block request/completion and read-after-write persistence only with QEMU evidence.
3. Wire a real kernel dispatch loop and CPU-timer accounting to the single ProcessManager authority, without enabling SMP until per-CPU current-task state and synchronization exist.
4. Integrate the polled PS/2 controller with the IRQ/driver-service path before claiming AYUI input support.

## 2026-11-11 (second session) — PR #67 CI verification: long jobs cancelled at timeouts, fresh full re-run queued

### What was done

- Continued from the 2026-11-11 unification session. Per its recorded next-priority item 2, inspected the final state of the three workflows that were still running when the previous entry was recorded (Bootable Image Pipeline run **43053697042**, VirtIO Runtime run **43053697468**, Multi-Architecture Images run **43053697934**), all on PR #67 validated code head 58c2dfd.
- Result: all three runs ended **cancelled**. Their jobs exceeded the configured maximum execution time and were killed by GitHub Actions (build-and-smoke-test-image 20m; exception-injection-selftest 25m; virtio-block-contract-and-qemu 25m; Build Multi-Arch Boot Images 25m). The virtio run additionally logged that no artifacts were produced.
- The previous entry's "still in progress" wording was corrected above with this real final outcome; a cancelled run is not counted as green.

### Diagnosis (honest reading of the evidence)

- The same PR head passed the nine workspace/quality/security/installer/clippy/rustfmt workflows in normal durations earlier that day, and the same three long workflows finished green in 3-8 minutes on PR #66 one day earlier on a near-identical tree.
- A kernel-hang theory cannot fully explain the Multi-Architecture job: it is a pure Rust build job with no QEMU, yet it also hit its 25m limit, while the same job passed in about 3.5 minutes on PR #66. The simultaneous timeout of all three long jobs on 2026-11-11 is most consistent with runner-platform slowness or queue starvation on that day, not with a code regression.
- This is a diagnosis, not proof. The decisive test is a fresh full-suite run: this commit (docs-only, deliberately without a [skip ci] marker) re-triggers every workflow on a new PR head. PR #67 stays a draft and must not be merged unless the Bootable Image, VirtIO Runtime and Multi-Architecture runs complete with success on the fresh head.

### Rules kept

- No Rust source changed in this session; no merge, no force-push, no branch deletion; external draft PR #64 untouched.

### Files changed in this session

- docs/AI-DEVELOPMENT-LOG.md — this entry plus the correction of the previous entry's in-progress claim.

### Next priorities

1. If the fresh re-run is fully green: mark PR #67 ready for review and merge with an expected-head-SHA compare-and-swap; record run IDs and the standard main-branch caveat (token merges do not trigger Actions on main).
2. If the long jobs time out again on a fresh head: treat it as a real defect, capture the job logs, and bisect between runner-platform slowness and a code-induced hang, starting from the QEMU serial log of exception-injection-selftest on the PR head versus PR #66.
3. Then continue with guarded per-process page-table roots and checked user-copy primitives before any Ring-3 entry, and VirtIO block request/completion with read-after-write persistence.

### Final outcome of this session (same day)

- The docs-only re-trigger commit 0cf0f09 produced a fully green suite on the fresh PR head: **16/16 check-runs success**, including the three previously cancelled workflows — Bootable Image Pipeline (exception-injection-selftest + build-and-smoke-test-image) run **43094204694**; VirtIO Runtime (virtio-block-contract-and-qemu) run **43094204603**; Multi-Architecture Images run **43094204786**. Also green: CI Pipeline / Rust workspace + UEFI loader run **43094204605**; Workspace Build & Validation run **43094204721**; Clippy with PR diagnostics run **43094204705**; cargo failure diagnostics run **43094204706**; cloud-security + cloud-release-contract run **43094204829**; AOSIN installers run **43094204599** (4/4 jobs); rustfmt automation run **43094193967**.
- This confirms the earlier cancellations were runner-side timeouts/queue starvation on 2026-11-11, not a code defect: the same long jobs passed on a tree whose only difference from 58c2dfd is documentation.
- PR #67 was marked ready for review and merged into main as **97a4bb120c5751095fb52746df200819423f7786** (merge method, expected-head-SHA compare against 0cf0f09). Standard caveat: token merges do not trigger Actions on main itself; the green evidence is the PR head run 0cf0f09.
- The ProcessManager / ExecutionCore unification (one authoritative current-task/lifecycle model) is now on main. External draft PR #64 remains open, untouched and conflicting (mergeable_state dirty); it is not authored by this automation.

### Next priorities (updated)

1. Add guarded per-process page-table roots, explicit kernel/user mapping permissions, and checked user-copy primitives before any Ring-3 entry.
2. Wire a real kernel dispatch loop and CPU-timer accounting to the single ProcessManager authority, without enabling SMP until per-CPU current-task state and synchronization exist.
3. Continue VirtIO block I/O with real request/completion and read-after-write persistence tests, with QEMU evidence.
4. Re-evaluate external draft PR #64 (initd restart policies) against current main: rebase or ask for a rebase; it conflicts and must not be merged as-is.
5. Integrate the polled PS/2 controller with the IRQ/driver-service path before claiming AYUI input support.

---

## 2026-11-12 — CellNet: first executable distributed-cell fabric (directive §V)

### What was done

- Full-tree audit (`docs/ARCHITECTURE_AUDIT_2026-11-12.md`): local builds and tests executed on the pinned nightly; no readiness claims taken from documents. Baseline before this session: 481 workspace tests green, bare-metal `x86_64-unknown-none` kernel build green, UEFI loader check green, Clippy gate clean.
- `awe-securityd::crypto`: added X25519 key agreement (with contributory / low-order rejection), HKDF-SHA256, and ChaCha20-Poly1305 AEAD wrappers. Validated against RFC 7748 (DH vector + low-order rejection), RFC 5869 (HKDF case 1) and RFC 8439 (AEAD vector + tamper rejection).
- New service crate `awe-cellnetd` (`services/cellnetd`): bounded, allocation-free distributed cell fabric per `docs/CELLNET_ARCHITECTURE.md` — signed discovery announcements, Open/Pinned trust, mutually authenticated 4-flight handshake (Ed25519 identity, X25519 ephemeral, HKDF-SHA256 schedule, HMAC transcript confirmation), ChaCha20-Poly1305 session envelopes with a 64-bit anti-replay window, publish-policy-gated remote cell IPC, suspect/offline liveness, exponential-backoff reconnect hints, and DirectorySync anti-entropy for partition recovery. Key material is scrubbed on session teardown; no hidden RNG (caller-supplied nonces/secrets).
- 22 new tests in `awe-cellnetd` + 4 new crypto tests: handshake roundtrip, encrypted IPC both directions, ciphertext tamper, frame replay, signature forgery, node-id spoofing, pinned-trust rejection, low-order key rejection, publish-policy filtering, unpublished-cell drop + stale-entry repair via snapshot, suspect→offline→backoff→reconnect→partition-heal, and offer TTL expiry.
- Validation after the change: `cargo test --workspace --no-fail-fast` **507 passed, 0 failed**; `cargo fmt --all --check` clean; Clippy CI gate flags clean; `cargo build --release -p aweos-kernel-bin --target x86_64-unknown-none` clean; aweloader UEFI check clean.

### Honest limits

- The fabric's `Transport` trait is loopback-tested only; no NIC/UDP wiring yet (needs the netd QEMU path first). No CellIpc retransmission/ordering (at-most-once), no multicast discovery, no ratchet.
- No changes to the kernel crate; the trusted computing base is untouched.

### Next priorities (unchanged order, see audit doc)

1. Per-process page-table roots + checked user copy before any Ring-3 entry.
2. Timer-driven dispatch loop on the unified ProcessManager.
3. VirtIO block request/completion with read-after-write persistence in QEMU.
4. netd UDP path in QEMU; then bind cellnetd to it and run a two-instance mesh.

## 2026-11-12 — CellNet trust-boundary hardening follow-up

### Changes

- accept_hello now rechecks node-id/public-key binding, trust policy and Ed25519 signature, and accepts only the exact Hello frame previously validated by inspect_hello.
- Peer liveness is refreshed only after the AEAD envelope and anti-replay checks succeed; bad-tag traffic can no longer keep a peer alive.
- Remote IPC now requires the claimed source cell to exist in that authenticated peer's directory; outbound IPC requires a currently published local source cell.
- Clarified that required_caps is advertised metadata until a trusted caller-capability token is integrated at the service boundary; it is not represented as an enforced per-caller ACL.
- Added regression coverage for substituted Hello frames, bad-tag liveness refresh, and local/remote source-cell spoofing.

### Validation

- Pending CI on the updated PR head. No QEMU/NIC transport claim is added; the fabric remains loopback-tested only.
