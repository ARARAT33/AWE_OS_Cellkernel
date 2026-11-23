# AWE_OS Product Readiness

**Project status:** Active development / long-term project #2  
**Readiness policy:** evidence-based; no percentage is treated as release certification.

## Current readiness position

AWE_OS CellKernel is an **active development project**, not a finished daily-driver operating system. The repository contains substantial implementation foundations across boot, kernel execution, hardware contracts, storage, networking, userspace, native package admission, update/recovery, and CI.

The previous **90% implementation checkpoint** is retained as historical context. It must not be interpreted as 90% product completion or release certification. The current project uses executable runtime evidence and milestone gates as the authoritative readiness model.

## Implemented foundations

The current tree contains foundations for:

- AWE loader and versioned boot information contract.
- x86_64 paging, early identity mapping, CR3 activation and interrupt primitives.
- Per-process x86_64 address-space metadata and a four-level user page-table builder; currently verified through a test-only `TableMemory` backend, not the live frame allocator.
- Process, scheduler, syscall, IPC, capability and security foundations.
- PCI, ACPI, APIC/IOAPIC and VirtIO hardware contracts/execution primitives.
- Bounded block-device, GPT, VFS, journaling and recovery foundations.
- IPv4/UDP/TCP transport and bounded socket foundations.
- Versioned userspace service and identity contracts.
- AWOSA runtime contracts and native `.asd` / `.awos` package admission/lifecycle foundations.
- A/B update and recovery state-machine foundations.
- Automated formatting, compilation, tests, Clippy and boot-related CI workflows.

## Validation gates

### Gate A — Boot

- [ ] UEFI/QEMU image boots reliably from the current `main` tree.
- [ ] Boot protocol validation is exercised in runtime.
- [ ] CellKernel reaches a deterministic initialized state.
- [ ] Timer/interrupt execution is demonstrated.

### Gate B — Kernel execution

- [ ] Memory/paging operations are exercised beyond static unit tests.
- [ ] Scheduler/process execution is demonstrated.
- [ ] Syscall and capability enforcement are exercised end-to-end.
- [ ] IPC behavior is exercised with bounded failure handling.

### Gate C — Hardware and I/O

- [ ] VirtIO device execution is demonstrated in QEMU.
- [ ] Persistent storage path is exercised.
- [ ] Network runtime path is exercised.
- [ ] Input/display paths are exercised where supported.

### Gate D — Userspace/native platform

- [ ] Init/service manager starts real services.
- [ ] AWOSA runtime loads a native component.
- [ ] `.asd` admission is exercised end-to-end.
- [ ] `.awos` admission/lifecycle is exercised end-to-end.
- [ ] Cryptographic signing/verification is integrated with trust roots.

### Gate E — Recovery and release

- [ ] A/B update and rollback are exercised in a runtime/recovery path.
- [ ] Crash/recovery behavior is validated.
- [ ] Fuzz/stress testing is established for critical parsers and boundaries.
- [ ] Reproducible release builds are demonstrated.
- [ ] Hardware compatibility matrix is maintained.
- [ ] No high/critical unresolved issue blocks release.

## Remaining long-term work

- Real DMA/IOMMU enforcement.
- Full NVMe/AHCI, xHCI/USB, HID, Ethernet/Wi-Fi/Bluetooth, audio and GPU runtime.
- Persistent native filesystem implementation and recovery tooling.
- Full network stack services such as ARP/IPv6/ICMP/DNS/DHCP/TLS/firewall/recovery.
- Userspace loader, device/filesystem/network managers and crash recovery.
- Complete AWOSA SDK, headers and bindings.
- Cryptographic trust integration.
- AYUI desktop and first-party applications.
- Optional Linux/Windows/Android compatibility layers.
- App Builder/SDK and native ecosystem tooling.
- ARM64/RISC-V64 production validation.
- Full security hardening and release certification.

## Definition of done

AWE_OS is fully product-ready only when the required boot, kernel, driver, storage, networking, userspace, security, recovery and release gates are implemented **and backed by reproducible runtime evidence**.

Documentation, architecture diagrams, static compilation, or percentage labels alone do not satisfy a readiness gate.

## Historical records

Dated progress/evidence documents under `docs/` are preserved for traceability. They describe their historical milestone and should not be read as a guarantee about the current `main` tree.

See `PROJECT_STATUS.md` for the current project-state source of truth.


## Per-process virtual-memory status (2026-11-16)

The repository now contains a bounded `AddressSpace` contract and a
four-level PML4 → PDPT → PD → PT builder for validated 4 KiB user mappings.
The builder rejects writable+executable leaves, preserves the kernel half of
the PML4, and is covered by host unit tests. The latest validated PR head
passed 564 tests across 57 test binaries and all nine substantive CI workflows.

This is **not yet process memory isolation**. The current `TableMemory`
implementation is test-only; it is not wired to the physical frame allocator
or a production physical-to-virtual mapping. CR3 switching between process
roots, TLB invalidation/shootdown, page-table teardown/reference counting,
physical checked user-copy, Ring 3 entry, and QEMU user/kernel isolation tests
remain unimplemented. Keep the memory-isolation and userspace execution gates
unchecked until those runtime milestones are proven.


## Checked user-copy and live boot-owned frame reservation update (2026-11-21)

PR #78 merged the checked physical user-copy contract into `main`. It validates the complete user range before calling a page-bounded `PhysicalAccess` backend and rejects requests above 1 MiB. Its host tests cover page-boundary copies, validation-before-access, backend faults and per-page segmentation. This does not yet provide a live physical-access backend or Ring 3 isolation.

PR #79 (reservation-aware bitmap frame allocation) and PR #80 (boot-owned range derivation and all-or-nothing reservation) are merged into `main`.

PR #81 (`feat/live-boot-memory-reservations-2026-11-21`) connects the reservation helper to the monotonic `PhysicalFrameAllocator` used by `kernel_entry`. It adds a bounded reservation ledger, atomic batch preflight/commit, frame skipping for every page intersecting a boot-owned range, and a distinct fatal boot status on reservation failure. The x86_64 Multiboot/PVH image now exports `__kernel_start` and `__kernel_end`; the kernel binary copies those checked bounds into `BootInfo.kernel_base/kernel_size` before entering the kernel.

The exact PR #81 head `17b54cc64eb07bec95aa3922597c4f934808efc0` passed **15/15 GitHub check-runs**. This includes `cargo check --workspace --all-targets`, `cargo test --workspace`, formatting, Clippy, the UEFI loader build, the bootable ISO QEMU smoke test, the exception-injection self-test, the VirtIO runtime workflow, the multi-architecture image workflow, the Cloud Security/Release checks and all four AOSIN installer jobs. The new allocator/reservation regression tests passed. PR #81 remains a draft and is not merged.

The QEMU VirtIO workflow confirms that a VirtIO PCI device is exposed and that the kernel reaches RUNNING. It does **not** demonstrate block request/completion, read-after-write, flush or durable persistence.

This is still not full physical memory management. The live allocator is monotonic, is only probed once during kernel entry, and is not yet retained as a global allocator. The `PhysicalTablePool` implementation can allocate, zero, access and release owned physical table frames through a caller-supplied `DirectMapWindow`, but the boot path does not yet create that window or instantiate the pool. PR #81 does not add CR3 switching, TLB invalidation, Ring 3, QEMU user/kernel isolation or production process isolation.

Until those runtime milestones exist, keep per-process CR3 switching, TLB invalidation, Ring 3, QEMU user/kernel isolation and production process isolation unchecked.



## Superseding memory-path evidence (2026-11-23)

PR #84 is now merged into `main` at merge commit
`a1248b8974919a970f329d6a72ae5cefc228f749`. It adds a bounded
`TableArena` that implements the frame-pool source used by
`PhysicalTablePool`, including release/reuse and zero-before-reuse behavior.
A QEMU self-test build includes a boot-time probe over the bootstrap identity
mapping. PR #85 adds a contiguous unreserved-run preview and makes the QEMU
exception test require the probe's PASS marker plus foreign-frame rejection.
The QEMU exception-injection self-test passed on code head
`d4b65c1341129735516a05910c143e82b6fe0b48` with both assertions satisfied.
This is runtime evidence for the bounded self-test arena only, not for a
persistent production allocator or process address-space isolation.

This does not supersede the remaining isolation gates: the arena is reserved
only for the self-test probe, the allocator/pool is not retained as a
production global memory manager, no per-process CR3 switch or TLB
shootdown is active, Ring 3 remains deferred, and the checked user-copy
backend is not wired to a verified live user-memory mapping. The kernel still
uses a lower-half bootstrap identity map, so user mappings in the same PML4
region require a deliberate kernel higher-half mapping design before process
isolation can be activated.

