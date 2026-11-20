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


## Checked user-copy and physical-frame reservation update (2026-11-20)

PR #78 merged the checked physical user-copy contract into `main`. It validates the complete user range before calling a page-bounded `PhysicalAccess` backend and rejects requests above 1 MiB. Its host tests cover page-boundary copies, validation-before-access, backend faults and per-page segmentation. This does not yet provide a live physical-access backend or Ring 3 isolation.

The `PhysicalTablePool` implementation can allocate, zero, access and release owned physical table frames through a caller-supplied `DirectMapWindow`, but the boot path does not yet create that window or instantiate the pool.

PR #79 (`feat/reserved-physical-frames-2026-11-20`) adds explicit, bounded-range reservation to `BitmapFrameAllocator` and passed all 16 PR check-runs on source head `a225e73280a27cf4170c377175c24a23e1454d94`; the PR remains draft and unmerged. The Quality Gate and CI Diagnostics each reported 405 kernel tests plus 3 kernel-bin tests with zero failures. This is still only an allocator primitive: it does not reserve any actual boot memory until the boot handoff integration supplies and validates those ranges. The next integration must reserve the kernel image, loader/boot metadata, memory-map descriptors, bootstrap page tables and stacks before exposing frames to the physical table pool.

Until that integration and runtime tests exist, keep per-process CR3 switching, TLB invalidation, Ring 3, QEMU user/kernel isolation and production process isolation unchecked.
