## 2026-11-27 — Run 21: repair PR #90 const-evaluation regression

- **Baseline:** continued on the existing open PR #90 branch `feat/cr3-tlb-activation-contract-2026-11-26` at `ecc3a354b6d5b011cd2084f44ae7462547241d18`; main remains unchanged.
- **CI diagnosis:** all substantive failures on the initial PR #90 head shared Rust E0658 in `cr3_contract.rs:117`: `Result::map_err` and `?` were used inside a `const fn`, unsupported by the stable toolchain. This prevented workspace tests, Clippy, boot/QEMU, VirtIO, multi-architecture and cloud-security jobs from reaching their later gates. UEFI loader and AOSIN installers passed.
- **Code fix:** replaced the const `map_err/?` expression with explicit `match`, preserving the fail-closed error mapping. Corrected `cr3_low_flags` to return only CR3 bits 0..11 and added a regression assertion that reserved high bits are excluded.
- **Files changed:** `kernel/src/arch/x86_64/cr3_contract.rs`; `PROJECT_STATUS.md`; this log; new dated Run 21 log.
- **Validation:** the pre-fix exact head `ecc3a354b6d5b011cd2084f44ae7462547241d18` failed with E0658. The first follow-up head passed Quality Gate and CI Pipeline (including workspace check, tests and Clippy; 476 kernel tests + 4 kernel-bin tests), but the bare-metal `kernel-selftest` compile exposed a separate E0432 invalid re-export of `boot_cr3_contract_probe`. The ISO smoke boot itself passed; the exception-injection QEMU run did not start. The invalid re-export is now removed and the parent probe is feature/target-gated; fresh exact-head CI is pending.
- **Second follow-up CI:** exact code head `dcf7242a25943bd33937346c1629980e6a8f2bda` passed the Bootable Image Pipeline end-to-end (kernel build, Multiboot2 validation, ISO/GPT image creation, QEMU smoke boot, and exception-injection QEMU self-test), Multi-Architecture Images, CI Diagnostics, and all four AOSIN installer jobs. Quality Gate and Cloud Security failed only on rustfmt; VirtIO Runtime stopped at its formatting gate before storage/driver tests. The rustfmt workflow produced formatting-only commit `8834463e52122759feb710d6b1a2d1f082e7e972`, whose sole diff removes one extra blank line in `table_arena.rs`. GitHub marked the workflow runs for that bot-generated head `action_required` with no jobs, so they are not validation evidence; a follow-up documentation commit is being used to trigger checks on a new exact head.
- **Safety:** PR #90 remains open for review; no force push, merge, branch deletion, CR3 write, TLB switch or Ring 3 change.
- **Next:** validate all checks on the exact new head; only then review/merge. Higher-half kernel loading remains the prerequisite for a future CR3 activation milestone.

---

## 2026-11-26 — Run 20: repair Run 19 boot direct-map gate compilation

- **Baseline:** continued on the existing open PR #89 branch `feat/boot-direct-map-gate-2026-11-25`, head `703f59752036fde42a2d825fb9b0ea0e70c97d9d`. The PR was left open; no main-branch changes, merges, force pushes, or branch deletions were made.
- **CI audit:** all eight code/boot workflow runs associated with the PR head had completed. The AOSIN cross-platform installer passed; the Quality Gate, Clippy, Cloud Security, CI Pipeline, VirtIO Runtime, Multi-Architecture Images, CI Diagnostics and Bootable Image Pipeline failed because the same three Rust compile errors prevented the kernel crate from building.
- **Root causes from exact GitHub Actions job logs:**
  1. `boot_direct_map.rs` called `fits_direct_map_window` without importing it.
  2. It imported `DIRECT_MAP_MAX_PHYSICAL` through a private import in `kernel_page_table_build`; the public defining constant is in `kernel_map`.
  3. `usize::from(u32)` is not implemented on the target toolchain; the bounded `memory_region_count` is now converted with `as usize` after the existing maximum-count guard.
- **Implemented:** commit `9d59967f8818d9211afa46a6282b7fe76f3d188e` updates only `kernel/src/memory/boot_direct_map.rs`: imports the helper and physical limit from their defining modules and corrects the integer conversion. Existing validation order and fail-closed behavior are preserved.
- **Files changed in this run:** `kernel/src/memory/boot_direct_map.rs`; this log; the dated Run 19 log.
- **Validation:** the previous PR head failed workspace compilation with E0425, E0603 and E0277. The fix commit was pushed to the existing PR branch; fresh CI results for the new head are pending. No passing result is claimed yet. Local Rust/QEMU execution is unavailable through the connected GitHub-only development path.
- **Second CI diagnostic pass:** after the first three errors were fixed, the next exact-head run exposed additional compile/runtime-probe issues before tests could pass: `kernel-bin` had a non-exhaustive `KernelBootStatus` match; the new test helper used `u8` although the boot protocol stores `MemoryRegion.kind` as `u32`; the kernel-root probe's explicit `PhysicalTablePool` type assumed the default ledger capacity instead of the configured probe capacity; and `OwnedTableFrames::drop` would release intermediate frames before the probe's manual release, causing double-release evidence. The fixes add the boot-status arm, use the protocol's `u32` type, infer the pool backend type, and call `take_ownership()` before explicit reverse-order release. A final semantics correction maps malformed/misaligned/overflowing memory-map descriptors to `InvalidMemoryMap`; only a genuinely out-of-window usable RAM range returns `DirectMapWindowExceeded`.
- **Follow-up commits:** `305414c3884b3e45082c692cf2ee07a1821c50fe` (test type + rustfmt), `0ff4b299aa6c15a25aa530605facbec04c0b10ce` (boot status handling), `d80d9899f17299a8c437f1c09116ef22d7b6574c` (owned table-frame release), `36fe7420b733dee8bd9cd74b157facf22c590057` (precise boot status mapping).
- **Unresolved:** rerun all PR checks on the exact fix head; inspect any newly exposed compile/Clippy/QEMU failures before declaring Run 19 validated.
- **Next priorities:** once CI is green, merge only after review and exact-head validation; then continue the memory roadmap with full pre-activation kernel-root validation and a separately specified CR3/TLB contract. Do not enable Ring 3 or claim process isolation before per-process root construction, mapping permissions, teardown and QEMU isolation tests exist.

---

## 2026-11-25 — Run 18: kernel-half page-table builder hardening and CI repair

- **Baseline:** `main` at `57897c72b8ab307ba6c7ae5fd2336b9dcfda98a9`; resumed the unfinished Run 17 kernel-half page-table builder on PR #87. PR #87's first CI exposed an unregistered module; follow-up CI then exposed host-test compilation and policy-fixture errors. No changes were made to `main`.
- **Implemented on PR #88** (draft, branch `fix/kernel-page-table-builder-compile-2026-11-25`):
  - Registered `kernel_page_table_build` in `kernel/src/memory/mod.rs` and exported the backend-independent `validate_kernel_mapping` helper.
  - Fixed the direct-map bound check: the physical end is now compared to `DIRECT_MAP_MAX_PHYSICAL` (64 TiB), not `DIRECT_MAP_WINDOW_END` (a high-half virtual address). Added exact-boundary and straddling-range regression assertions.
  - Removed `GLOBAL` from intermediate paging entries. The bit is now preserved only on leaf entries when explicitly requested by the mapping policy.
  - Added `OverlappingMappings` rejection to `install_all`, so overlapping virtual ranges are detected before any page-table mutation.
  - Corrected the builder contract: invalid/overlapping batches are rejected before mutation, but allocation/backend failures during installation are not rollback-transactional. On any such error the root must remain unpublished and be discarded.
  - Fixed the generic associated-validator inference problem, test-only `PageTable` initialization, invalid-root assertions, and the writable-kernel-text test fixture.
- **Files changed:** `kernel/src/memory/mod.rs`; `kernel/src/memory/kernel_page_table_build.rs`.
- **Host regression evidence at exact code head `acd38595c4d06da922d14d578108bdeeb347f5da`:**
  - The kernel page-table builder's 17 tests all passed, including direct-map boundary checks, leaf/intermediate flag behavior, batch-overlap rejection, W^X and region-policy checks, and allocator exhaustion.
  - `cargo test --workspace --all-targets`: **460 passed, 0 failed**.
  - Workspace compilation, formatting, Clippy, UEFI loader, bootable-image smoke test, exception-injection self-test, VirtIO/QEMU contract, multi-architecture image build, Cloud Security/Release checks, and all four Linux/Windows AOSIN installer jobs passed.
  - The CI Diagnostics workflow's cargo collection step passed on retry and independently reported 460/460 workspace tests. Its final PR-comment step failed twice with GitHub API `HttpError: Unexpected end of JSON input`; this is an external comment API failure, not a Rust/test failure. The check remains non-green for that infrastructure reason.
- **Limitations:** PR #88 is still a draft; no merge was performed. The new kernel page-table builder is still host-tested only: it is not called from `kernel_entry`, does not install the production root, does not write CR3, does not perform TLB invalidation, and does not enable Ring 3. The direct-map range helper is not yet wired into boot-memory-map validation. Cross-architecture image workflow success is not proof of ARM64/RISC-V kernel runtime support.
- **Run 17 contract clarification:** its earlier “all-or-nothing install_all” wording was too broad. Only preflight rejection is mutation-free; backend/resource failure after installation starts can leave a partial unpublished root.
- **Unresolved CI issue:** the optional diagnostics-comment step can fail when GitHub returns a truncated/empty JSON response. Core compile/test and runtime workflows passed.
- **Next priorities:**
  1. Add a no-allocation boot-memory-map validator that checks only `MEMORY_REGION_USABLE` ranges against the 64 TiB direct-map limit before allocator/page-table use; return a distinct fail-closed boot status and test malformed/straddling ranges.
  2. Add a `kernel-selftest` path that builds a kernel-half root through the real `PhysicalTablePool` and validates readback without activating it.
  3. Only after QEMU evidence, plan the CR3 switch/TLB invalidation contract; keep Ring 3 disabled until per-process isolation is real.

### Run 18 follow-up — CI comment reliability (2026-11-25)

- Updated `.github/workflows/ci-diagnostics.yml`: PR-comment delivery now retries three times and degrades to a warning if GitHub returns an empty/truncated JSON response. Cargo diagnostics remain authoritative: a failed Clippy/check/test command still fails the diagnostics job; only optional comment delivery is best-effort.
- Exact final validation head: `8ace162a891190bec6665d9d13cc9712990d2e7c`.
- **GitHub Actions: 16/16 check-runs passed.** This includes workspace compile/tests/Clippy, 460/460 workspace tests, all 17 kernel page-table builder tests, UEFI loader, bootable-image smoke test, exception-injection self-test, VirtIO block QEMU boot, multi-architecture images, Cloud Security/Release, all four Linux/Windows AOSIN installer jobs, rustfmt, and CI diagnostics.
- The CI Diagnostics cargo-collection step passed and reported 460/460 tests. GitHub's PR-comment endpoint still returned `Unexpected end of JSON input` on all three attempts; the new best-effort handling emitted warnings and correctly kept the validation job green.
- PR #87 was closed as superseded by PR #88; its branch and commits were preserved. PR #88 remains unmerged and is ready for review.
- The kernel-half page-table builder remains **host-tested only**. Existing QEMU/UEFI/VirtIO boot checks do not activate this new builder, do not write CR3 for it, and do not establish TLB invalidation or Ring 3 readiness.


---

## 2026-11-24 — Run 17: supervisor-only kernel-half page-table builder

- **Baseline:** `main` at `57897c72b8ab307ba6c7ae5fd2336b9dcfda98a9` (post-PR-#86 merge plus Run 16 status commits). PR #86 and Run 16 are fully merged; the Run 16 next-priority list was read before any new work.
- **Implemented (new `kernel/src/memory/kernel_page_table_build.rs`):**
  - `KernelPageTableBuilder` builds PML4 -> PDPT -> PD -> PT entries through the same `TableMemory` backend contract as the user-half builder.
  - Hard rules: kernel half only (never writes PML4 indexes 0..255), supervisor only (`USER` rejected on leaves, never set on intermediates), planned regions only (every mapping must start inside a `kernel_map` region and cannot cross its half-open boundary).
  - W^X policy: writable kernel mappings must be non-executable; writable mappings are rejected outright inside the kernel text region.
  - `KERNEL_INTERMEDIATE_FLAGS` = `PRESENT|WRITABLE|GLOBAL` with no USER bit; bounded 512-table tracking; all-or-nothing `install_all` (whole batch preflighted before any mutation); `leaf_entry` hardware-walk diagnostics helper; `user_half_is_untouched` observability helper.
  - `fits_direct_map_window` and `validate_ram_ranges`: boot-side direct-map fit validation honoring the 64 TiB bounded window, naming the first offending physical range, so oversize memory maps are rejected instead of silently growing the window.
- **Files changed:** new `kernel/src/memory/kernel_page_table_build.rs`; `kernel/src/memory/mod.rs` (module registration + re-exports); `PROJECT_STATUS.md` (evidence bullet + next-priority 1 update); this log entry.
- **Test results:** 15 new host tests added in the module (user-half / USER / HUGE rejection, outside-region and cross-region-boundary rejection, text-region W^X policy, global W^X policy, install + readback with supervisor-only GLOBAL intermediates, double install without growth, atomic batch rejection, direct-map fit acceptance and rejection, first-offender naming, invalid root, out-of-frames fail-closed). To be executed by remote CI on this PR head; no local Rust toolchain is available in this run, so no green test result is claimed here.
- **Explicit limits:** the builder is not called from `kernel_entry`; nothing installs page tables, writes CR3, or invalidates the TLB. The higher-half layout remains a validated contract plus an unpublished builder; QEMU layout/permission evidence, boot-range wiring, CR3/TLB switching and process-root integration are still open.
- **Branch/PR:** `feat/kernel-half-page-table-builder-2026-11-24`, draft PR against `main` until all CI gates pass on the exact head.
- **Next priorities:** (1) wire the builder into the `kernel-selftest` boot path with QEMU layout/permission evidence, preserving the TableArena PASS and foreign-frame rejection assertions; (2) run `validate_ram_ranges` over the derived BootInfo/RAM ranges during boot; (3) then continue the existing Run 16 order (CR3/TLB policy, user-copy backend wiring, VirtIO block I/O, NIC/UDP transport, ARM64/RISC-V builds).

## 2026-11-24 — Run 16: repair, validate and merge higher-half layout contract

- **Baseline:** `main` at `00e8f3f14c7b07c8153fa21fde13806dc153b3be`; draft PR #86 initially at `f6b18f965bb1fd962e60636e098c3d8ee0cbe803`.
- **CI diagnosis:** the first PR run failed Rust test compilation because two `assert_eq!(Option<VirtualAddress>, None)` calls imposed `Debug`/`PartialEq` bounds on the address newtype. A test-only `FrameReserver` import was redundant. After those fixes, deeper review found `is_kernel_half()` accepted noncanonical upper-hole values and the original oversized-map test did not actually call the pairwise validator. A later Clippy run also caught `clippy::identity_op` in the overflow-test fixture.
- **Implemented:**
  - Replaced the two `Option<VirtualAddress>` equality assertions with `.is_none()` and removed the redundant `FrameReserver` import.
  - Made `is_kernel_half()` require both the kernel-half threshold and 48-bit canonicality.
  - Added a compile-time assertion over the complete shipped layout using `validate_layout()`.
  - Added half-open region-boundary coverage, including adjacent heap/per-CPU boundaries and intentional gaps.
  - Factored pairwise validation into a const helper that distinguishes unordered regions from overlap; added explicit overlap, unordered, noncanonical and overflow rejection tests.
  - Simplified the overflow-test mask to satisfy Clippy without changing production behavior. Final `kernel_map.rs` coverage: 14 host tests.
- **Files changed:** `kernel/src/memory/kernel_map.rs`, `kernel/src/memory/table_arena.rs`, `PROJECT_STATUS.md`, and `docs/AI-DEVELOPMENT-LOG.md`.
- **Final validation:** exact PR head `32387a44460cab52215d5981a6fd0cd94d178a18` passed **9/9 workflow families**: AWEOS Quality Gate, AWEOS CI Pipeline, Clippy, CI Diagnostics, Bootable Image Pipeline, VirtIO Runtime, Multi-Architecture Images, Cloud Security and AOSIN Cross-Platform Installers. CI diagnostics reported **443 kernel tests + 4 kernel-bin tests** passing. Workspace format/check/tests/Clippy passed; UEFI loader, QEMU exception-injection self-test, QEMU boot with VirtIO block device, multi-architecture image builds and all four Linux/Windows GUI/CLI installer jobs passed.
- **Merge:** PR #86 was marked ready only after the green checks, then merged with expected-head compare-and-swap into `main` at `bd47eca67259945c044454dfe31b09212e8a90b5`. No force push or branch deletion was performed.
- **Scope boundary:** this merge validates a virtual-layout contract, not an active higher-half mapper. No higher-half page-table installation, CR3 switch, TLB invalidation/shootdown, permanent direct map, Ring 3 entry or process-isolation proof was added. VirtIO CI proves the current boot/transport smoke path, not persistent block I/O, flush semantics or crash recovery.
- **Next priorities:** implement a supervisor-only kernel-half page-table builder that preserves the user half; validate all boot memory-map/BootInfo ranges against the direct-map cap; then add QEMU tests for mapping permissions, canonicality, guard gaps and boot-time failure handling before activation.

---
---
## 2026-11-23 — Run 15: PR #85 merged; higher-half kernel layout contract introduced

- **Merge:** PR #85 (TableArena reservation preflight + QEMU probe evidence, Run 14) was merged into `main` at merge commit `00e8f3f14c7b07c8153fa21fde13806dc153b3be` after its head `c1dc74ea4fea706d11fefc71773233adcea03a63` completed **all check-runs with conclusion success** (Rust workspace, Workspace Build & Validation, clippy-with-diagnostics, cargo failure diagnostics, UEFI loader, build-and-smoke-test-image, exception-injection-selftest, virtio-block-contract-and-qemu, cloud-security, cloud-release-contract, Build Multi-Arch Boot Images and all four AOSIN installers, plus the rustfmt bot commit). Merge used `expected_head_sha` compare-and-swap; no force push, no branch deletion; open draft PRs #83 and #64 remain untouched.
- **Implemented on `feat/higher-half-kernel-layout-2026-11-23`:** new `kernel/src/memory/kernel_map.rs` — the higher-half kernel virtual layout contract:
  - Single typed constants for the PML4-index-256 kernel-half base, a **bounded** 64 TiB direct map (translation returns `None` beyond the cap), kernel text/read-only data at `0xffff_c100…`, kernel heap at `0xffff_c180…` and per-CPU space at `0xffff_c1c0…`, all 2 MiB aligned.
  - `validate_layout()` / `validate_region()` const validators: misaligned, zero-length, non-canonical, user-half, overflow, overlap and unordered layouts fail closed.
  - `direct_map_address()` bounded physical→virtual alias translation; `kernel_region_of()` region lookup; `pml4_index()` / `is_kernel_half()` helpers.
  - 9 host unit tests in the initial Run 15 submission; Run 16 adds canonical-kernel-half and half-open region-boundary regression tests, bringing the current PR head to 14 tests. Coverage includes shipped-layout validity, PML4-index separation, bounded translation, direct-map/text separation, distinct region kinds, canonical-address checks, and invalid-region rejection.
  - `kernel/src/memory/mod.rs` registers and re-exports the new module.
- **Files changed:** `kernel/src/memory/kernel_map.rs` (new), `kernel/src/memory/mod.rs`, `PROJECT_STATUS.md`, this log.
- **Validation limits (honest):** no local Rust/QEMU toolchain exists in this execution environment; validation comes from remote CI on exact Commit SHAs, once available. The layout module is a validated contract, **not** an active mapper: no CR3 write, no higher-half page-table installation, no TLB invalidation, no Ring 3, no runtime boot evidence for these addresses yet. The next stage must wire these constants into the address-space builder and the boot mapper before any activation claim.
- **Next priorities:** review this PR's CI; then implement the higher-half kernel-half mirror of `UserPageTableBuilder` (kernel-half PML4 walk with supervisor-only flags) plus boot-time validation that `BootInfo` memory maps never exceed `DIRECT_MAP_MAX_PHYSICAL`.

---

## 2026-11-23 — Run 14: harden TableArena reservation preflight and require QEMU probe evidence

- **Baseline audit:** continued from main `a1248b8974919a970f329d6a72ae5cefc228f749` after PR #84 merged. The latest main-head CI family (workspace, Quality Gate, boot image, VirtIO, Cloud Security, Multi-Architecture and AOSIN installers) completed successfully; scheduled branch integration audit run #19 also succeeded. PR #84's exact head passed 16/16 check-runs. Open draft PRs #83 (branch-audit quoting) and #64 (initd restart policy) were preserved.
- **Repository reality check:** the `TableArena` / `PhysicalTablePool` code is present, but the QEMU exception self-test did not yet assert the `[cell:tables]` probe marker. A successful exception-injection test therefore did not independently prove the table-pool probe. The normal production boot path still does not retain this pool.
- **Implemented on `fix/table-arena-reservation-contract-2026-11-23`:**
  - Added `TableArenaReserveError` and preflight validation for zero frames, arena bitmap capacity, page alignment and physical-end overflow **before** calling the allocator's mutating reservation method. This prevents invalid arena shapes from permanently consuming frames and then failing later in `TableArena::open`.
  - Wrapped genuine allocator failures separately as `Reservation(BitmapFrameError)`, preserving their original reason.
  - Added a regression test that submits misaligned, oversized, zero-length and overflowing requests and proves the allocator still returns its original first frame.
  - Added `peek_contiguous_frames` to scan past boot-owned reservations and short usable regions without allocating; the table-arena probe now requests a full contiguous 16-frame run instead of trusting the first cursor page.
  - Added a regression test proving a four-frame preview skips a three-frame reservation and leaves the first free frame for the subsequent allocator call.
  - Tightened the in-kernel probe's PASS condition to include foreign-frame rejection, not merely readback/release/reuse.
  - Extended the QEMU exception self-test to require both `[cell:tables] probe result: PASS` and `foreign-rejected=1` in the serial log.
  - Refreshed `PROJECT_STATUS.md` and appended a superseding memory-path note to `PRODUCT_READINESS.md` so the merged PR #84 state and remaining limits are not misreported.
- **Files changed:** `kernel/src/memory/frame.rs`, `kernel/src/memory/table_arena.rs`, `kernel/src/memory/mod.rs`, `tests/product-core/src/lib.rs`, `PROJECT_STATUS.md`, `PRODUCT_READINESS.md`, and this log.
- **Validation:** no local Rust/QEMU toolchain is available in this execution environment. Remote CI evidence, kept separate by exact head:
  - Initial head `713db3c`: workspace compilation and all 428 tests passed; the formatting gate caught two nested error-assertion layouts. The rustfmt workflow produced formatting-only commit `548db367`.
  - Code head `d4b65c1`: workspace compilation and all 429 tests passed, including the new reservation-preflight and contiguous-run tests. The QEMU exception-injection self-test passed with both the table-pool PASS marker and `foreign-rejected=1`. Stable rustfmt found one line-layout difference; the formatting workflow produced `0ea2fbb`.
  - Latest fully inspected head `4c0370a`: **15/16 check-runs succeeded, zero failed, one still running**. Workspace build/tests (429 tests), stable formatting, workspace Clippy, UEFI loader, normal QEMU ISO boot smoke, VirtIO runtime, Cloud Security, Multi-Architecture images, AOSIN installers, cloud-release contract and CI diagnostics succeeded. The only pending check was the dedicated QEMU exception-injection job on this latest head; do not call the matrix fully green until it completes.
  - These CI results are the validation for the listed heads only; no local test execution is claimed.
- **Unresolved:** the `TableArena` remains a bounded validation arena, not a persistent production allocator; no production direct map, per-process CR3 activation, TLB shootdown, Ring 3, user/kernel isolation, durable VirtIO block I/O or NIC-backed multi-node CellNet is claimed.
- **Next priorities:** complete the full CI matrix on the latest PR #85 head; after green, define the higher-half kernel mapping and persistent direct-map/allocator lifetime before attempting per-process roots, CR3/TLB work and live checked user-copy.

---

## 2026-11-22 — Run 13 (CI follow-up): reservation-intersection test fixture corrected, PR #84 fully green

- **CI repair completed.** The single remaining test failure, `kernel::memory::frame::tests::reservation_intersection_report_is_conservative` at `kernel/src/memory/frame.rs:678` (426 passed / 1 failed), was a bad test fixture, not a production defect: it reserved `[0x2000, 0x3000)` but asserted that the adjacent range `[0x3000, 0x4000)` intersects it. The fixture now reserves `[0x2000, 0x3800)` (`reserve_range(0x2000, 0x1800)`), which genuinely covers frame 0x3000; all assertions (intersect at 0x3000, no intersect at 0x4000, overflow reported as intersecting) are coherent. The overflow short-circuit in `range_intersects_reservation` is unchanged and exercised by the test.
- **Files changed this follow-up:** `kernel/src/memory/frame.rs` (test fixture only). No production code paths and no workspace structure were touched.
- **Real CI results on this repo during Run 13:**
  - Head `7fd8fffc` (before fixture fix): `Rust workspace`, `virtio-block-contract-and-qemu`, `cloud-security`, `cargo failure diagnostics` failed on the single test above; UEFI loader, exception-injection-selftest, build-and-smoke-test-image, all four AOSIN installers, Workspace clippy and rustfmt passed.
  - Head `f5977317` (after fixture fix): **all 16 check runs completed with conclusion `success`**, including `Rust workspace` (full `cargo test -p aweos-kernel --lib` suite), `virtio-block-contract-and-qemu`, `cloud-security`, `Workspace Build & Validation`, `Build Multi-Arch Boot Images` and `cloud-release-contract` no longer skipped. Verified via the PR #84 check-run API on head SHA `f59773176fe00277291b989e92b614f554ba673d`.
- **Honest limits:** this run adds no CR3/TLB behavior, no ring-3 transition, and no VirtIO block I/O beyond what earlier runs established; the TableArena is a boot-time reservation and the probe evidence path remains QEMU-boot-gated.
- **Next priorities:** per-process page-table roots, checked user-space copy primitives, extending the table-arena probe into a persistent post-boot pool handoff.

---
#### 2026-11-22 — Run 13: live PhysicalTablePool over a reserved TableArena (evidence-probe boot integration)

- **Continuation:** built on main `9d5cf91d` (after the PR #81/82 memory-reservation track) and the Run 12 next priorities. Branch: `feat/table-arena-live-source-2026-11-22`. No force push, no branch deletion; PRs #83 and #64 untouched.
- **Implemented (`kernel/src/memory/table_arena.rs`, new):**
  - `reserve_table_arena(&mut frames, count)` reserves an arena of N table frames through the bounded boot-owned reservation ledger (bounded slots, all-or-nothing, reservation rejected if it conflicts with live allocations or boot ranges) **before** the first frame allocation.
  - The arena is a real `FramePoolSource`: its own bitmap supports true `allocate`/`release`, so a released table frame becomes reclaimable and re-allocatable zeroed, unlike the monotonic main allocator. Typed errors: exhaustion, double release, unknown release, reservation conflict (`TableArenaError`).
- **Live allocator additions (`kernel/src/memory/frame.rs`):** non-mutating `peek_next_frame()` for the probe and `range_intersects_reservation()` diagnostics. Monotonic allocation semantics untouched.
- **Boot integration (`kernel/src/entry.rs`):** immediately before the first `frames.allocate()` in `kernel_entry`, a `#[cfg(all(feature = "kernel-selftest", target_arch = "x86_64", target_os = "none"))]` call runs `crate::memory::table_arena::boot_table_pool_probe(&mut frames)`. Production boot builds contain no such code path. The probe reserves a small arena, builds a real `PhysicalTablePool` over it through the existing `DirectMapWindow` mapping, prints the `AWEOS: [cell:tables] ...` evidence lines, releases table frames and proves the reclaimed frame is re-allocated and re-zeroed.
- **Tests:** 6 new host tests (arena alloc order/exhaustion; release/double-release/unknown errors; pool-over-arena zero + modify + release + rezero against real frames; live-allocator conflict rejection; error propagation). The existing `PhysicalFrameAllocator` test suite is unchanged.
- **Validation:** CI is pending on PR #84 ([head 722e16b](https://github.com/ARARAT33/AWE_OS_Cellkernel/pull/84)). **No pass believed or claimed this run; build success is not counted as runtime proof.**
- **Honest limits (unchanged, not claimed):** no CR3 switching, TLB invalidation, per-process address spaces, Ring 3, direct-map window sourced from QEMU-verified mappings on other paths, virtio-udev VirtIO I/O or NIC-backed CellNet.
- **Next run priorities:** review PR #84 CI diagnostics; fix any findings; only after every check is green on the final head, mark ready and merge with an expected-head-SHA compare-and-swap; then continue per-process page-table roots + checked user-copy.

#### 2026-11-21 — Run 12: PR #81 merged; boot-owned ranges now reserved in the live kernel path

- **Merge:** PR #81 (head 491c746a25bde426ed31823454aa4fb0b7bccb72,
  16/16 check-runs passed on that exact head, mergeable_state clean) was
  marked ready for review and merged into main as merge commit
  42c4e4d1a166d42c5a8b3f7382bb6840ece13e22. No force push and no
  branch deletion were performed.
- **What landed in main:**
  - Bounded reservation ledger on the live monotonic `PhysicalFrameAllocator`:
    batch preflighted reservations, skip of every 4 KiB frame intersecting a
    reserved byte range, idempotent containment, rejection of reservations
    after allocation begins.
  - `boot_owned_ranges` / `reserve_boot_owned_ranges` wired into `kernel_entry`
    before the first frame allocation; reservation failure returns the distinct
    fatal `KernelBootStatus::BootReservationFailed` instead of continuing.
  - Linker-defined `__kernel_start` / `__kernel_end` bounds; `kernel-bin`
    populates `BootInfo.kernel_base/kernel_size` from those symbols.
  - Host regression tests for intersecting-frame exclusion, idempotent
    reservations, late-reservation rejection, live-allocator boot reservation
    and checked linker-bound arithmetic.
- **Honest limits (unchanged, not claimed):** no direct-map window, no
  `PhysicalTablePool` instantiation in the live boot path, no CR3 switching,
  TLB invalidation, Ring 3, QEMU user/kernel isolation, persistent VirtIO I/O
  or NIC-backed CellNet transport.
- **Next run priorities:** build a direct-map window only from mappings proven
  active in QEMU, instantiate the physical table pool over frames reserved by
  the new ledger, and add QEMU evidence that table frames are allocated,
  zeroed, mapped and safely reclaimed.



#### Validation completion — PR #81 exact head `17b54cc64eb07bec95aa3922597c4f934808efc0`

- **Result:** all **15/15 check-runs passed**, with zero failed or pending checks. PR #81 remains a draft; no merge or main-branch update was performed.
- **Workspace Build & Validation:** passed `cargo check --workspace --all-targets`, `cargo test --workspace`, `cargo fmt --all -- --check`, boot-image build and artifact verification. The new frame-reservation, batch-atomicity, live-allocator integration and linker-range tests all passed.
- **Clippy / Quality Gate:** passed with `-D warnings`; no lint suppressions were added.
- **Bootable Image Pipeline:** both jobs passed. The ISO boot smoke test reached `AWEOS: kernel is alive`; the exception-injection QEMU self-test passed.
- **VirtIO Runtime:** passed. QEMU exposed a `virtio-blk-pci` device and the kernel reached RUNNING; this is device-presence/boot evidence only, not block I/O or persistence evidence.
- **Other gates:** UEFI loader, Multi-Architecture Images, Cloud Security, Cloud Release Contract, CI Diagnostics and all four AOSIN installer jobs passed.
- **First-attempt failures:** the initial branch head failed formatting and Clippy due to rustfmt differences and unnecessary `unsafe` around immutable linker symbols. Both were corrected; the final tested head is clean.
- **Next:** keep PR #81 unmerged pending review, then prioritize a verified direct-map window and live `PhysicalTablePool` instantiation with QEMU allocation/zeroing/reclamation evidence.



#### CI follow-up — PR #81 first validation pass

- The first PR workflow set correctly caught two issues before merge: rustfmt differences in `boot_reserve.rs` and the linker-bound test, plus two `clippy::unused_unsafe` diagnostics around `addr_of!` of the immutable linker symbols.
- Corrected the formatting and removed the unnecessary unsafe blocks; no lint suppression was added.
- The first CI attempt is recorded as **failed**, not passed. The workspace test job was stopped at the formatting gate, and Clippy failed on the two unnecessary unsafe blocks. A fresh full workflow run is required for the corrected head.
- Main-head baseline successes remain separate from this branch's validation.



#### 2026-11-21 — Run 11: wire boot-owned reservations into the live boot path

- **Baseline:** continued from `main` at `b16abe9c55135240393b9689d0cab54504de0681`, after PR #80 merged. The previous run had derived boot-owned ranges but explicitly left them disconnected from `kernel_entry`.
- **Repository audit:** the default branch has 30 Cargo workspace members and 11 GitHub Actions workflows. The current main-head runs for the workspace CI, Quality Gate, Bootable Image Pipeline (including exception-injection QEMU self-test), VirtIO Runtime, Cloud Security Gate, Multi-Architecture Images and all four AOSIN installer jobs completed successfully. The latest PR #80 validation reported 16/16 checks green. These are the baseline results, not validation of this new branch.
- **Implemented on `feat/live-boot-memory-reservations-2026-11-21`:**
  - Extended the live monotonic `PhysicalFrameAllocator` with a bounded reservation ledger. It now rejects new reservations after allocation begins and skips every 4 KiB frame intersecting a reserved byte range; repeated fully-contained reservations are idempotent.
  - Implemented the existing `FrameReserver` contract for `PhysicalFrameAllocator`, so `reserve_boot_owned_ranges` can operate on the allocator actually used by `kernel_entry`.
  - Wired boot-owned range derivation/reservation into `kernel_entry` before the first frame allocation. Reservation failure now returns a distinct fatal boot status instead of silently continuing.
  - Added linker-defined `__kernel_start` / `__kernel_end` bounds and populate `BootInfo.kernel_base/kernel_size` from those symbols in the Multiboot/PVH kernel binary, covering the kernel image, boot page tables, boot stacks and in-image memory-map storage.
  - Added regression tests for intersecting-frame exclusion, idempotent reservations, late-reservation rejection, boot-owned reservation through the live allocator, and checked linker-bound arithmetic.
- **Files changed:** `kernel/src/memory/frame.rs`, `kernel/src/memory/boot_reserve.rs`, `kernel/src/entry.rs`, `kernel-bin/src/main.rs`, `kernel-bin/linker.ld`, and this log.
- **Validation:** pending PR CI on the exact branch head. No local Rust/QEMU run is claimed in this environment. The prior main-head green runs are not counted as validation of this change.
- **Remaining limits:** the live allocator is still monotonic (no frame release), and this change does not construct a direct map or instantiate `PhysicalTablePool`. The linker range is for the current x86_64 Multiboot/PVH image; other boot/architecture loaders must populate their own image bounds. No CR3 switching, TLB invalidation, Ring 3, persistent VirtIO I/O or NIC-backed CellNet transport is claimed.
- **Next priority:** after this branch is validated, construct a direct-map window only from mappings proven active, instantiate the physical table pool over reserved frames, and add QEMU evidence that table frames are allocated, zeroed, mapped and safely reclaimed.

#### 2026-11-20 — Run: PR #80 validated and merged-ready; boot-owned ranges green

- **Validation (head 9e6a3d6 / formatted tree):** all 16 PR check-runs
  passed, including workspace tests, Clippy, QEMU boot/VirtIO and
  cloud-security gates. The earlier heads of this PR are recorded as
  failures and are not counted as passes.
- **CI-caught defects fixed this run (all from bot diagnostics):**
  (1) `checked_range` returned `(start, end)` while callers stored the
  second value as a length, inflating every derived boot-owned range by
  its start address; (2) host tests relied on an empty-slice pointer,
  which is non-null, so a dangling pointer with a positive descriptor
  count passed boot validation and derived a bogus map range — the
  validate-failure test now uses a genuinely null pointer and the
  zero-field skip test expects the BootInfo struct plus the descriptor
  array; (3) a transitional commit accidentally stored a placeholder
  string in boot_reserve.rs, corrected in the immediately following
  commit on the same branch before any merge.
- **Status:** PR #80 marked ready and merged once this log head is
  green. Honest limits unchanged: boot-owned reservation is still not
  wired into kernel_entry; kernel-bin still fills kernel_base/kernel_size
  with 0; no CR3 switching, TLB invalidation, VirtIO persistence or
  NIC-backed CellNet transport yet.
- **Next priorities:** fill kernel_base/kernel_size in kernel-bin
  boot-info construction from linker symbols; wire the bitmap allocator
  and boot-owned reservation into kernel_entry after memory-map
  validation; then supervisor-only kernel mappings, per-process CR3
  switching and TLB invalidation.


#### CI follow-up — rustfmt bot reformatted boot_reserve.rs

- The suite on the first authored head a5b3ac8/e3af5ba triggered the
  formatter bot, whose commit 151b159 applied formatting-only changes to
  kernel/src/memory/boot_reserve.rs. No semantic source change.
- Bot-authored heads do not trigger Actions in this repository, so this
  owner-authored entry re-runs the full PR suite against the formatted
  tree. Until that run completes, all gates remain unverified for this
  head; the partial runs on earlier heads are recorded as incomplete,
  not as passes.
#### 2026-11-20 — Run: PR #79 merged; boot-owned range reservations started

- **Continuation:** PR #79 (reserved physical frame ranges) was draft with 16/16 green
  check-runs on its final head. It was marked ready and merged into main as
  merge commit 2686fbc; the branch and in-progress reserved-range work are live.
- **Implemented (branch feat/boot-owned-range-reservations-2026-11-20):**
  new kernel/src/memory/boot_reserve.rs. boot_owned_ranges derives the physical
  ranges a validated BootInfo owns, in deterministic order: kernel image,
  BootInfo structure, memory-map descriptor array, framebuffer, ACPI RSDP page.
  reserve_boot_owned_ranges reserves them on any FrameReserver all-or-nothing:
  every range is preflighted before any bitmap mutation, so a failed request
  leaves the allocator untouched. BitmapFrameAllocator gained the non-mutating
  check_reserve_range and reserve_range was refactored to preflight through it.
  Host tests cover in-order derivation, zero-field skipping, invalid-boot-info,
  kernel-image overflow, misaligned and count/pointer-inconsistent memory maps,
  non-mutating preflight, and reservation/idempotence/atomic-conflict behavior.
- **Honest limits:** not yet wired into kernel_entry or the live boot memory map;
  kernel-bin does not yet fill kernel_base/kernel_size from linker symbols, so at
  boot only the BootInfo, memory-map, framebuffer and RSDP ranges are derivable.
  Device-tree reservation is intentionally absent: the v1 protocol carries no
  length for that pointer. No direct map, CR3 switching, TLB invalidation or
  Ring-3 change is claimed.
- **Next priorities:** fill kernel_base/kernel_size in kernel-bin boot-info
  construction from kernel-image linker symbols; wire the bitmap allocator and
  boot-owned reservation into kernel_entry after memory-map validation; then
  supervisor-only kernel mappings, per-process CR3 switching and TLB invalidation.

#### 2026-11-20 — Run: reservation-aware bitmap frame allocation

- **Baseline audit:** continued from main head `d781f94344b700021ac5ce9bf2ce13f84426b1f2`. Inspected the workspace manifest, current project/readiness documents, the repository tree, memory/address-space/page-table/user-copy code, x86_64 bootstrap entry, boot memory-map handoff, Cell runtime contract, and the relevant Quality Gate and VirtIO workflows. Main-head Quality Gate, Bootable Image Pipeline, AOSIN installers, Multi-Architecture Images, Cloud Security Gate, CI Pipeline and VirtIO Runtime all completed successfully. The latest scheduled safe-branch audit also completed successfully. The VirtIO workflow proves QEMU device exposure and kernel boot, not block read/write or persistence.
- **Architecture finding:** `PhysicalTablePool` and checked user-copy now exist as bounded, testable layers, but neither is instantiated against the live boot memory map. Before exposing physical frames to the table pool, the allocator needs an explicit way to reserve kernel and boot-owned ranges and prevent those frames from being returned by `free`.
- **Implemented on branch `feat/reserved-physical-frames-2026-11-20`:** `BitmapFrameAllocator::reserve_range(start, length)` reserves every 4 KiB frame intersecting a non-empty byte range, rounds unaligned boundaries outwards, rejects overflow/out-of-range inputs, is idempotent for already-reserved pages, and preflights the whole range so a conflict with an allocated frame cannot partially mutate the bitmap. A second bitmap distinguishes permanently reserved pages from ordinary allocations; `try_free_frame` now rejects reserved frames with `ReservedFrame`.
- **Regression tests added:** outward rounding and idempotence; reserved-frame free rejection; atomic failure when a range overlaps a live allocation; empty, overflowing and out-of-range rejection with allocator state unchanged.
- **Files changed:** `kernel/src/memory/frame.rs`, `PROJECT_STATUS.md`, `PRODUCT_READINESS.md`, `docs/AI-DEVELOPMENT-LOG.md`.
- **Validation:** no local checkout/toolchain was available, so no local Rust test is claimed. The initial PR head failed Clippy on `clippy::manual_is_multiple_of`; that was corrected with `u64::div_ceil(PAGE_SIZE)`. On final source head `a225e73280a27cf4170c377175c24a23e1454d94`, all 16 PR check-runs passed. Quality Gate and CI Diagnostics each reported 405 kernel tests plus 3 kernel-bin tests, with zero failures (the suites overlap and must not be added). Formatting, workspace compilation, Clippy, UEFI loader, exception-injection selftest, QEMU boot-image smoke test, VirtIO QEMU device-detection/boot, cloud-security, multi-architecture-image build and all four AOSIN installer jobs passed. This still does not prove VirtIO block read/write or persistence.
- **CI correction:** the first PR attempt reached workspace Clippy and failed on `clippy::manual_is_multiple_of` in the range-ceiling calculation. The rustfmt workflow itself succeeded. Replaced the manual quotient/remainder expression with `u64::div_ceil(PAGE_SIZE)`; the failed head is recorded as a failure, and the corrected head requires a fresh full run.
- **Remaining limits:** this is an allocator primitive, not boot integration. The boot path still needs to reserve the actual kernel image, loader/boot metadata, memory-map storage, bootstrap page tables and stacks; then it must prove every frame exposed by the allocator is both usable and reachable through the active direct map. CR3 switching, TLB invalidation, Ring 3, persistent VirtIO I/O and NIC-backed CellNet remain open.
- **Next priorities:** (1) validate this branch with all PR workflows; (2) integrate reservation ranges from the validated boot handoff and kernel linker symbols without freeing loader-owned pages prematurely; (3) construct a direct-map window only for addressable frames and instantiate `PhysicalTablePool`; (4) copy supervisor-only kernel mappings into each process root and design a safe CR3/TLB transition; (5) connect checked user-copy only after that backend is proven in QEMU.

#### 2026-11-19 — Run: checked physical user-copy layer merged

- **Status:** merged. PR #78 (branch `feat/checked-user-copy-layer-2026-11-19`) merged into `main` as merge commit `37e4c18` (head `3deb489`, 16/16 checks green, 0 failures; Rust workspace tests passed).
- **Work done:** implemented `kernel/src/memory/user_copy.rs` — `PhysicalAccess` backend trait, `CopyUserError` (`Access/TooLarge/PhysOverflow/Accessor`), `MAX_COPY_BYTES = 1 MiB` cap checked before mapping lookup, and `copy_user_bytes_in/out` which validate the whole range via `AddressSpace::check_user_access` first, then translate and copy page-bounded segments. 6 host tests, including page-boundary roundtrip, validation-before-access, RW->RO cross read, backend fault propagation, fail-closed zero-length/oversize, and 4096+16 per-page segment splitting.
- **Fix iterations (all caught by CI, none shipped broken):** (1) test used `Vec` which is out of scope in the no_std kernel crate, replaced with `RefCell<[usize; 8]>` recorder; (2) a bot-driven edit briefly dropped one `#[test]` attribute, restored; (3) roundtrip test asserted the wrong second-segment index (0x101f -> 0x100f); (4) fixture RO page was mapped outside the FrameMemory window; (5) write over RO+unmapped pages reports PermissionDenied first because checks walk pages in order — test expectation corrected, production code unchanged.
- **Honest limits:** this layer is not yet wired into the live boot memory map; no CR3 switching/TLB invalidation; no Ring-3 proof; an Accessor fault after validation can leave the copy partially applied (documented, caller must treat destination as unspecified until `Ok(())`).
- **Next priorities:** wire the checked user-copy layer into boot memory-map direct-map/allocator integration; supervisor-only kernel mappings + CR3 switching + TLB invalidation in the physical table layer; VirtIO persistence follow-through; real CellNet transport.

## CI follow-up — no_std test fix (bounded recorder instead of Vec)

- The first full test run on PR #78 failed to compile the kernel lib tests with E0433/E0425: the segment-splitting
  regression test used `core::cell::RefCell<Vec<usize>>`, but the kernel crate has no allocator, so `Vec` is not
  in scope in a `#![no_std]` crate. Production code was unaffected; two identical diagnostics came from the same
  test fixture.
- Replaced the collector with a bounded `[usize; 8]` fixture that records at most the first eight segment lengths,
  keeping the assertion (4096 + 16 for a 4112-byte copy starting at 0x1000) and the no_std, allocation-free
  workspace contract. No production behavior changed.
- Fresh CI on the corrected owner-authored head is pending; the failed run is recorded as a failure, not a pass.

#### CI follow-up — formatter correction and suite retrigger

- The first PR #78 run exposed only rustfmt layout differences; no compile or test result was obtained and the
  failed format gates are not counted as code-test failures or passes. The repository rustfmt bot applied the
  formatting-only correction in commit `983a6691171a23da383e635ea94e559889e7382e`; no semantic source change.
- Bot-authored heads do not trigger Actions, so this owner-authored documentation-only commit re-runs the full
  suite on the formatted tree. Until that run completes, all runtime gates remain unverified for this head.

## 2026-11-19 (second session) — PR #77 merged; checked physical user-copy layer added

### Continuation and PR #77 outcome

- Continued from the 2026-11-19 morning entry. On the final PR #77 head `0f4d01945b99375a5cab5864e105d7760eab537d`,
  all 16 check-runs completed **success**, including the previously pending multi-architecture image job
  (build-only; still x86_64 targets only).
- PR #77 was merged into main as `415dd9800201034d1622b6f0f85b616cb9d1d73f` (merge method,
  expected-head-SHA compare against `0f4d019`). The branch is preserved; nothing was force-pushed or deleted.

### Implemented this session (branch feat/checked-user-copy-layer-2026-11-19)

- `kernel/src/memory/user_copy.rs` (new): the checked physical user-copy layer that closes the gap between
  `AddressSpace::check_user_access` and an actual kernel-side copy.
  - `PhysicalAccess` trait: a bounded physical-memory backend (`read_bytes` / `write_bytes`), page-bounded per
    call. The layer never dereferences memory itself; a backend that hands out unchecked mappings is a caller
    error, exactly as for `DirectMapWindow`.
  - `CopyUserError` (`Access`, `TooLarge`, `PhysOverflow`, `Accessor`) and the `AccessorFault` marker.
  - `MAX_COPY_BYTES` (1 MiB) hard cap checked before any mapping lookup.
  - `copy_user_bytes_in` (user -> kernel buffer, read access) and `copy_user_bytes_out` (kernel buffer -> user,
    write access): the full range is validated first, then copied one page-bounded segment at a time via checked
    address arithmetic.
- `kernel/src/memory/mod.rs`: module registered and API re-exported.
- New tests: page-boundary roundtrip (`0x1ff0`, 32 bytes over two pages), validation-before-access for write
  denial and unmapped tail pages (the backend is never called), cross-mapping read across the RW -> RO boundary,
  out-of-window `Accessor` fault propagation, zero-length rejection, oversized-copy cap, and per-page segment
  splitting of backend calls (4096 + 16 for a 4112-byte copy).

### Honest scope

- This is a checked-copy contract layer over an explicit backend. It is not wired into the live boot memory map,
  does not switch CR3, and does not enable Ring 3.
- An `Accessor` fault after validation can leave the copy partially applied; the module documentation requires
  callers to treat destination buffers as unspecified until `Ok(())`.
- VirtIO block I/O, durable storage, NIC/UDP transport and ARM64/RISC-V runtime remain untouched open gates.

### Next priorities

1. Validate this layer with the full PR CI suite; fix any Clippy/rustfmt/test findings before merge.
2. Audit the boot memory-map handoff and design a validated direct-map/allocator integration so
   `PhysicalTablePool` and this copy layer can run on real boot memory.
3. Continue with supervisor-only kernel mappings, per-process CR3 switching and TLB invalidation before any
   Ring-3 entry.
4. Keep real VirtIO block read/write persistence and two-node authenticated CellNet transport as separate
   runtime milestones.

### Final validation on the formatted CellNet head — 2026-11-13

- The final source+documentation head at this validation point was
  `6b74be812a15b83fe571d92b0b877d95164fb40e`; all nine PR workflows passed.
- The replay-window change's regression coverage passed as part of the
  workspace suites. The compatibility re-export fix also passed Clippy.
- The PR remains draft: loopback-only transport, volatile replay state,
  caller-supplied validity clock and development TOFU default are still
  explicit production blockers. No merge was attempted.

### Second rustfmt correction and final-head CI retry — 2026-11-13

- The next authored head `00644f0` reached the full PR workflow set, but
  Quality Gate, Cloud Security and Workspace CI stopped at formatting checks.
  The rustfmt bot's exact change was to move the scoped
  `#[allow(deprecated)] pub use caps::MAX_USED_GRANTS` item before the
  grouped `pub use caps::{...}` block. No semantic source change was made.
- Formatter commit: `72f095052bf3146fd74691edb88ec8d1dad038c4`.
- The failed `00644f0` formatting gates are not counted as code-test
  failures or passes. A documentation-only commit now triggers the complete
  suite on the formatter-clean head; final validation must use that new head.

### Clippy regression diagnosed and corrected — 2026-11-13

- The full rerun on `184695216` reached Clippy and found one real compile
  error: re-exporting the deprecated `MAX_USED_GRANTS` compatibility alias
  from `services/cellnetd/src/lib.rs` was denied by the repository's
  `-D warnings` gate. The replay-window implementation itself produced no
  Clippy diagnostic in that run.
- Preserved source compatibility and scoped `#[allow(deprecated)]` only to
  the legacy re-export; new callers use `MAX_REPLAY_STREAMS`. This is a
  narrow compatibility boundary, not a workspace-wide lint suppression.
- The formatter had already applied its two formatting-only changes. A new
  full CI run is required on the corrected commit; the prior Clippy failure
  is not counted as a pass.

### CI formatting correction — 2026-11-13

- The first validation attempt on source commit `6d542777` failed only at the
  workspace formatting gates in Quality Gate #1101 and Cloud Security #1046.
  Their compile/test/Clippy stages were skipped, so no Rust correctness result
  was obtained from those two jobs. The UEFI loader job passed.
- The repository's rustfmt workflow applied only the expected formatting
  changes to `services/cellnetd/src/caps.rs` (commit
  `9093aa1899887e0d4a0f864e21783065dcf21550`): wrapping the replay-window
  iterator chain and compacting the test helper signature. No semantic
  changes were made by the formatter.
- The bot-authored commit's PR workflows reported `action_required`, so a
  maintainer-authored documentation-only commit is used to re-run the full
  suite on the formatted source head. The failed formatting attempt is not
  counted as a code-test failure or a pass.

## 2026-11-13 — CellNet capability replay-window hardening

### Baseline and continuation

- Continued on `feature/cellnetd-fabric` (draft PR #68) from head
  `864b6169827c216244ce71a16abd2e8289997a76`. The previous session had
  added signed per-call capability grants, but its 64-entry table retained
  every consumed `(grantee node, sequence)` forever. After 64 calls the
  authority permanently denied further calls, even for an otherwise healthy
  caller. The table also did not distinguish independent caller cells.
- Audited the exact PR head and its GitHub Actions runs before changing code.
  All nine workflow runs completed successfully on `864b616`: Quality Gate,
  workspace CI, Cloud Security Gate, bootable-image/QEMU, VirtIO/QEMU,
  multi-architecture image build, AOSIN installer matrix, Clippy diagnostics
  and CI diagnostics. The Quality Gate Rust workspace job recorded 536
  passing test cases; Workspace Build & Validation recorded 525; Cloud
  Security Gate recorded 555. These are separate overlapping suites, not
  additive totals. UEFI loader, x86_64 QEMU ISO smoke, exception-injection,
  and QEMU VirtIO-device-presence/boot jobs passed. The VirtIO job did not
  perform block read/write or persistence.

### Implemented

- `services/cellnetd/src/caps.rs`: replaced the lifetime-growing 64-token
  replay cache with 64 fixed replay streams keyed by
  `(grantee_node, caller_cell)`. Each stream tracks a 64-sequence sliding
  bitmap, rejects duplicates and values older than the window, accepts
  unique in-window out-of-order messages, and continues indefinitely as
  sequence numbers advance without evicting still-replayable entries.
- New regression tests cover 256 sequential grants, duplicate rejection,
  bounded out-of-order acceptance, stale-sequence rejection, independent
  caller-cell streams, and full stream-table behavior (existing streams
  continue while new streams fail closed).
- `services/cellnetd/src/lib.rs`: exports `MAX_REPLAY_STREAMS`; retains
  `MAX_USED_GRANTS` as a deprecated compatibility alias and clarifies the
  table-full error.
- `docs/CELLNET_ARCHITECTURE.md`: documents sliding-window semantics and
  explicitly records that replay state is volatile and resets on
  `cellnetd` restart; cross-restart protection requires durable state or a
  fresh unpredictable authority epoch from secure entropy.

### Validation

- Final formatted PR head `6b74be812a15b83fe571d92b0b877d95164fb40e`:
  **9/9 pull-request workflow runs succeeded** (Quality Gate, Workspace CI,
  Cloud Security Gate, bootable-image/QEMU, VirtIO/QEMU, multi-architecture
  image build, AOSIN installer matrix, Clippy diagnostics and CI diagnostics).
- Quality Gate: **540 passed / 0 failed** across 30 test binaries.
  Workspace Build & Validation: **529 passed / 0 failed** across 29 binaries.
  Cloud Security Gate: **559 passed / 0 failed** across 36 binaries. These
  suites overlap and must not be added together.
- Formatting, workspace compilation, Clippy and UEFI loader checks passed.
  QEMU bootable ISO smoke reached `AWEOS: kernel is alive`; exception
  injection self-test passed (1/1). VirtIO contract tests passed and QEMU
  detected `virtio-blk-pci` with the kernel booting while attached.
- AOSIN installer matrix passed on Ubuntu and Windows (4/4 jobs). The
  multi-architecture workflow passed but built x86_64 targets only; ARM64
  and RISC-V are not validated.
- No local Cargo/Rust toolchain was available in this automation
  environment; all validation evidence above comes from GitHub Actions.
  The VirtIO run still does not perform real block read/write or persistence.

### Remaining limitations

- No persistent replay state or authority-epoch binding across restart.
- The CellNet `Transport` is still loopback-tested only; no NIC/UDP wiring
  or two-instance network mesh test.
- `TrustPolicy::Open` remains the development default; production must
  configure pinned peer keys.
- No kernel changes, Ring-3 IPC, or userspace service supervision were made.

### Next priorities

1. Validate this replay-window change with the full PR CI suite and fix any
   failures before changing PR readiness.
2. Implement per-process page-table roots and checked user-copy before Ring 3.
3. Add a live timer-driven dispatch loop to the unified ProcessManager.
4. Implement real VirtIO block request/completion and QEMU read-after-write
   persistence.
5. Wire netd UDP TX/RX into QEMU, then run a two-instance CellNet mesh.

---


---

## 2026-11-12 — CellNet capability-token enforcement (ABI 0.2)

### Continuation point

- Continued from the CellNet trust-boundary hardening session on branch `feature/cellnetd-fabric` (PR #68, draft). That session's recorded blocker was: "Per-caller capability authorization is not implemented inside cellnetd; `required_caps` is metadata until a trusted capability token is supplied by the service boundary." This session closes exactly that blocker.
- No force-push, no branch deletion, no main-branch changes; external draft PR #64 remains untouched.

### Implemented (commits on `feature/cellnetd-fabric`)

- `services/cellnetd/src/caps.rs` (new, `d61c1a8`): `CapabilityGrant` — a 49-byte signed payload (version, grantee node, cell id, capability mask, validity window, sequence) plus a 64-byte Ed25519 signature, 113 bytes on the wire; `CapabilityAuthority` with a bounded, fail-closed 64-slot single-use replay table. Signing/verification reuse `awe-securityd`'s Ed25519; no new cryptography.
- `services/cellnetd/src/wire.rs` (`ea9689a`): `MAX_ENVELOPE_PAYLOAD` 576 → 704 with a compile-time assertion, so a CellIpc frame fits the 24-byte header + 113-byte token + 512-byte body budget.
- `services/cellnetd/src/lib.rs` (`a76f748`): caps module registered and re-exported; four new `CellNetError` variants (`CapabilityDenied`, `CapabilityExpired`, `CapabilityReplayRejected`, `CapabilityTableFull`); crate ABI 0.1 → 0.2.
- `services/cellnetd/src/fabric.rs` (`2dc502e`): CellIpc payload is now `24 B header ‖ 113 B token ‖ body`; `FabricConfig::capability_enforced()` (`require_ipc_caps = true`) and `capability_enforced()`; `set_capability_authority()` pins the issuer key; `send_cell_ipc_authorized()` verifies the grant (node, cell, caps ⊇ destination `required_caps`, validity window) and spends its sequence before sealing; the grant-free `send_cell_ipc` returns `CapabilityDenied` in enforced mode; the receiver independently re-verifies the token against its own authority pin, the destination's published `required_caps` and its replay table, dropping forged/expired/replayed tokens and counting `ipc_dropped`. Development mode keeps the old grant-free behaviour for compatibility.
- `services/cellnetd/src/fabric/tests.rs` (`0b233e1`): the unpublished-source raw-frame test now seals a full header-plus-token payload; seven new two-node loopback tests — authorized end-to-end delivery, fail-closed refusal of the grant-free API, insufficient caps, expired grant, single-use sequence replay, foreign-issuer signature failure, receiver-side drop of a forged all-zero token.
- `docs/CELLNET_ARCHITECTURE.md` (`ac77553`): ABI 0.2; rule 5 rewritten (capability-gated access); CellIpc wire row updated to ≤649 B payload; new "Capability authorization" section; integration contract now shows `send_cell_ipc_authorized` and lists delegation/attenuation as deliberate future work.

### Honest limits

- The remaining CellNet blocker from the previous session is unchanged: the `Transport` trait is still loopback-only; no NIC/UDP wiring and no two-instance mesh test exist yet.
- Grants are minted directly by the pinned authority; delegation and attenuation chains are not implemented.
- No kernel-crate changes; the trusted computing base is untouched.

### Validation status

- No local toolchain is available in this environment; validation is deferred to the GitHub Actions suite on this PR head. **No green result is claimed yet.** If CI fails, the diagnostics-bot comment will be read and the defect fixed before any merge decision; PR #68 stays a draft until a full green suite (including the QEMU jobs) exists on the final head.

### Next priorities

1. Watch the fresh CI suite on this head; fix any clippy/rustfmt/test failures from the diagnostics output.
2. On a full green 16/16 suite: decide draft vs ready for PR #68. The previous session held it draft pending "tokens + real transport"; tokens now exist, the real transport does not — record the decision explicitly either way, and merge only with an expected-head-SHA compare-and-swap.
3. Then continue the audit queue: per-process page-table roots + checked user copy, timer-driven dispatch, VirtIO block read-after-write in QEMU, netd UDP wiring and the two-instance CellNet mesh.

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

### Validation — code head `6c280ff4c67c27dab634f002cd1c5f4fcac965fa`

- Workspace tests: **521 passed, 0 failed** (all test binaries in the AWEOS CI workspace job).
- Workspace compilation, formatting, Clippy, UEFI loader check, and security gate: **passed**.
- QEMU exception-injection self-test: **passed**.
- Bootable-image smoke test: x86_64 Multiboot2 ISO booted in QEMU and reached the kernel-alive marker.
- VirtIO runtime check: QEMU exposed a `virtio-blk-pci` device and the kernel booted with it attached; storage/driver contract tests passed. **No actual VirtIO block read/write or persistence test is claimed.**
- Multi-architecture workflow: x86_64 bare-metal kernel and x86_64 UEFI loader artifacts built. **ARM64 and RISC-V were not built or runtime-tested by this workflow.**
- Cross-platform AOSIN CLI/GUI installer jobs: Ubuntu and Windows jobs passed.
- All nine substantive CI workflows on this code head completed successfully. The rustfmt bot's intermediate commit had `action_required` runs, so validation was re-triggered on a maintainer-authored documentation commit after formatting.
- This log update is documentation-only; the validated Rust source tree is unchanged from code head `6c280ff`.

### Remaining blockers and next priorities

- Per-caller capability authorization is not implemented inside cellnetd. `required_caps` is explicitly documented as metadata until a trusted capability token is supplied by the service boundary.
- The fabric still uses a loopback-only Transport; no NIC/UDP integration or two-instance network test exists.
- No real VirtIO block request/completion, read-after-write persistence, or power-loss recovery evidence exists.
- No per-process page-table roots or checked user-copy path; Ring 3 remains deferred.
- Next order: (1) per-process address-space roots + checked user copy, (2) timer-driven dispatch, (3) actual VirtIO block I/O and persistence in QEMU, (4) netd UDP wiring and two-instance CellNet mesh.
- Main remains unchanged. PR #68 is open and draft; external draft PR #64 remains untouched.

## 2026-11-13 — PR #68 (CellNet fabric) rebased, re-validated and merged into main

### Baseline and continuation

- Started from main head `64def465` (PR #69, docs-only merge of the advanced cell runtime architecture spec). The previous session's recorded next-priority item 1 was "Validate this replay-window change with the full PR CI suite and fix any failures before changing PR readiness."
- Audited open PRs first. PR #68 (CellNet distributed-cell fabric, directive §V) was draft with head `a4fdbf39` behind main; PR #64 (external bot draft, initd restart policies) still conflicts with main.

### What was done

- Updated PR #68's branch onto current main via the GitHub branch-update API (new head `7a34fcd30dd238cc55624b8a07d781e35f634255`), which re-triggered the complete workflow suite on the rebased head.
- Reviewed the merged content before the merge decision: `awe-securityd` crypto wrappers (X25519/HKDF/ChaCha20-Poly1305/Ed25519, RFC-vector-tested), `awe-cellnetd` fabric (signed announcements, 4-flight mutually authenticated handshake, AEAD envelopes with anti-replay window, per-call capability grants with sliding replay streams, publish policy, heartbeat/backoff/partition recovery), all fixed-capacity, no_std, transport-neutral.
- On the rebased head: **16/16 check-runs completed success** (cloud-security, cloud-release-contract, Rust workspace, UEFI loader, Workspace Build & Validation, Workspace clippy, cargo failure diagnostics, exception-injection-selftest, build-and-smoke-test-image, virtio-block-contract-and-qemu, Build Multi-Arch Boot Images, 4x Build AOSIN Installers, rustfmt bot).
- Marked PR #68 ready for review and merged into main as **df13d7a5c3c9f493b0f5d995878e24dd8b596951** (merge method, expected-head-SHA compare-and-swap against `7a34fcd`). Standard caveat: token merges do not trigger Actions on main itself; the green evidence is the PR head run on the rebased head, whose only delta from the merged tree is the merge commit itself.
- PR #64: re-confirmed merge conflict against current main (`update_pull_request_branch` fails with a merge conflict). It is an external draft not authored by this automation; it was left untouched, unmerged, and nothing was force-pushed or deleted.

### Honest scope

- The CellNet fabric is a contract/protocol layer: Transport is still loopback-tested only, no NIC/UDP wiring, no two-instance mesh, replay state is volatile, TrustPolicy::Open is the development default. No kernel-crate changes were made in this session.
- No Ring 3, per-process page tables, VirtIO block I/O or persistence evidence exists; those remain the priority queue.

### Files changed in this session

- main via PR #68 merge: services/cellnetd/* (new crate), services/securityd/src/crypto.rs + Cargo.toml, Cargo.toml, Cargo.lock, docs/CELLNET_ARCHITECTURE.md, docs/ARCHITECTURE_AUDIT_2026-11-12.md, docs/AI-DEVELOPMENT-LOG.md.
- docs/AI-DEVELOPMENT-LOG.md — this entry.

### Next priorities (unchanged order)

1. Per-process page-table roots + checked user-copy before any Ring-3 entry.
2. Timer-driven dispatch loop on the unified ProcessManager.
3. Real VirtIO block request/completion with read-after-write persistence in QEMU.
4. netd UDP path in QEMU, then bind cellnetd to it and run a two-instance mesh.
5. Re-evaluate external draft PR #64 (still conflicting; must not be merged as-is).


## 2026-11-14 — Initd restart-policy enforcement and ramdisk stack-safety repair

### Baseline audit

- Current main at audit start: `36f076782707bff10fbd972918709f472c69cb60`; 44 branches and one open PR at that point.
- Rechecked external draft PR #64 (`fix/initd-restart-policy-enforcement-2026-11-10`): it is still dirty/non-mergeable and diverged from main by 4 commits ahead / 58 behind (merge base `dc898bf66af0f906ac42de9dcbc8b8eae81b8bfc`). It was left untouched; no force-push or branch deletion.
- The initd bug was still present on main: `ServiceTable::restart()` enforced policy, but callers could bypass it by calling `set_state(Failed|Stopped, Starting)` directly.
- Main's prior CI evidence was rechecked from real job logs: 347 kernel tests passed; the x86_64 ISO reached the kernel-alive marker; exception injection passed. The VirtIO job only confirmed QEMU's `virtio-blk-pci` exposure and kernel boot, not block I/O.

### Changes on PR #70

Branch: `fix/initd-restart-policy-main-2026-11-14`

Final code head before this log-only commit: `0b39d7620a107168e93962a0ed165e1be58f1c4e`

- `services/initd/src/lib.rs`: `ServiceTable::set_state` now checks the registered service's restart policy whenever a non-Declared state attempts to enter `Starting`. Initial `Declared -> Starting` remains valid; `Never`, `OnFailure`, `Always`, and quarantined-state behavior remain explicit. Added three regression tests covering policy bypass and allowed/denied restart transitions.
- `kernel/src/storage/ramdisk.rs`: replaced the 256 KiB inline array in `RamBlockDevice` with an exact-size caller-owned mutable backing buffer. Construction zeroes that buffer, remains allocation-free, and no longer returns a huge value by stack. This is a test/in-memory device contract, not durable storage.
- Updated `kernel/src/storage/mod.rs`, `kernel/src/storage/vfs.rs`, and `tests/product-core/src/lib.rs` to provide backing buffers explicitly.
- The first PR CI attempt exposed a real Clippy `large_unopt_stack_frames` error in the old ramdisk constructor (estimated 524,292-byte frame). The borrowed-buffer change fixed that finding. The next run caught only rustfmt layout differences; those were corrected and the final code head passed formatting.

### Validation on final code head `0b39d762`

- AWEOS CI Pipeline run **43612951043**: success. Its workspace log contains **532 passed, 0 failed across 57 test binaries**. The new initd suite reported **14 passed**, including all three added regression tests; the kernel suite reported **347 passed**; product-core reported **11 passed**.
- AWEOS Quality Gate run **43612951076**: success (workspace compile/tests/format/Clippy and UEFI loader jobs).
- Clippy diagnostics run **43612951055**: success.
- Bootable Image Pipeline run **43612951202**: success. QEMU ISO smoke test reached `AWEOS: kernel is alive`; exception-injection self-test **1 passed**.
- VirtIO Runtime run **43612951058**: success. QEMU exposed `virtio-blk-pci`, and the kernel booted with it attached. **No VirtIO block read/write or persistence claim is made.**
- Multi-Architecture Images run **43612951048**: success, but the workflow built only `x86_64-unknown-none` and `x86_64-unknown-uefi`. ARM64 and RISC-V were not built or runtime-tested.
- AOSIN Cross-Platform Installers run **43612951046**: success, all 4 Ubuntu/Windows CLI/GUI jobs.
- CI Diagnostics run **43612951095**: success.
- rustfmt automation run **43612946838**: success; it made no additional source commit.
- The 14 PR check-runs visible for this code head completed success. AWEOS Cloud Security Gate run **43612951062** is still pending without an assigned job/step log; an older run on an intermediate head remains queued/in progress. Therefore the cloud-security workflow is **not** counted as passed and PR #70 remains a draft.

### Honest scope and remaining limits

- PR #70 is open, unmerged, and mergeable; main was not modified. The stale external PR #64 remains open and untouched.
- The ramdisk is still volatile test memory. It is not a native filesystem, durable storage, journal recovery, or power-loss recovery implementation.
- The QEMU VirtIO evidence remains device enumeration + kernel boot only, not request submission/completion or read-after-write.
- Ring 3 remains deferred. No per-process page-table roots, active address-space switch, or checked user-copy path was added in this session.

### Next priorities

1. **P0: per-process address-space roots + checked user-copy before any Ring-3 entry.** The current audit found that `PageMapper` owns only one table level; `PageTable` is a 512-entry table primitive; x86_64 bootstrap uses static PML4/PDPT/PD tables with supervisor-only 2 MiB identity leaves for the first 1 GiB; `ProcessContext` carries a CR3 field, but `x86_64_switch.S` does not load CR3. `enter_userspace` is deliberately unused and the general syscall dispatcher fails closed. Implement ownership/lifetime for page-table frames, preserve supervisor-only kernel mappings, enforce USER/NX/W^X policy at every level, add TLB-safe map/unmap, and make user copies validate overflow, permissions, and page crossings before enabling Ring 3.
2. Wire timer-driven dispatch to the unified ProcessManager only after CR3 switching and per-CPU current-task ownership are explicit.
3. Implement actual VirtIO block request/completion and QEMU read-after-write persistence; do not treat PCI detection as storage I/O.
4. Connect netd to a real UDP/NIC path, then run two CellNet instances over that transport.
5. Re-evaluate PR #64 only after its owner rebases; do not merge its divergent branch as-is.


## 2026-11-15 — Verified PR #70 merged

- PR #70 was merged into main as `eef712f1d9fbf2119dcd78dbc6fa7e3bb6f573c9` after all nine substantive GitHub workflows on head `af210e33486c512fa3540dd36ac6cc474118cbb3` completed successfully.
- The merged changes include initd restart-policy enforcement and the caller-owned, stack-safe RAM-disk test device.
- This corrects the earlier historical note in the PR #70 section that said it remained open; that statement was accurate at the time of that entry but is no longer the current repository state.

## 2026-11-15 — PR #72 CI repair: scheduler-aware QEMU smoke test and bounded ramdisk backing

### CI findings on initial PR #72 head

- Workspace CI, installer matrix, diagnostics, and x86_64/multi-architecture image build completed successfully.
- Three independent gates failed for actionable reasons:
  - QEMU product-core smoke test still expected the removed `timer programmed (no scheduler yet)` marker, so it stopped at 8/17 milestones even though the serial log showed the new scheduler booting and running.
  - The smoke test still asserted that the boot log must not claim preemptive scheduling, which became stale once the ring-0 scheduler was implemented.
  - Clippy (including the exception-injection and security gates) rejected the 256 KiB inline ramdisk value for a 524,292-byte potential stack frame.

### Changes on `feat/preemptive-scheduler`

- Reused the allocation-free caller-owned ramdisk backing design: `RamBlockDevice` now borrows an exact-size `RAMDISK_BYTES` buffer, avoiding the large by-value stack frame.
- Updated storage module/VFS/product-core test callers to provide the backing buffer.
- Updated the QEMU boot milestone to require the real `preemptive scheduler armed` marker.
- Extended the QEMU settling interval to 3 seconds and now require heartbeat + scheduler diagnostics with a non-zero reported switch count. Ring 3 remains explicitly deferred; the test no longer treats the implemented ring-0 scheduler as an unimplemented feature.

### Validation status

- Fixes are committed to the PR branch; the new CI run is required before this work is considered validated.
- No merge to `main`, force-push, or branch deletion was performed.

### Next priorities

1. Confirm the complete CI suite is green on the updated PR #72 head; inspect QEMU serial evidence for heartbeat and non-zero switch count.
2. Review and harden PR #71's address-space contract (mapping-slot reuse, null virtual mappings, and physical-range overflow) before any hardware page-table activation.
3. Keep Ring 3 deferred until page-table ownership, CR3 switching, USER/NX/W^X policy, TLB invalidation, and real checked-copy operations are wired and tested.


### Follow-up CI correction — scheduler test spin loops

- The initial PR run exposed two Clippy errors in test-only dummy thread entry points: empty infinite loops were rejected under `-D warnings`.
- Replaced both empty bodies with `core::hint::spin_loop()`. This preserves the non-returning dummy-thread behavior while avoiding a hot empty loop warning.
- The earlier `RamBlockDevice::default()` diagnostic came from an intermediate PR push; the current branch tree now uses caller-owned backing storage in the product-core test.
- A fresh full CI run on the resulting branch head is required; the earlier failed checks are not counted as validation of this final head.


### Scheduler regression follow-up

- Added a regression test proving that a zero-tick quantum is clamped to one tick before preemption is armed.
- This is a bounded scheduler configuration invariant; it does not claim blocking sleep, SMP, user-mode execution, or FPU/SIMD context switching.
- Full CI is triggered for the updated, main-synchronized PR #72 tree.


## 2026-11-15 — PR #72 final validation

Validated code head: `b99ca39a5d6a2f18e5d9b0e2aa3e95a7409c21e9` (main-synchronized; zero commits behind main).

### Real CI results

- AWEOS CI Pipeline run **43732931241**: success; workspace test log reports **541 passed, 0 failed across 57 test binaries**. Kernel: **356 passed**; initd: **14 passed**; product-core: **11 passed**.
- AWEOS Quality Gate run **43732931167**: success (Rust workspace and UEFI loader).
- Clippy report run **43732931222**: success.
- Bootable Image Pipeline run **43732931184**: success; exception-injection self-test passed.
- VirtIO Runtime run **43732931177**: success. QEMU exposed `virtio-blk-pci`, booted the kernel, and emitted:
  - scheduler armed with 3 kernel threads and a 50 ms quantum;
  - heartbeat at ticks 50, 1100, and 2150;
  - scheduler diagnostics `switches=44`, dispatch counts `15/15/15`.
  This validates the ring-0 timer-driven context-switch path in QEMU. It does **not** validate VirtIO block request/completion, read-after-write, or durable storage.
- Multi-Architecture Images run **43732931201**: success, but actual targets were only `x86_64-unknown-none` and `x86_64-unknown-uefi`. ARM64 and RISC-V were not built or tested.
- Cloud Security Gate run **43732931202**: success.
- Cross-platform installer run **43732931162**: success, all four Ubuntu/Windows CLI/GUI jobs.
- CI Diagnostics run **43732931205**: success.

All **9/9 substantive workflows** passed on the validated code head.

### Remaining limitations

- The scheduler is single-CPU and kernel-thread-only; it does not schedule user processes or switch CR3.
- `sleep_ticks` is a busy wait, not a blocked sleep/wakeup mechanism.
- FPU/SIMD register context is not saved/restored; current kernel threads must not depend on it.
- No SMP, per-CPU run queues, user-mode isolation, or Ring 3 support is claimed.
- The VirtIO workflow still proves device exposure + boot, not actual block I/O or persistence.
- The multi-architecture workflow currently builds x86_64 only.

### Next priorities

1. Complete the real page-table allocator/ownership and mapping hierarchy behind PR #71; add TLB-safe map/unmap and preserve supervisor-only kernel mappings.
2. Wire CR3 switching into process context only after address-space isolation and user-copy tests pass.
3. Add blocking thread states and timer wakeups; preserve the busy-wait implementation only as an explicitly named temporary delay.
4. Add FPU/SIMD save/restore (or an enforced kernel soft-float policy) before allowing general kernel-thread code to use those registers.
5. Implement VirtIO block request/completion and QEMU read-after-write tests; keep persistence claims deferred until crash/power-loss tests exist.
6. Add actual ARM64 and RISC-V builds before describing the workflow as multi-architecture support.


## 2026-11-15 — PR #71 address-space contract hardening

### Review findings and fixes

- Fixed a mapping-table corruption bug: after removing a middle mapping, the old insertion logic used the live-entry count as an array index and could overwrite a different live mapping. Registration now finds the first vacant slot; a regression test verifies that existing mappings survive middle-slot removal and reuse.
- Reject null virtual mappings at registration, not only at later access validation.
- Validate the complete physical range against the 52-bit x86_64 page-table address encoding, including overflow at the exclusive end; the highest representable single page remains valid.
- Make UserMapping::virtual_end checked so externally constructed invalid descriptors fail closed, and make page translation use checked addition.
- Clarify that AddressSpace records a caller-owned root-frame address; it does not allocate, own, free, install page tables, or activate CR3.

### Validation status

- Regression tests added for middle-slot reuse, null mapping rejection, and physical-range overflow.
- The updated PR head must pass the full GitHub Actions suite before merge. No hardware page-table activation, physical user-copy, CR3 switch, or Ring 3 is claimed.

### Next priorities

1. Wire page-table frame allocation/lifetime and real PML4/PDPT/PD/PT installation while preserving supervisor-only kernel mappings.
2. Add TLB-safe map/unmap and enforce USER/NX/W^X policy at all page-table levels.
3. Integrate checked physical user-copy with process contexts only after address-space isolation tests pass.
4. Keep Ring 3 disabled until the above hardware and QEMU isolation tests are green.


### Follow-up main synchronization after PR #72

- PR #72 (preemptive ring-0 kernel-thread scheduler) was merged into main as `20a3a24d90d7641b1c5d1f49e9b0f70966c2d50a` after its nine substantive workflows passed on the validated code head.
- The PR #71 branch was synchronized with that main commit through a normal two-parent merge commit `1b8b4ce270334f12eb91c1142d0c5291a046714e`; compare reports zero commits behind main.
- PR #71 remains open and mergeable, but its latest head has no completed CI run. The prior workflow attempt was `action_required`; no test success is claimed and no merge was performed.


## 2026-11-15 — PR #71 CI root cause fixed and real page-table construction added

### CI root cause diagnosis and fix

- The first completed CI run on this branch (head c69a905) failed seven
  jobs (Rust workspace, cloud-security, virtio-block-contract-and-qemu,
  exception-injection-selftest, build-and-smoke-test-image, multi-arch,
  AOSIN installers) with exit code 101. The repo's CI diagnostics comments
  named one root cause: E0277, `&PageFlags` does not implement `Debug`,
  raised by the `#[derive(Debug)]` on `UserMapping`
  (kernel/src/memory/address_space.rs:65). `PageFlags` and
  `PageTableEntry` in kernel/src/memory/paging.rs did not derive `Debug`.
- Fix (commit 52b20d8): both types now derive `Debug`. No semantic change.
  This was a compile error, so no test result from that run is counted as a
  pass or failure of the address-space logic.

### Real page-table construction (new module)

- New kernel/src/memory/page_table_build.rs (commit a51345f, registered in
  memory/mod.rs): `UserPageTableBuilder` installs real four-level
  PML4 -> PDPT -> PD -> PT entries for every registered user mapping of an
  `AddressSpace`, through a caller-supplied `TableMemory` backend
  (alloc_table / table / table_mut). No identity map is assumed; nothing
  touches CR3.
- Security properties: intermediate entries carry
  PRESENT|WRITABLE|USER so leaf permissions are the only boundary; the
  kernel half of the PML4 (indexes 256..512) is never written and a
  mapping that would land there fails closed with
  `KernelHalfForbidden`; frame exhaustion fails closed with
  `OutOfFrames` and all allocations are recorded in a bounded tracker
  for later reclamation.
- `AddressSpace::mappings()` iterator added so the builder (and future
  CR3-switch code) can walk live mappings.
- A hardware-style four-level `translate` walk allows verifying built
  tables without dereferencing frames as data.
- Seven new host unit tests: walk-equivalence with the contract layer,
  kernel-half preservation, intermediate flag policy, exact leaf flags
  (R/RW/NX preserved), fail-closed frame exhaustion, kernel-half and
  invalid-mapping rejection at install time.

### Explicit non-goals (still unimplemented)

- Frame teardown / reference counting, TLB invalidation, CR3 switching,
  the physical checked user-copy path, Ring 3 entry, and W^X/NX policy
  enforcement at all page-table levels.

### Validation status

- No local Rust toolchain is available in this run's environment; the
  branch has not been compiled or tested locally. The new head must pass
  the full GitHub Actions suite before any merge. No green result is
  claimed yet.

### Next priorities

1. Inspect the CI run on this head; fix any compile or clippy findings.
2. Wire `TableMemory` to the kernel's physical frame allocator and
   mapper so the builder runs on real boot memory.
3. Enforce W^X/NX policy at all page-table levels; then CR3 switching and
   the checked physical user-copy path, then QEMU isolation tests before
   any Ring 3 work.


## 2026-11-16 — PR #71 merged: x86_64 address-space contract and page-table builder

### Baseline and scope

- Continued from main head `20a3a24d90d7641b1c5d1f49e9b0f70966c2d50a` (PR #72, preemptive ring-0 kernel-thread scheduler).
- PR #71 was the existing main-synchronized branch `feat/address-space-user-copy-2026-11-14`; it was kept separate until the full CI suite passed.
- Final validated code head: `fe8db1d599e478cf6e530cfb013eb57b512092a2`.
- PR #71 merged into main as `991473ee1bd41d7578b5de9df66666a4f19a2730`. No force push or branch deletion was used.

### Implemented

- `kernel/src/memory/address_space.rs`:
  - Bounded per-process address-space contract with caller-owned, validated PML4 root address.
  - User mapping registration, lower canonical user-half validation, overlap/range/52-bit physical-address checks, exact unmap, safe vacant-slot reuse, page translation and checked user-access validation.
- `kernel/src/memory/page_table_build.rs`:
  - Four-level PML4 → PDPT → PD → PT construction for 4 KiB user mappings through the `TableMemory` backend contract.
  - Kernel-half PML4 indexes 256..511 are never written.
  - Intermediate USER/WRITABLE checks; rejection of huge-page intermediates, invalid/duplicate table frames, existing leaf overwrites, invalid ranges, GLOBAL/HUGE user-leaf flags and writable+executable user leaves.
  - Executable leaves are rejected beneath NX intermediate entries; writable leaves must carry NX.
  - Full mapping preflight before writing and tracker-capacity validation before requesting another frame.
  - `leaf_entry` exposes the actual leaf PTE for permission tests; `translate` returns the mapped physical page base.
- `kernel/src/memory/mod.rs` exports the new address-space and page-table builder APIs.
- `kernel/src/memory/paging.rs` provides Debug support required by the new contract/test diagnostics.
- `docs/CELL_RUNTIME.md` now documents the builder and its explicit limitations (commit `526b2bed928e931df47d63cfa5353d8b8268d8fa`).

### CI findings and corrections

- The first full compile attempt caught E0502 in the pre-existing leaf-permission test: it tried to borrow the fake memory backend while the builder still held its mutable borrow, and it incorrectly treated a mapped data frame as a page-table frame. Added `leaf_entry` and rewrote the test to inspect the returned PTE.
- The first executable test run caught an incorrect expected range: `0x1400..0x1c00` is entirely inside the mapped `0x1000` page, not an unmapped cross-page range. Corrected the test to `0x2c00..0x3400`; no user-access error precedence change was made.
- Fixed the out-of-frames fake backend so it truly permits only two intermediate allocations; it now verifies deterministic `OutOfFrames` behavior.
- Added a tracker-capacity check before allocating a frame, preventing an untracked allocation when the bounded table-frame list is full.
- Rustfmt automation completed successfully on the final code head.

### Validation — final code head `fe8db1d`

- AWEOS CI Pipeline run **43905385673**: success; **564 passed, 0 failed across 57 test binaries** (kernel: **379 passed**).
- AWEOS Quality Gate **43905385639**: success.
- AWEOS Cloud Security Gate **43905385660**: success.
- CI Diagnostics **43905385559**: success.
- Clippy report **43905385546**: success.
- AOSIN Cross-Platform Installers **43905385527**: success.
- AWEOS Bootable Image Pipeline **43905385461**: success; boot image and exception-injection self-test passed.
- AWEOS VirtIO Runtime **43905385408**: success; QEMU exposed the VirtIO PCI device, booted the kernel to RUNNING, and showed the existing 3-thread scheduler heartbeat/diagnostics (`switches=44`). **This is not block request/completion, read-after-write, or durable-storage evidence.**
- AWEOS Multi-Architecture Images **43905385519**: success, but the workflow currently builds only `x86_64-unknown-none` and `x86_64-unknown-uefi`; ARM64 and RISC-V remain unbuilt and untested.
- All **9/9 substantive workflows** passed on the final PR head.

### Honest limitations

- `TableMemory` has only the in-memory test backend; it is not wired to the live physical-frame allocator or a production physical-to-virtual table mapping.
- The builder constructs an unpublished page-table tree only. It does not switch CR3, invalidate/shoot down TLB entries, tear down/refcount frames, or perform a physical checked user copy.
- No Ring 3 entry or per-process memory-isolation runtime is enabled. Kernel-half mappings must be supplied by a future address-space owner; this builder deliberately does not clone or modify them.
- The existing scheduler remains single-core and kernel-thread-only.
- VirtIO storage I/O/persistence, real NIC/UDP integration, and ARM64/RISC-V ports remain incomplete.

### Next priorities

1. Implement a real `TableMemory` backend over owned physical frames and a verified kernel mapping; define frame lifetime, rollback and teardown.
2. Preserve a supervisor-only kernel mapping in every process root, then implement safe CR3 switching and TLB invalidation.
3. Implement physical checked user-copy and QEMU tests proving user/kernel isolation before enabling Ring 3.
4. Implement real VirtIO block request/completion, read-after-write and persistence/recovery tests.
5. Connect netd/CellNet to a real NIC/UDP transport and test two independent nodes.
6. Add actual ARM64 and RISC-V builds and runtime evidence before claiming those ports.


### Documentation synchronization (2026-11-16)

- `docs/CELL_RUNTIME.md` documents the four-level builder and its non-goals (commit `526b2bed928e931df47d63cfa5353d8b8268d8fa`).
- `PRODUCT_READINESS.md` records the per-process VM milestone and explicitly leaves memory-isolation/userspace gates unchecked (commit `69461219d811423b874eab47e28d14b30d4a6fd4`).
- `PROJECT_STATUS.md` was refreshed with the QEMU/test evidence and the next concrete VM, storage, networking and architecture-port milestones (commit `9884ffd301cc0df60f5a5cc6f8f47aaa1a7bcf2a`).
- These documentation-only commits do not change kernel code; the validated code evidence above remains the final PR head `fe8db1d`.


### Frame ownership, rollback and teardown for the page-table builder (2026-11-16)

- `kernel/src/memory/table_frames.rs` adds the ownership layer the builder
  left to its caller: a `TableFramePool` trait, an `OwnedTableFrames`
  `TableMemory` wrapper that records every allocated frame including the PML4
  root, batch `rollback`, full `teardown`, drop-time best-effort release, a
  `take_ownership` transfer (`BuiltTables`) and `build_address_space`, which
  releases every frame on any build failure before returning the error.
- Explicit non-goal preserved: no code path yet dereferences real physical
  frames. A kernel-side `TableFramePool` over the live frame allocator plus a
  verified physical-to-virtual mapping remains required before boot-path use;
  `build_address_space` documents that restriction.
- Ten new unit tests cover successful transfer, rollback on frame exhaustion
  (including the root), kernel-half rejection before allocation, batch-scoped
  rollback, double-release surfacing with continued teardown, drop safety,
  double-root rejection, leaf flag preservation, a manual four-level walk and
  contract/hardware translation parity.
- Validation pending: CI on this branch (fmt, Clippy, workspace tests). No
  merge claim is made before the PR workflows pass.


### PR #73 compile-failure correction — 2026-11-17

- Reviewed the actual PR checks and diagnostics for the frame-ownership change.
  The first source head failed workspace compilation with Rust borrow-checker
  errors (E0502 / E0499) confined to the new table_frames.rs tests:
  tests attempted to inspect or mutate the pool while OwnedTableFrames
  still held its exclusive mutable borrow.
- Corrected the tests on the existing review branch
  feat/frame-backed-table-memory-2026-11-16:
  - explicitly end builder/pool-wrapper lifetimes before inspecting the pool;
  - drop OwnedTableFrames after explicit teardown before checking pool
    state;
  - replace the impossible direct foreign-release-while-borrowed test
    setup with a DoubleReleasePool test adapter that models a stale-owner
    release inside the pool contract, reports
    FrameReleaseError::DoubleRelease, and still verifies teardown
    continues to release the root frame.
- Code correction commit: 877147733cdaf9d8e8061c8ca809825395a2f85c.
- Validation: pending fresh GitHub Actions on the corrected head. The earlier
  failed compile is not counted as a pass; no merge or readiness promotion
  is claimed. The PR remains draft until the full check set is green.


### PR #73 follow-up borrow-scope correction — 2026-11-17

- The first test-lifetime correction moved one builder drop after an
  assertion that still borrowed the wrapper. The next GitHub Actions run
  caught that remaining E0502 at the test's owned-frame-count assertion.
- Reworked the affected tests to use lexical scopes for
  UserPageTableBuilder, so its mutable borrow ends before the wrapper or
  pool is inspected. Removed an unused PageTableEntry test import.
- Follow-up code commit: eed650da3a2d8d45e9681065c4623cc1cc86b923.
- Validation is pending on the resulting final head. Earlier failed runs are
  not counted as passes; PR #73 remains draft and unmerged.


### PR #73 test-fixture corrections — 2026-11-17

- After the borrow-scope fix, the workspace test run compiled and executed
  the new tests, then exposed two incorrect expectations:
  - the four-level tree for the tested mapping is four frames total
    (PML4 root, PDPT, PD, PT); the PML4 entry is stored in the root and is
    not a separate frame;
  - USER_HALF_END is a non-canonical boundary, so it correctly produces
    InvalidMapping. The kernel-half rejection test now uses the canonical
    high-half address 0xffff800000000000 and expects KernelHalfForbidden.
- Corrected both fixtures in commit
  261b4a34d55203597d8e952a19baa3b33bdaa82c.
- The previous run reached 387 passing kernel tests but had these two
  assertion failures. A fresh full validation is pending on the new head;
  no pass or merge is claimed yet.


### PR #73 Clippy correction — 2026-11-17

- The new page-table tests progressed past the previous assertion failures.
  Clippy then rejected one manual 0..3 loop that indexed the four-level
  page-table index array (needless_range_loop, promoted by -D warnings).
- Replaced it with iteration over the first three indices, preserving the
  explicit walk of the three intermediate levels.
- Code commit: 191e6f041772f9524f5818f358daa4605ed404d4.
- Fresh CI is pending; no merge/readiness claim is made until workspace tests,
  Clippy, QEMU boot, VirtIO and security checks all pass.


### PR #73 validation completed — 2026-11-17

- Final validated code head: b5712b7a94a32b8f357cfab5ed98666527eada00.
- All 16 reported PR check-runs completed successfully: workspace formatting,
  check, tests and Clippy; UEFI loader; bootable-image build/smoke; exception
  injection; VirtIO QEMU transport/boot; cloud security; release contract;
  multi-architecture image workflow; and four AOSIN installer matrix jobs.
- CI diagnostics reported 389 kernel tests plus 3 kernel-bin tests passing,
  zero failures. The Quality Gate workspace tests, product-core evidence and
  workspace Clippy all passed.
- PR #73 was marked ready for review after CI passed. It remains unmerged.
- Runtime boundary unchanged: the pool is still a contract/test backend.
  Physical-frame dereferencing, live allocator integration, CR3/TLB lifecycle,
  checked user-copy and Ring 3 remain unimplemented.

### Integration automation audit — 2026-11-17

- The repository setting reports allow_auto_merge=false, while
  .github/workflows/branch-integration.yml invokes gh pr merge --auto and
  suppresses failure with || true. That path can propose branch PRs but cannot
  enable auto-merge under the current repository setting; the failure is
  hidden rather than surfaced.
- No branch or PR was merged by this run. Next automation task: make branch
  integration report this configuration mismatch and use an explicit
  head-SHA-guarded merge only after every required check is successful, or
  have the repository owner enable auto-merge intentionally.

### Scheduled development audit and CI diagnostics hardening — 2026-11-18

#### Repository state reviewed

- Default branch: `main`; audited head: `90a88a6c92cdd43889a76e499a0b7d82500dd695`.
- Workspace manifest contains 30 members across boot/protocol, kernel/kernel-bin, system services, first-party apps, tools and product tests.
- Open PRs at audit time: #74 (physical-frame page-table pool) and #64 (initd restart-policy enforcement). Neither was merged by this run.
- The current project-status and product-readiness documents correctly keep live per-process address-space activation, user/kernel isolation, durable VirtIO storage, real NIC-backed networking and ARM64/RISC-V runtime validation as open gates. No historical percentage was treated as evidence.
- The local Cell Runtime ABI 1.1 is a bounded, process-backed in-kernel lifecycle/IPC manager. Its own documentation explicitly excludes per-cell memory isolation and distributed authenticated IPC.
- The latest main-line evidence recorded before this run is 564 tests across 57 test binaries, with the substantive boot, loader, VirtIO and security workflows reported green. This does not validate the new code in PR #74.

#### PR #74 and memory boundary findings

- PR #74 head at review time: `01e39d0a005f2987da0f72f5cf3ce5d8b253379a`.
- The proposed `PhysicalTablePool` adds a checked arithmetic window and a physical-frame allocator adapter. Its unsafe constructor still relies on the caller to have installed the actual writable mapping; it does not activate CR3 or wire the live boot allocator.
- Review follow-up remains necessary: `table` and `table_mut` currently check only whether an address translates through the window, not whether the frame is currently owned by this pool. Before treating this as a hardened `TableFramePool`, add ownership-aware access/release tests or explicitly narrow the trait contract.
- The PR's first diagnostics run (run `44172684575`) failed in the diagnostic job. The workflow itself had `-e -o pipefail`; a failing Cargo pipeline exited before its `PIPESTATUS` value and `/tmp/diag.txt` were printed. Therefore the underlying Rust failure was not established from that run. Later PR checks reported `action_required`, not success; no pass is inferred.

#### CI change in this branch

- Updated `.github/workflows/ci-diagnostics.yml` so each Cargo command is allowed to finish and its pipeline exit status is captured immediately.
- The report is now printed before the step exits nonzero, and the existing `if: always()` PR-comment step can read the report.
- This is a diagnostics reliability fix only; it does not change kernel runtime behavior or readiness gates.

#### Integration automation finding (not changed in this PR)

- Scheduled run `44298405205` completed successfully at the workflow level but its logs show repeated PR-creation failures: GitHub Actions is not permitted to create or approve pull requests.
- Repository metadata reports `allow_auto_merge=false`; the integration workflow currently suppresses `gh pr merge --auto` errors with `|| true`. The green scheduled workflow therefore does not mean branches were integrated.
- Follow-up: make the integration workflow report these permission/settings blockers explicitly and avoid repeatedly attempting operations the workflow token cannot perform. Do not enable or merge unverified changes.

#### Validation and limitations

- Fresh GitHub Actions completed successfully on PR head `50c8829a44b3d5f57e4debcbffa383148adbf23a`:
  - CI Diagnostics: Clippy exit 0; `x86_64-unknown-none` kernel-bin check exit 0; workspace/all-target tests exit 0.
  - The diagnostic run reported 389 kernel tests and 3 kernel-bin tests passing, with zero failures.
  - AWEOS CI Pipeline: workspace compilation, workspace tests, formatting and boot-image generation passed.
  - AWEOS Quality Gate: UEFI loader, workspace check/tests, product-core evidence and Clippy passed.
  - Bootable Image Pipeline: kernel build, Multiboot2 validation and QEMU ISO smoke boot passed.
  - VirtIO Runtime: storage/driver contract tests, kernel build, QEMU VirtIO PCI detection and QEMU boot passed. This is not proof of actual block read/write or durable persistence.
  - Cloud Security Gate, Multi-Architecture Images and AOSIN installer matrix passed.
  - rustfmt-fix completed successfully without changing the final head.
- The initial failing run on head `226a534` is retained as historical evidence; the deterministic fixture correction was validated by the successful run on `50c8829`.
- No local Rust toolchain or QEMU runner was available through this GitHub-only editing session; all reported passes above are GitHub Actions results.
- The revised branch-integration workflow is triggered only by schedule/workflow_dispatch, so its new audit path has not yet been exercised by this PR's pull-request checks. Its next scheduled run must verify the new summary and explicit skip behavior.
- PR #74 remains separate and unmerged.

#### Next priorities

1. Review and test ownership checks for `PhysicalTablePool::table`, `table_mut` and `release`; do not wire it into boot until the direct map is actually installed and validated.
2. Obtain actionable diagnostics for PR #74 after the corrected diagnostics workflow runs.
3. Repair the scheduled branch-integration workflow's permission/configuration reporting.
4. Continue toward safe CR3/TLB lifecycle and checked user-copy, then prove isolation in QEMU before Ring 3.
5. Keep real VirtIO block persistence and two-node authenticated CellNet transport as separate runtime milestones.

### Scheduled branch-integration workflow permission fix — 2026-11-18

- Followed up on scheduled run `44298405205`. Its logs show repeated `gh pr create` failures because GitHub Actions is not permitted to create or approve pull requests; the old script suppressed each failure with `|| true`, so the run appeared green while creating no PRs.
- Repository metadata reports `allow_auto_merge=false`; the old workflow also suppressed `gh pr merge --auto` errors, so its success did not mean any branch was integrated.
- Updated `.github/workflows/branch-integration.yml` to inventory branches and open PRs, publish branches without an open PR in the Actions step summary, skip PR-creation attempts that the current workflow token cannot perform, and explicitly skip auto-merge while the repository setting is disabled.
- If repository auto-merge is enabled later, the workflow requests auto-merge only for non-draft PRs targeting the default branch with at least one reported check and all reported checks successful. Merge errors are no longer hidden.
- This changes automation reporting and safety only; no kernel code or branch was merged. Fresh workflow validation is pending.

### Deterministic bump-allocator regression fixture — 2026-11-18

- The corrected CI diagnostics workflow exposed the actual PR validation failure: Clippy and the `x86_64-unknown-none` kernel-bin check both exited 0, but workspace tests reported 388 passed and one failed.
- The failing test was `memory::allocator::tests::allocator_respects_alignment_and_bounds` at `kernel/src/memory/allocator.rs:112`. It expected 104 bytes remaining even though its stack byte array had alignment 1, so the array's base address could vary modulo 16 between runners. The implementation's alignment padding, and therefore the remaining-byte count, was nondeterministic relative to that hard-coded expectation.
- Updated only the test fixture: a 16-byte-aligned 136-byte backing object is used, with the allocator region starting exactly 8 bytes into it and a fixed 128-byte capacity. This makes the expected 8-byte base offset and 104-byte remaining count deterministic while still exercising the 16-byte allocation alignment.
- No production allocator behavior was changed. The fresh run on PR head `50c8829` passed the test and reported 389 kernel tests plus 3 kernel-bin tests with zero failures.
### Physical-frame table pool over a verified direct-map window — 2026-11-18

Continued the memory-isolation track from where PR #73 stopped: the
`TableFramePool` contract now has a kernel-side implementation over real
physical frames.

- New `kernel/src/memory/physical_tables.rs`:
  - `DirectMapWindow`: a bounded physical window with a fixed virtual
    offset. Construction validates alignment, nonzero length and both the
    physical-end and highest-virtual-translation overflow; `translate`
    returns `None` for unaligned or out-of-window addresses instead of
    guessing.
  - `FramePoolSource`: allocation plus ownership-tracked release,
    implemented for `BitmapFrameAllocator` (double-free maps to
    `DoubleRelease`, unknown frames to `UnknownFrame`).
  - `PhysicalTablePool`: implements `TableFramePool` over the source.
    Every allocated frame is verified to be inside the window before it is
    handed out; a frame the window cannot translate is returned to the
    source and the allocation fails closed. Handed-out frames are zeroed
    through the mapped virtual address with volatile stores, and
    `table`/`table_mut` resolve through the same checked window.
- The safety contract is explicit: the caller guarantees the window really
  maps the physical range writable. The module does not fake that mapping.
- Tests use real page-aligned host memory behind an identity window, so the
  dereferencing path is exercised for the first time: zeroing, table access,
  out-of-window fail-closed allocation with frame return, double-release
  detection, window arithmetic, and an end-to-end `build_address_space`
  run whose hardware walk matches the contract translation and whose PML4
  root entry in real memory points at a built frame.
- Explicit non-goals unchanged: CR3 activation, TLB invalidation and boot
  wiring of the live allocator. `build_address_space` still must not be
  called from boot code until the boot path installs a real window.
- Validation: pending fresh GitHub Actions on this branch; no pass or merge
  is claimed before the full check set is green.


### 2026-11-19 — Physical table-pool ownership and PR #76 CI correction

#### Baseline and failure diagnosis

- Continued on `feat/physical-table-pool-rebased-20261118`, PR #76, from
  head `b2dbfaa31a632958dbbdcc967c5665d8619327d7`; `main` remained at
  `bdfaf4dabc18593951c83c26f398e98cecd0ef34`.
- Inspected all nine workflow runs associated with the PR head. Two
  completed successfully (multi-architecture image workflow and installer
  matrix); seven failed. The workspace/Clippy diagnostics identified:
  - private `PageTableEntry` import from `page_table_build` (E0603);
  - two mutable-borrow conflicts in the out-of-window test (E0499);
  - a mutable/immutable borrow conflict in the end-to-end test (E0502);
  - an unused `TableMemory` import promoted to an error by the Clippy gate.
- The review also found a security/correctness defect: address-window
  containment alone allowed table access and release for any in-window
  frame, including frames allocated by another owner and frames already
  released by this pool.
- A further test-fixture defect was found during code inspection:
  the supposed identity window used virtual base zero. The translation
  contract requires the actual backing-buffer address as the virtual base;
  the old fixture would dereference address zero once compilation passed.

#### Implemented on the existing PR branch

- `kernel/src/memory/physical_tables.rs` now maintains a fixed-capacity
  ownership ledger (default 256 distinct physical addresses):
  - only frames allocated through this pool can be read, mutated or released;
  - foreign/in-window frames return `UnknownFrame` and are never freed;
  - released frames become tombstones, so access is denied and a second
    release returns `DoubleRelease`;
  - when the source allocator reuses a released address, its record becomes
    live again and the frame is zeroed before it is handed out;
  - ledger exhaustion fails closed and returns a newly allocated frame to
    the source when possible; a source that returns an already-live frame
    is not freed, avoiding corruption of the existing owner.
- Corrected the host test identity mapping to map the backing buffer at its
  actual virtual address.
- Fixed the private test import and borrow scopes; the end-to-end test now
  explicitly releases the frames transferred through `BuiltTables`.
- Updated `TableFramePool` contract documentation and exported the default
  ledger-capacity constant.
- No CR3 activation, TLB invalidation, live boot-map installation, Ring 3
  entry or production per-process isolation is claimed.

#### Validation

- GitHub Actions on the original PR head failed as listed above; the new
  ownership and fixture changes have not yet been run by CI.
- No local Cargo/QEMU execution is available in this GitHub-only session.
  The PR remains unmerged until fresh workspace, Clippy, QEMU, VirtIO and
  security workflows pass on the corrected head.

#### Next priorities

1. Validate the ownership ledger and corrected identity mapping on the new
   PR head; fix every fresh CI failure before considering merge.
2. Review the direct-map boot contract and implement a real, validated
   boot-time mapping only after the kernel memory map and allocator ownership
   rules are proven.
3. Continue with supervisor-only kernel mappings, per-process CR3 switching,
   TLB invalidation and checked physical user-copy before Ring 3.
4. Keep VirtIO durable block I/O and NIC-backed two-node CellNet transport as
   separate runtime milestones.


#### CI follow-up — explicit physical-pool capacity constructor

- The first fresh run on corrected head `6052faa09e3fc8c267733f044dc290798cabdea4`
  passed the non-test Clippy/self-test checks but the workspace test build
  still failed with E0284: Rust could not infer the const-generic ledger
  capacity at `PhysicalTablePool::new` call sites. The default const
  parameter does not resolve an associated constructor's unconstrained
  generic parameter.
- Split the API into `PhysicalTablePool::new` for the documented default
  capacity and `PhysicalTablePool::<_, N>::with_capacity` for an explicit
  bounded capacity. This keeps ordinary callers ergonomic while making
  custom limits unambiguous.
- Added a capacity-exhaustion regression test proving that an untracked
  source frame is returned rather than leaked when the ledger is full.
- Fresh validation on head
  `cb4acffd0cc83f13265df743b6c9250d258854b6` is pending. The E0284 run is
  recorded as a failure, not a pass; no merge/readiness promotion is claimed.


#### CI follow-up — high-half test literal typing

- The first run after splitting `new` / `with_capacity` passed workspace
  library compilation and the non-test kernel checks, so the E0284 constructor
  inference issue was resolved.
- Workspace test compilation then failed on two pre-existing high-half
  address expectations in the new test: unsuffixed literals passed to
  `usize::try_from` defaulted to `i32` and triggered
  `overflowing_literals`. Added explicit `u64` suffixes to both values.
- Corrected source commit: `414789a42224abe3bb496530316416e054bd38d6`.
  Fresh CI is pending on this exact source head. The earlier test-build
  failure remains recorded and is not counted as a pass.


#### CI follow-up — direct-map overflow fixture boundary

- The next workspace run compiled the full test targets and executed the kernel
  suite: **395 passed, 1 failed**. The only failure was the virtual-overflow
  expectation in `window_rejects_misaligned_empty_and_overflowing_configuration`.
- The fixture used a one-page window at `u64::MAX - 0xfff`. The implementation
  correctly accepts that range because its highest translated byte address
  is exactly representable; the test's expected overflow was wrong.
- Increased that fixture's length to two pages, making the highest translated
  page address actually overflow. Production translation logic was unchanged.
- Corrected test commit: `3a50c7d84ac38b286e8dfbb7aaed5742926c665f`.
  Fresh full validation is pending; the 395/1 result is recorded as a failed
  test run, not a pass.


#### CI follow-up — workspace test pass and Clippy cleanup

- On head `ba7e608106096b5ae60453e90fce0e3eeac21679`, the kernel/workspace
  test suite completed with **396 passed, 0 failed**, including the new
  ownership, foreign-frame, reuse, capacity-exhaustion and real-table tests.
- The separate Clippy gate still failed on four test-only diagnostics:
  one `needless_range_loop` and three `drop_non_drop` findings caused by
  explicit `drop(pool)` calls used to end mutable borrows.
- Replaced the indexed zero-check with iterator traversal and used lexical
  scopes to end the pool borrows before touching the underlying allocator.
  This preserves the test's ownership assertions without suppressing Clippy.
- Correction commit: `00d665d3b2707d61dd9283f3ae522875b385f9bc`.
  Fresh workspace, formatting, Clippy, boot-image, VirtIO and security
  validation is pending on this new head.


#### CI follow-up — test/Clippy pass, formatter bot correction

- On the pre-format head `bbca195d8b6b3fa7a88e165dbe514ddcff8958a5`:
  - workspace compilation passed;
  - kernel/workspace tests passed (396 kernel tests plus 3 kernel-bin
    bootstrap tests in the diagnostics run);
  - the standalone Clippy report passed;
  - CI diagnostics completed successfully.
- The quality gate, CI pipeline, VirtIO gate and cloud-security gate stopped
  at formatting before their downstream steps. No QEMU/VirtIO runtime result
  is claimed from those skipped jobs.
- The repository's rustfmt bot applied the exact formatting-only correction
  in commit `0abee6b3210b16511c7bda1002548a92d7db03cb`. No semantic source
  changes were made by that bot commit.
- GitHub reported all runs for the bot-authored head as `action_required`
  with no jobs created. A new owner-authored documentation commit follows
  this entry to trigger the full suite against the already-formatted tree.
  Until that run completes, formatting and all downstream runtime gates are
  still treated as unverified.


### 2026-11-19 — Final validation snapshot for PR #77

- Current PR head at the time of this entry: `e1bec4b4702a7197699289e0ea1ed116edb65ba4`.
- Passed on this head:
  - workspace compilation, all workspace tests and formatting;
  - standalone Clippy;
  - AWEOS Quality Gate, including package/recovery, persistent-storage,
    network-runtime, security-focused and release-blocker checks;
  - UEFI loader validation;
  - bootable ISO/GPT image generation and QEMU ISO smoke boot;
  - QEMU exception-injection self-test;
  - VirtIO block transport detection and QEMU boot with a VirtIO block device;
  - cloud-security and cloud-release-contract gates;
  - all four AOSIN CLI/GUI installer matrix jobs (Linux and Windows).
- The multi-architecture image workflow was still installing its boot-image
  and emulator tools when checked; no final result is claimed for that job.
- PR #77 remains open and unmerged. PR #76 was closed as superseded; its
  branch and commits were preserved. No main-branch changes or force pushes
  were made.
- Scope boundary: the new physical table pool passes its host-backed
  ownership/access/reuse/capacity tests, but it is not yet wired into the
  live boot memory map. QEMU boot and VirtIO checks validate the existing
  boot/runtime pipeline, not CR3 activation or process isolation through
  this new pool. CR3 switching, TLB invalidation and Ring 3 remain open.
- Next run: finish multi-architecture workflow review, then audit the boot
  memory-map handoff and design a validated direct-map/allocator integration
  without activating unverified page tables.




## 2026-11-26 — Run 19 follow-up: Clippy correction on PR #89

- Resumed the existing Run 19 branch and PR #89 after reading its dated development log, current main commit, open PR state, and latest GitHub Actions results.
- On PR head `2762eeb2241d5618f881d47c4b794c1d4db54dad`, the workspace compile/tests/format, ISO QEMU smoke boot, VirtIO block QEMU boot, UEFI loader, multi-architecture image and installer checks passed. Quality, Cloud Security and exception-injection self-test workflows failed at Clippy. The exception-injection runtime test was skipped because its feature-build lint step failed.
- Exact failure: `clippy::unnecessary_cast` at `kernel/src/memory/boot_direct_map.rs:85`; `MAX_MEMORY_REGIONS` is declared `u32`, the same type as `memory_region_count`. Removed the redundant `as u32` only.
- **Fix commit:** `146d71adc81d13f727c8a665a2e0deb0f0dd4446` on `feat/boot-direct-map-gate-2026-11-25`, within existing PR #89. No main-branch write, force push, deletion or merge was performed.
- **Changed file:** `kernel/src/memory/boot_direct_map.rs`.
- **Validation status:** fresh CI on the fix commit is pending; this GitHub-only execution had no local Rust/QEMU toolchain. Prior-head successes are not represented as tests of the fix commit. No green status or production readiness is claimed.
- **Open technical limits:** direct-map itself is not installed at boot; the kernel-half root probe remains validation-only and must not publish CR3 or enable Ring 3; process-root sharing, TLB invalidation, user-copy backend, durable VirtIO I/O and NIC-backed two-node CellNet remain future milestones.
- **Next priorities:** inspect exact CI on the new PR head; require Clippy and exception-injection QEMU pass; then review the root-probe evidence and define a tested CR3/TLB and per-process teardown contract before activation.


### 2026-11-26 — Run 19: enforce and validate the kernel-half QEMU probe

- Reviewed the QEMU integration test after the new `boot_kernel_root_probe` was added. The harness asserted the older table-pool probe and foreign-frame rejection, but did not require the new `[cell:kernmap]` probe to pass; a skipped/failed new probe could therefore have been hidden by an otherwise passing QEMU run.
- Added a fail-closed assertion in `tests/product-core/src/lib.rs` requiring `AWEOS: [cell:kernmap] probe result: PASS`. Commit: `07fa2425fc3c5e219bcab37a965681a61f56dcc4`. No production boot behavior changed.
- Exact code head `07fa2425fc3c5e219bcab37a965681a61f56dcc4` passed all 9 PR workflow families: Quality Gate, CI Pipeline, Clippy, CI Diagnostics, Bootable Image Pipeline, VirtIO Runtime, Multi-Architecture Images, Cloud Security/Release Contract, and AOSIN Cross-Platform Installers. The rustfmt-fix workflow also passed.
- Test evidence: workspace validation passed; `aweos-kernel` **469 passed / 0 failed**, `aweos-kernel-bin` **4 passed / 0 failed**, and the QEMU exception-injection self-test **1 passed / 0 failed** with the new root-probe assertion. ISO smoke boot, VirtIO transport/QEMU boot, multi-architecture image build, cloud-security/release gates and Linux/Windows GUI/CLI installer jobs all passed.
- The new QEMU assertion now proves the unpublished kernel-half root's install/readback, supervisor-only mapping policy, untouched user half, and table-frame release path. It does not prove CR3 activation, TLB invalidation, Ring 3, production process isolation, or a production direct map. VirtIO QEMU success remains transport/boot evidence, not durable storage; multi-architecture image success is not ARM64/RISC-V runtime evidence.
- PR #89 remains open and unmerged; all work remains on `feat/boot-direct-map-gate-2026-11-25`. No main-branch write, force push, branch deletion or merge was performed.
- Next priorities: define and test CR3/TLB activation and per-process root ownership/teardown before publication; connect checked user-copy to a verified physical backend; then implement real VirtIO block request/completion/flush and NIC-backed two-node CellNet transport.
