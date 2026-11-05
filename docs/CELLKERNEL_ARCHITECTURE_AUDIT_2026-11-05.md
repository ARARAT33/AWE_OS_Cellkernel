# AWE_OS CellKernel — evidence-based architecture audit

**Audit snapshot:** `main` at `6b77c7bb2d5d2b27878811107935ce346b8d8148` (2026-11-04).  
**Scope:** repository tree/workspace inventory, project/readiness/architecture documents, all nine workflow definitions, and focused source review of boot, memory, interrupt, process, IPC, capability, driver, storage, network, AWOSA/service and AYUI boundaries.

This is a focused engineering audit, not a formal line-by-line security review of every source file. The repository tree contains 447 tracked entries and 29 Cargo manifests; the nine GitHub Actions workflows were enumerated.

## Executive finding

AWE_OS has a substantial Rust `no_std` foundation and a QEMU-tested x86_64 boot image. It is not yet a complete daily-driver OS. The largest gap is end-to-end integration: several subsystems have bounded data structures, parsers, state machines and tests, but are not yet driven by real hardware/runtime paths.

A critical security issue was found in the bootstrap page tables: the initial Multiboot assembly used U/S=1 at PML4/PDPT and 2 MiB leaves (`0x87`), and the Rust identity-map helper also included `PageFlags::USER`. The boot path additionally attempted to enter Ring 3 without a per-process address space. That would expose the bootstrap identity-mapped kernel memory to CPL3. This development pass changes the bootstrap maps to supervisor-only and defers Ring 3 until isolated user mappings exist.

A second correctness issue was found in interrupt-frame accessors: an IST stack switch saves the interrupted SS:RSP even when CPL remains zero. The prior accessor treated every same-CPL interrupt as if those words were absent. This pass makes frame interpretation aware of the same IST-vector map used by IDT configuration.

## Repository and workspace map

- `boot/protocol`: versioned `BootInfo` contract and validation.
- `boot/aweloader`: architecture-neutral loader state, memory-map, ELF, UEFI, policy and verification modules.
- `boot/arm`, `boot/riscv`, and x86 boot assets: architecture/boot scaffolding exists; the current runtime evidence is centered on x86_64.
- `kernel`: `no_std` core with architecture, memory, process, scheduler, syscall, IPC, security, driver, storage, network, time and service contracts.
- `kernel-bin`: Multiboot1/Multiboot2/PVH entry and bootstrap page tables.
- `services`: appd, ayui, driverd, identityd, initd, netd, nexus, platformd, securityd, storaged and update crates.
- `apps`: calculator, file manager, package center, recovery, settings, system monitor, terminal and text editor.
- `tools`: AOSIN GUI, awe-cli and awescript.
- `tests/product-core`: cross-service/product integration tests.

## Subsystem evidence and gaps

| Area | Evidence in source | Current classification / gap |
|---|---|---|
| Boot | Versioned BootInfo validation; AWELoader modules; x86_64 Multiboot1/2/PVH entry; image-building workflow | Implemented foundation. UEFI/ISO QEMU smoke is exercised by CI; ARM64/RISC-V runtime is not established by this evidence. |
| CPU/interrupts | GDT/TSS, 256 IDT vectors, ISR stubs, PIC/PIT and IST stack structures | x86_64 primitives exist. This pass wires IST into the boot path and fixes IST frame semantics. Real NMI/#DF/#MC delivery tests remain missing. |
| Memory | Physical-frame allocator, page-table entry/flags, mapper, bump/heap allocators and a 1 GiB identity-map helper | Foundation, not a full process VM system. Per-process address spaces, guard pages, full user/kernel separation and IOMMU/DMA enforcement remain incomplete. |
| Process/scheduler | Process states, ID allocation, resource budgets, scheduler queue, context-switch frame/backend and reap paths | Lifecycle and switch primitives exist. Timer-driven preemptive scheduling is not wired end-to-end; boot output must not claim a running scheduler. |
| Syscalls | SYSCALL MSR setup and assembly entry plus typed syscall ABI/dispatch modules | Early entry/contract exists. A complete validated user-buffer and capability-enforced syscall path is not yet demonstrated end-to-end. |
| IPC/cells | Bounded mailbox, service channels, handshake/shared-ring primitives and service bus | Local bounded IPC foundations exist. There is no demonstrated general cell runtime with fault containment and remote IPC transport integration yet. |
| Capabilities | Rights model, derivation, lookup, revoke and fixed-capacity table | Useful security foundation. The capability ID allocator currently wraps from `u64::MAX` back to 1; this can reuse a stale handle ID and is a high-priority follow-up. |
| Drivers/HAL | PCI enumeration, VirtIO contracts/queues, ACPI/APIC/IOAPIC service modules and driverd supervisor/contracts | Discovery and protocol foundations exist. APIC/IOAPIC EOI/routing runtime, broad USB/HID/GPU/audio/Wi-Fi support and DMA/IOMMU enforcement are not established. |
| Storage | GPT parser with CRC checks, bounded VFS metadata, RAM disk and journal/recovery state machine; storaged fixed-capacity volume/cache structures | Parsers and state contracts exist. Durable block-device read/write, native filesystem persistence and power-loss recovery are not yet proven end-to-end. |
| Networking | ARP packet/cache, IPv4 parsing/checksum, UDP/TCP header and socket-table primitives, firewall rules; netd service structures | Protocol foundations exist. Full Ethernet device I/O, IPv6, DNS, DHCP, TLS, routing, reconnect and distributed discovery are not demonstrated. |
| AWOSA/services | initd service lifecycle/restart/quarantine contracts, driverd, appd, identityd, securityd, update, netd and storaged crates | Service contracts and bounded managers exist. A complete isolated service supervisor with real process start/stop, health checks, signed package loading and crash recovery remains to be integrated. |
| Packages | `.asd`/`.awos` format/admission and driver package validation modules | Admission foundations exist; cryptographic trust-root integration, dependency resolution and secure install/rollback runtime need evidence. |
| AYUI | Bounded software compositor/widget/window state, terminal backend, notifications/themes/accessibility structures | UI engine code exists. Native display/GPU/input integration and a booted interactive desktop are not proven by current boot tests. |
| Multi-architecture | Boot protocol enumerates x86_64/AArch64/RISC-V64; boot crates exist | Port scaffolding only. Each architecture needs its own build and runtime evidence. |

## CI/workflow audit

The main branch at the audit snapshot had seven latest workflow runs completed successfully on 2026-11-04:

- AWEOS CI Pipeline
- AWEOS Quality Gate
- AWEOS Cloud Security Gate
- AWEOS Bootable Image Pipeline
- AWEOS VirtIO Runtime
- AWEOS Multi-Architecture Images
- AOSIN Cross-Platform Installers

The successful boot workflow includes a QEMU ISO smoke test. The VirtIO workflow verifies QEMU exposes a VirtIO PCI device and boots the image with that device attached; this is not, by itself, proof of durable filesystem I/O.

The current IST boot-wiring PR's first workflow runs were marked `action_required` and had no jobs available to inspect. Earlier PR-head runs contained failures in CI/Quality/Cloud Security/VirtIO while the image-build path succeeded. Those are not treated as passing validation.

## Work in this development pass

1. Wire the dedicated NMI/#DF/#MC IST stacks after IDT stub installation and before IDT load; fail closed on configuration failure.
2. Correct same-CPL IST stack-frame interpretation and add a regression test.
3. Preflight all critical IDT gates and stack tops before mutating TSS/IDT state.
4. Remove U/S from both bootstrap page-table builders and add tests for the shared supervisor-only permission constants.
5. Defer Ring 3 entry until a real per-process user address space exists.
6. Remove fabricated early-boot readiness output and update the AI development log with actual CI state.

## Next priorities

1. Fix capability-ID exhaustion so IDs never wrap/reuse.
2. Add QEMU exception exercise for #DE/#UD/#PF and actual NMI/#DF/#MC IST delivery.
3. Implement per-process page tables and explicit user code/data/stack mappings before re-enabling Ring 3.
4. Implement APIC/IOAPIC EOI and routing, then timer-driven context switching and FPU/XSAVE ownership.
5. Prove block-device persistence/recovery, network device I/O and AWOSA service lifecycle end-to-end.
6. Add fuzz/stress coverage for boot parsers, IPC, capabilities, storage metadata and network packet boundaries.

No completion percentage is assigned. Interfaces, tests, builds and runtime validation are reported separately.
