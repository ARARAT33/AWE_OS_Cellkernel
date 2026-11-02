# AWE_OS 1.0 Hardware Support & Validation Matrix

This document defines the formal compatibility matrix for AWE_OS 1.0 across primary (x86_64), secondary (ARM64), and tertiary (RISC-V 64) architectures, distinguishing executable QEMU emulated paths from physical bare-metal hardware validation.

---

## 1. Primary Target Architecture: x86_64

| Subsystem | Hardware Controller / Architecture | QEMU Emulation Verification | Physical Hardware Status |
| :--- | :--- | :--- | :--- |
| **CPU / System** | x86_64 (Intel Core i5/i7/i9, AMD Ryzen) | PASS (`qemu-system-x86_64 -smp 4`) | Certified / Target Baseline |
| **Firmware** | UEFI (OVMF) & Legacy BIOS | PASS (Multiboot2 & AWE Loader) | Certified Baseline |
| **Interrupt Controller** | 8259 PIC / PIT; APIC/IOAPIC pending | Partial (boot initialization exists; dedicated IRQ-delivery assertions pending) | Not validated |
| **Storage (Block)** | VirtIO Block (`virtio-blk-pci`) | PASS (`-drive if=virtio`) | Emulated Execution Proof |
| **Storage (NVMe)** | NVMe 1.3 Controller | PASS (`-device nvme`) | Real HW Target |
| **Storage (AHCI)** | AHCI SATA Controller | PASS (`-device ahci`) | Real HW Target |
| **Network** | VirtIO Network (`virtio-net-pci`) | PASS (`-netdev user,id=n1 -device virtio-net-pci`) | Emulated Execution Proof |
| **Network (Intel)** | Intel e1000 / e1000e PCIe | PASS (`-device e1000`) | Real HW Target |
| **Graphics / Display** | QEMU VBE / Standard VGA Framebuffer | PASS (`-vga std`, AYUI 32bpp) | Emulated Execution Proof |
| **Input Devices** | PS/2 Keyboard & Mouse / USB HID | PASS (`-device usb-kbd`, `-device usb-mouse`) | Certified Baseline |

---

## 2. Multi-Architecture Targets: ARM64 & RISC-V 64

| Architecture | Platform Target | QEMU Execution Status | Image Artifact Generation |
| :--- | :--- | :--- | :--- |
| **ARM64 (aarch64)** | QEMU `virt` machine (`cortex-a53` / `cortex-a72`) | PASS (`qemu-system-aarch64 -M virt`) | `aweos-aarch64.img` |
| **RISC-V 64 (riscv64)** | QEMU `virt` machine (`rv64`) | PASS (`qemu-system-riscv64 -M virt`) | `aweos-riscv64.img` |

---

## 3. Driver Resilience & Fail-Safe Contract

Every native `.asd` driver and kernel hardware subsystem adheres to the mandatory lifecycle contract:
```text
discover -> identify -> probe -> bind -> initialize -> run -> suspend -> resume -> stop -> remove -> recover
```

### Fail-Closed Hardening Guidelines:
1. **DMA & MMIO Boundary Enforcement**: Invalid or out-of-range physical address accesses trigger immediate fail-closed driver isolation.
2. **Interrupt Ownership**: APIC/IOAPIC routing enforcement is pending hardware-router integration; current IDT stubs do not prove interrupt ownership.
3. **Quarantine & Recovery**: Faulty driver loops trigger quarantine state transition and rollback to a staged backup package.
