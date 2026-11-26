# AWE_OS CellKernel — Development Log, Run 19 (2026-11-25)

Dated run entry; to be merged into docs/AI-DEVELOPMENT-LOG.md at the next
log-reconciliation commit, following the repo's dated-log precedent.

- **Baseline:** `main` at `85682e0912686d8467758336fddb98ece0e5211f` (PR #88, Run 18, merged after 16/16 check-runs). Run 18's next priorities were read before any new work.
- **Implemented:**
  1. **Boot direct-map gate** — new `kernel/src/memory/boot_direct_map.rs`:
     - no-allocation `validate_boot_direct_map(info)` walks the BootInfo descriptor array exactly as `PhysicalFrameAllocator::try_from_boot_info` does (pointer/alignment/extent checks before any read) and gates every `MEMORY_REGION_USABLE` range against the 64 TiB `DIRECT_MAP_MAX_PHYSICAL` window via the existing `fits_direct_map_window`;
     - fail-closed `DirectMapGateError` variants (missing map, misaligned pointer, descriptor-array overflow, empty/overflowing/beyond-window usable region) with the offending region index, plus a `DirectMapBootReport` (usable range count, highest physical end);
     - new `KernelBootStatus::DirectMapWindowExceeded = 7` returned by `kernel_entry` before any allocator or page-table state exists: a memory map beyond the window now fails closed instead of reaching the allocator;
     - compile-time guard re-asserting the shipped 64 TiB bound; 8 host tests (in-window pass with report, exact-boundary pass, straddling rejection with named region, MMIO-beyond-window not gated, empty region, overflow region, missing map, misaligned pointer).
  2. **Kernel-half root QEMU probe** — `boot_kernel_root_probe` in `table_arena.rs` (feature `kernel-selftest`, x86_64, os none): through the same proven TableArena => PhysicalTablePool identity-window path, builds a real supervisor-only kernel-half root with `KernelPageTableBuilder` + `OwnedTableFrames`, installs one writable/non-executable kernel-heap mapping, validates it by hardware-walk readback (`leaf_entry`), asserts the user half is untouched, then releases the never-published root and every built table frame. Serial evidence: `[cell:kernmap] install/readback/user-half-untouched/tables/released` + PASS/FAIL.
- **Files changed:** `kernel/src/memory/boot_direct_map.rs` (new); `kernel/src/memory/table_arena.rs`; `kernel/src/entry.rs`; `kernel/src/memory/mod.rs`; `PROJECT_STATUS.md`; this dated log.
- **Intentional behavior change in host `kernel_entry` tests:** an overflowing usable region and a misaligned memory-map pointer now fail as `DirectMapWindowExceeded` (new gate) instead of `InvalidMemoryMap`, because boot fails before allocator construction. Overlapping regions still fail as `InvalidMemoryMap`. The tests were updated to assert the earlier, more conservative failure.
- **Test results:** to be executed by remote CI on this PR head; no local Rust toolchain is available in this run, so no green test result is claimed. The gate carries host tests; the kernel-half root probe is exercisable only by the QEMU kernel-selftest job.
- **Explicit limits:** the kernel-half root is still never published: no CR3 write, no TLB invalidation, no Ring 3 change; the gate gates usable RAM only (reserved/reclaimable/MMIO descriptors validated elsewhere); the direct map itself is still not built at boot.
- **CI note:** an early push to the PR branch briefly posted a placeholder and a probe version of `table_arena.rs` that dropped the module's existing test suite; a follow-up commit restored the full file with all original tests preserved.
- **Next priorities:**
  1. If CI on this head is green, wire the built kernel-half root into a checked pre-activation validation step and design the CR3-switch/TLB-invalidation contract with failure-path and isolation tests before Ring 3.
  2. Build each process root preserving supervisor-only kernel-half mappings; define root/table ownership and teardown before activating a process address space.
  3. Connect checked user-copy to a verified physical-access backend and test mapped/unmapped, read-only, cross-page and fault cases in QEMU.
  4. VirtIO block request/completion with read-after-write/flush/power-loss tests; then NIC/UDP two-node transport.

### 2026-11-26 — CI diagnosis and compile repair

- The first full PR #89 CI attempt on code head `703f59752036fde42a2d825fb9b0ea0e70c97d9d` failed in the kernel crate before tests could execute. Exact logs showed E0425 (missing `fits_direct_map_window` import), E0603 (private re-export path for `DIRECT_MAP_MAX_PHYSICAL`), and E0277 (unsupported `usize::from(u32)`).
- Fixed in commit `9d59967f8818d9211afa46a6282b7fe76f3d188e`: import `DIRECT_MAP_MAX_PHYSICAL` from `kernel_map`, import `fits_direct_map_window` directly, and cast the already bounded region count to `usize`.
- The old-head run had 8 failing workflow families and the AOSIN installer succeeded; these failures were compile blockers, not runtime test evidence. Fresh CI on the fix commit is pending. No green result is claimed.

### 2026-11-26 — second compile pass and probe ownership correction

- The first compile-fix head exposed additional issues: the kernel binary did not handle the new `DirectMapWindowExceeded` enum variant; the test helper declared `MemoryRegion.kind` as `u8` although the boot protocol defines it as `u32`; and the kernel-root probe's explicit `PhysicalTablePool<_>` type selected the default ledger capacity (256) while the probe pool used capacity 8.
- Fixed the kernel-bin match, corrected the test helper type, and let `OwnedTableFrames` infer the concrete pool type/capacity.
- Runtime review found that dropping `OwnedTableFrames` automatically tears down its tracked frames. The probe now calls `take_ownership()` and then releases those transferred frames explicitly in reverse order, preventing a double release and avoiding the previous fixed eight-frame capture limit.
- Boot status semantics were tightened: only `BeyondDirectMapWindow` maps to `KernelBootStatus::DirectMapWindowExceeded`; malformed descriptor arrays, misalignment, empty/overflowing ranges map to `InvalidMemoryMap`. The corresponding entry tests were updated.
- Current code head at this point: `36fe7420b733dee8bd9cd74b157facf22c590057`. CI for this exact head is not yet confirmed; no test pass is claimed.


### 2026-11-26 — Clippy regression fix on PR #89

- **CI evidence:** On PR head `2762eeb2241d5618f881d47c4b794c1d4db54dad`, the workspace compile, workspace tests, formatting, bootable ISO smoke boot, VirtIO block QEMU boot, UEFI loader, multi-architecture image build, AOSIN installer matrix and CI diagnostics completed successfully. The Quality Gate, Cloud Security Gate and exception-injection self-test gate failed at Clippy before the QEMU exception-injection test could run.
- **Root cause:** all three failing Clippy paths reported `clippy::unnecessary_cast` at `kernel/src/memory/boot_direct_map.rs:85`: `MAX_MEMORY_REGIONS` is already `u32`, matching `BootInfo.memory_region_count`; the `as u32` cast was redundant. This was a lint failure, not a Rust compile or unit-test failure.
- **Fix:** commit `146d71adc81d13f727c8a665a2e0deb0f0dd4446` removes only the redundant cast. The comparison remains type-correct and preserves the 4096-descriptor bound.
- **Files changed in this follow-up:** `kernel/src/memory/boot_direct_map.rs`.
- **Validation:** no local Rust/QEMU toolchain is available in this GitHub-only execution. The fix has been pushed to the existing PR #89 branch; fresh remote CI is pending. The previous green workspace tests and QEMU ISO/VirtIO checks are recorded as evidence on the previous head only, not as validation of this fix commit. No merge/readiness promotion is claimed.
- **Next priorities:** (1) inspect every workflow on the new head and fetch exact logs for any remaining failures; (2) require Clippy and the QEMU exception-injection job to pass before merge; (3) review the kernel-half root probe's serial PASS evidence and ensure the root remains unpublished; (4) only then specify CR3/TLB activation and per-process root teardown.
