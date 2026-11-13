# AWE_OS CellKernel

**AWE development project #2 — active, preserved, and intended for long-term development.**

AWE_OS CellKernel is a Rust-first operating-system project centered on a small privileged CellKernel, a dedicated AWE boot chain, capability-oriented security, modular drivers/services, and a native userspace platform.

> **Status:** Development project — not a finished daily-driver OS.
>
> A historical snapshot of the repository before the 2026-08-27 project-state/documentation restructuring is preserved in `archive/pre-restructure-2026-08-27`.

## Project direction

The goal is to turn the existing architecture into a progressively more executable, testable, and reproducible operating-system platform. Development is evidence-driven: a subsystem is not considered complete merely because an interface or design document exists.

```text
UEFI / Firmware
      |
 AWE Loader
      |
 AWE Boot Protocol
      |
 CellKernel
  CPU • memory • interrupts • scheduler
  processes • syscalls • IPC • capabilities
      |
 Device / System Services
  PCI • ACPI • VirtIO • storage • network • input • display
      |
 AWOSA Runtime
  packages • manifests • sandbox • capabilities
      |
 AWE Capsule
      |
 AYUI + Native Applications
```

## Current implementation foundation

The repository already contains substantial kernel, boot, hardware-contract, storage, networking, userspace, package, update/recovery, and CI foundations. These components are being developed toward end-to-end runtime validation rather than treated as a completed product.

Important existing areas include:

- AWE loader and versioned boot information contract.
- x86_64 paging, early identity mapping, CR3 activation and interrupt primitives.
- Process, scheduler, syscall, IPC, capability and security foundations.
- PCI, ACPI, APIC/IOAPIC and VirtIO contracts/execution primitives.
- Bounded block storage, GPT, VFS, journaling and recovery foundations.
- IPv4/UDP/TCP transport foundations and bounded socket handling.
- Versioned userspace service and identity contracts.
- AWOSA runtime and native `.asd` / `.awos` package-admission foundations.
- A/B update and recovery state-machine foundations.
- Automated formatting, compilation, tests, Clippy and boot-related CI workflows.

These are implementation foundations, not a claim that every subsystem is already production-ready.

## Development status

The old repository documentation used percentage checkpoints. Those checkpoints are retained as historical engineering context, but the active project now uses **runtime evidence and milestone gates** as the primary measure of progress.

See **[PROJECT_STATUS.md](PROJECT_STATUS.md)** for the current project state and development rules.

## Documentation

- **[PROJECT_STATUS.md](PROJECT_STATUS.md)** — current project identity, development direction, preservation point, and next cycle.
- **[PRODUCT_READINESS.md](PRODUCT_READINESS.md)** — evidence-based readiness gates.
- **[ARCHITECTURE.md](ARCHITECTURE.md)** — system architecture and boundaries.
- **[docs/AWE_OS_100_PERCENT_MASTER_PLAN.md](docs/AWE_OS_100_PERCENT_MASTER_PLAN.md)** — long-term implementation plan.
- **[docs/ENGINEERING_GUIDE.md](docs/ENGINEERING_GUIDE.md)** — engineering workflow and invariants.
- **[docs/ABI.md](docs/ABI.md)** — boot ABI contract.
- **[docs/BOOT_ARCHITECTURE.md](docs/BOOT_ARCHITECTURE.md)** — boot flow.
- **[docs/DRIVER_SYSTEM.md](docs/DRIVER_SYSTEM.md)** — driver architecture.
- **[docs/AWE_CAPSULE.md](docs/AWE_CAPSULE.md)** — native application security model.
- **[docs/AWE_XENOSENSE.md](docs/AWE_XENOSENSE.md)** — experimental research direction.
- **[docs/ADVANCED-CELL-RUNTIME-ARCHITECTURE.md](docs/ADVANCED-CELL-RUNTIME-ARCHITECTURE.md)** — proposed Living Cells, Cell Continuity Capsule, Cell Immune System, Virtual Hardware Fabric, and AWENET boundary.

Historical progress/evidence files under `docs/` remain useful as dated records. They should not override the current status in `PROJECT_STATUS.md` and `PRODUCT_READINESS.md`.

## Roadmap

### Phase 1 — Executable core

- [ ] Reliable x86_64 UEFI/QEMU boot.
- [ ] Deterministic CellKernel entry and early initialization.
- [ ] Executed memory/paging and interrupt validation.
- [ ] Timer/scheduler execution evidence.

### Phase 2 — Hardware and runtime

- [ ] VirtIO end-to-end device exercises.
- [ ] Persistent storage runtime.
- [ ] Network runtime and recovery.
- [ ] Userspace service startup and supervision.

### Phase 3 — Native platform

- [ ] AWOSA loader/runtime integration.
- [ ] Cryptographic trust for `.asd` and `.awos` packages.
- [ ] Native application lifecycle and sandbox execution.
- [ ] A/B update and recovery exercised end-to-end.

### Phase 4 — Desktop and ecosystem

- [ ] AYUI compositor and windowing.
- [ ] Terminal and first-party applications.
- [ ] Filesystem and network managers.
- [ ] SDK/App Builder foundations.

### Phase 5 — Production validation

- [ ] Security hardening.
- [ ] Fuzz/stress testing.
- [ ] Reproducible release builds.
- [ ] Hardware compatibility matrix.
- [ ] ARM64/RISC-V64 validation.
- [ ] Release certification based on evidence.

## Engineering rules

- Keep privileged code Rust-first and `no_std` where appropriate.
- Keep the trusted computing base small.
- Keep hardware-dependent code behind explicit interfaces.
- Treat firmware, memory, process, IPC, ABI, and capability boundaries as security boundaries.
- Prefer deterministic validation and bounded resource use.
- Never describe planned work as implemented without evidence.
- Experimental intelligence/recovery concepts must remain subordinate to deterministic security and must not silently modify the trusted computing base.

## License

AGPL-3.0.
