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
