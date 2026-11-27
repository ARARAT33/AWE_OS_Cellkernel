# AI Development Log — 2026-11-26 (Run 20)

## CR3/TLB activation contract with a structural identity-image block

- **Baseline:** `main` at `68ef817ceb93e54bc3ceb881d10fd54201eacf8b` (PR #89, Run 19, merged this run after its CI reported `mergeable_state: clean` with the QEMU selftest asserting `[cell:kernmap] probe result: PASS`). No force pushes; the merged PR preserved its branch and commits.
- **Architectural finding (recorded, not hidden):** the kernel currently executes at user-half identity virtual addresses. A root built by `KernelPageTableBuilder` is kernel-half-only by design, so a `mov cr3, <kernel-half-root>` from this image would fault on the very next instruction fetch: the running code, stack and serial output all live in the user half, which the target root does not map. Any CR3 switch attempted today would triple-fault. Activation therefore stays structurally blocked until the kernel image itself is loaded and executed at kernel-half addresses.
- **Implemented on `feat/cr3-tlb-activation-contract-2026-11-26`:**
  - New `kernel/src/arch/x86_64/cr3_contract.rs`: `validate_root_physical` (zero/misaligned/beyond-4-level-physical rejection), `cr3_root_of`/`cr3_low_flags`, `check_activation_preconditions` (validates roots, rejects no-op switches, refuses activation while `identity_image_can_activate_kernel_half_root()` is `const false`), and `requires_pge_or_explicit_invalidation` documenting the TLB policy (a CR3 write flushes non-global translations; `GLOBAL` entries need CR4.PGE management or explicit `invlpg`). 8 host tests, including assertions that the bootstrap table/leaf flags carry no GLOBAL bit.
  - `kernel/src/memory/table_arena.rs`: new `boot_cr3_contract_probe` (serial tag `[cell:cr3]`), a kernel-selftest QEMU probe that reads the live CR3, validates the running bootstrap root against the contract, and asserts the contract still blocks activation. No CR3 write, no TLB invalidation, no mapping change.
  - `kernel/src/arch/x86_64/mod.rs`: registered the new module. `kernel/src/entry.rs`: runs the probe in kernel-selftest builds after the kernel-half root probe.
  - `tests/product-core/src/lib.rs`: the QEMU exception-injection selftest now fails closed unless `AWEOS: [cell:cr3] probe result: PASS` appears in the serial log.
  - `PROJECT_STATUS.md`: Run 20 evidence and updated next-cycle priorities.
- **Test results:** pending remote CI on this exact branch head; no green result is claimed from this run. No local Rust toolchain or QEMU is available in this GitHub-only session.
- **Explicit non-claims:** no CR3 write, no TLB invalidation, no direct-map installation, no per-process address space, no Ring 3. The CR3 activation contract deliberately returns `IdentityImageCannotSwitch` for every caller today.
- **Next priorities:**
  1. Higher-half kernel loading path (loader/ELF placement at kernel-half virtual addresses) so the running image can prove the structural precondition the contract requires.
  2. Only after that evidence: unlock `identity_image_can_activate_kernel_half_root` with a real proof and add a QEMU CR3-switch/TLB-invalidation isolation test.
  3. Continue the standing order: checked user-copy backend wiring, VirtIO durable block I/O, NIC/UDP two-node CellNet transport.
