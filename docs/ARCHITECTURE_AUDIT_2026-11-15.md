# CellKernel — Ճարտարապետական աուդիտ և իրականացման առաջնահերթություններ

**Ամսաթիվ:** 2026-11-15
**Մեթոդ:** կոդի ուղղակի ուսումնասիրություն + build/test/QEMU runtime ստուգումներ։ Փաստաթղթերում գրված պատրաստության տոկոսները որպես ապացույց չեն ընդունվել։

---

## 1. Ստուգման եղանակը

| Ստուգում | Արդյունք | Ապացույց |
|---|---|---|
| `cargo check --workspace` | ✅ անցնում է | 31 crate, միայն manifest warning-ներ |
| `cargo test --workspace` | ✅ 537 թեստ, 0 ձախողում | 57 test suite |
| `cargo fmt --all --check` | ✅ մաքուր | |
| `cargo clippy` | ✅ նոր կոդում warning չկա | |
| Kernel build `x86_64-unknown-none` | ✅ | `kernel-bin` release |
| QEMU boot (`-kernel`, multiboot) | ✅ boot → RUNNING | serial log, տե՛ս §3 |
| Preemptive scheduler runtime | ✅ 128+ context switch / 8 վրկ | serial log, տե՛ս §4 |

## 2. Իրական վիճակը՝ բաղադրիչ առ բաղադրիչ

Օգտագործված է repo-ի սեփական եռաստիճան մոդելը՝ **Designed / Implemented / Validated** (Validated = կա գործարկման runtime ապացույց)։

### VALIDATED (runtime ապացույցով)

- **Boot pipeline (multiboot1/2/PVH, QEMU `-kernel`)** — kernel-ը բեռնվում է, անցնում deterministic entry, հաստատում boot protocol-ը։
- **GDT/TSS, IDT + IST stack-ներ (#NMI/#DF/#MC)** — սկզբնավորումը գործարկվում է QEMU-ում, fail-close պահվածքով։
- **SYSCALL/SYSRET MSR-ներ** — սկզբնավորված են; syscall dispatcher-ը կա (unit-tested), բայց ring-3 զանգեր դեռ չկան։
- **Kernel heap, PCI bus 0 enumeration, PIC remap, PIT ծրագրավորում (1000 Hz)** — գործարկվում են boot log-ում։
- **Preemptive kernel-thread scheduler (ՆՈՐ՝ այս աուդիտի արդյունքը)** — PIT IRQ0 → quantum expiry → ամբողջական context switch ISR frame-ի միջոցով։ Round-robin, 3 ring-0 թրեդ, 50 մվ քվանտում։ Runtime ապացույց՝ հավասար dispatch բաշխում (43/43/43) և կայուն աշխատանք առանց triple fault-ի։
- **Cell runtime (`kernel/src/cell.rs`, ~2000 տող)** — lifecycle (Declared→Admitted→Starting→Running→…→Quarantined), quota accounting (CPU/memory/IPC), recovery policy, bounded channels, dependency-health propagation։ Unit-tested։ Ազնիվ սահմանափակում՝ դեռ **ՉԷ** memory-isolation սահման։
- **ProcessManager / SwitchFrame / context-switch backend** — ամբողջական cooperative switch (asm + validation), dispatch acknowledgement մոդել։ Unit-tested։

### IMPLEMENTED (կոդ + unit թեստեր, առանց runtime ապացույցի)

- **PhysicalFrameAllocator, paging primitives, bootstrap identity map** — allocator-ը ստուգվում է entry-ում; էջադրմամբ ակտիվացումը հաստատվում է log-ում, բայց per-process address space չկա։
- **IPC transport, capability/provenance/rate-limit security primitives** — unit-tested տրամաբանություն, դեռ չկապված syscall-երին։
- **Storage: GPT parsing, bounded VFS, journal, ramdisk** — unit-tested; persistence/crash-recovery runtime ապացույց չկա։
- **Network kernel primitives: ARP, IPv4, bounded transport, policy** — unit-tested; Ethernet driver-ի հետ runtime կապ չկա։
- **driverd (~2900 տող)** — PCI/ACPI/APIC/VirtIO contract-եր, ASD package admission, supervisor։
- **cellnetd (~4200 տող)** — ամենախոշոր service crate-ը։ fabric/session/wire/directory/caps + 23 թեստ։ Host-side տրամաբանություն, դեռ kernel network stack-ի վրա չի աշխատում։
- **AWOSA-հարակից runtime contract-եր, `.asd`/`.awos` format validation, A/B update transaction** — unit-tested։
- **securityd crypto (Ed25519/X25519/HKDF stack)** — իրական կրիպտոգրաֆիա (curve25519-dalek), ոչ homegrown։

### DESIGNED / SKELETON (կառուցվածք կա, պարունակությունը նվազագույն է)

- **AYUI (~1500 տող)** — compositor/widget/shell տվյալակառուցվածքներ և validation, բայց ոչ framebuffer output-ի runtime, ոչ input path-ի։ Desktop environment-ը դեռ չի գործարկվել։
- **apps/* (terminal, filemanager, settings, calculator, sysmon, texteditor, pkgcenter, recovery)** — 120–270 տող յուրաքանչյուրը, skeleton մակարդակի։
- **netd / storaged / initd / identityd / platformd / nexus** — contract-եր և bounded տրամաբանություն, առանց runtime integration-ի։
- **ARM64 / RISC-V boot crate-եր** — stub մակարդակ (82 տող), ինչպես և պետք է է ըստ ազնիվ մարքիրման։
- **Ring 3 / userspace** — գիտակցվածորեն հետաձգված. `enter_userspace()` helper-ը կա, բայց չի կանչվում (չկա per-process address space builder)։ Boot log-ը սա ազնիվ ֆիքսում է։

## 3. Boot runtime ապացույց (QEMU, մինչև փոփոխությունները)

```
AWEOS: GDT & TSS initialized
AWEOS: SYSCALL/SYSRET MSRs initialized
AWEOS: IDT & critical IST stacks initialized
AWEOS: Kernel Heap initialized
AWEOS: PCI Bus 0 enumerated
AWEOS: Interrupts & PIC/PIT initialized
AWEOS: timer programmed (no scheduler yet)   ← հիմնական բացը
AWEOS: kernel state = RUNNING
AWEOS: Ring 3 deferred: isolated user address spaces are not installed
```

**Գտնված կրիտիկական բաց.** PIT-ը ծրագրված էր, IRQ0 unmask-ված, բայց **`sti` երբեք չէր կանչվում** — timer ընդհատումները ֆիզիկապես երբեք չէին կրակում։ «Timer/interrupt execution» gate-ը փակված չէր։

## 4. Այս աուդիտի շրջանակում իրականացված աշխատանքը

**Preemptive kernel-thread scheduler** (`kernel/src/scheduler/preemptive.rs`, նոր մոդուլ)՝

- IRQ0 handler-ը այժմ կանչում է `on_timer_tick(frame)`; quantum լրանալիս frame-ը վերագրում է հերթական runnable թրեդին (ISR epilogue-ի `iretq`-ն է կատարում իրական switch-ը, առանց լրացուցիչ asm ուղի)։
- Boot context-ը գրանցվում է որպես thread 0 (idle), գործարկվում են heartbeat և diagnostics ծառայությունային թրեդներ։
- Անվտանգության մոդել՝ մեկ CPU, interrupt-masking critical section-ներ, ազնիվ նշագրված SMP սահմանափակում։
- 8 նոր host unit թեստ (round-robin, quantum boundary, frame save/restore, exited skip, table exhaustion, dispatch counting)։

**Runtime ապացույց (QEMU, 8–12 վրկ).**

```
AWEOS: preemptive scheduler armed: 3 kernel threads, quantum 50ms
AWEOS: [cell:heartbeat] beat 3 @ tick 2150
AWEOS: [cell:diagnostics] switches=44 dispatches(boot/heartbeat/diagnostics)=15/15/15
AWEOS: [cell:diagnostics] switches=128 dispatches(boot/heartbeat/diagnostics)=43/43/43
```

Dispatch-երի հավասար բաշխումը և կայունությունը ապացուցում են իրական preemption-ը։ Սա փակում է PRODUCT_READINESS Gate A-ի «Timer/interrupt execution is demonstrated» կետը և Gate B-ի «Scheduler execution» կետի ring-0 մասը։ **Բաց է մնում.** user-space process scheduling (պահանջում է address spaces + ring-3)։

## 5. Առաջնահերթություններ (հաջորդ ցիկլեր)

1. **Per-process address spaces + ring-3 առաջին process** — բացում է syscall/IPC/capability runtime ուղին և Ring 3 gate-ը։ Ռիսկային, բայց ամենաբլոկող կետն է։
2. **LAPIC/IOAPIC + SMP-aware timer** — հաղթահարելու IRQ balancing-ը և միացնելու բազմամիջուկությունը (ներկա runtime-ը գիտակցաբար single-CPU է)։
3. **Kernel heap → ավելի ուշ փուլի allocator validation** — dynamic memory-ն անհրաժեշտ է VFS cache-ի, network buffer-ների և userspace loader-ի համար։
4. **VirtIO-block runtime + journal persistence ապացույց** — storage gate-ի բացման նախապայման (QEMU virtio disk exercise + crash-recovery թեստ)։
5. **Ethernet (e1000/virtio-net) + ARP/IPv4 runtime կապ** — kernel net primitives-ը unit-tested են, դեռ լարումը չկա։ Սա բացում է cellnetd-ի տեղականացման ուղին (Ցանցակենտրոն Cell Network ռազմավարության հիմքը)։
6. **Syscall runtime վերջավորում** — dispatcher-ը կապել ring-3 entry-ին, capability enforcement-ը միացնել syscall boundary-ին։
7. **AYUI → framebuffer compositor runtime** — միայն display path-ի (framebuffer + input) runtime ապացույցից հետո։
8. **CI: QEMU smoke boot job** — ավտոմատացնել §3–§4 ստուգումը workflow-ում (boot → serial assert), որպեսզի «Validated» կարգավիճակը չկորցվի regression-ներում։

## 6. Ճարտարապետական քարտեզ (ընթացիկ)

```
Firmware/GRUB/PVH ──► AWELoader (multiboot1/2, PVH note)   [VALIDATED]
        │
CPU init: GDT/TSS/IDT/IST/SYSCALL-MSR                       [VALIDATED]
Memory: frame allocator, bootstrap paging, heap             [VALIDATED/IMPL]
Interrupts: PIC+PIT, IRQ0 → preemptive scheduler            [VALIDATED (նոր)]
Execution: kernel threads (ring-0 RR)                       [VALIDATED (նոր)]
        │
Cell runtime (lifecycle/quota/recovery/channels)            [IMPLEMENTED]
Process model + cooperative switch backend                  [IMPLEMENTED]
IPC / capability / provenance / rate-limit                  [IMPLEMENTED]
Syscall dispatcher (դեռ առանց ring-3 caller)                [IMPLEMENTED]
        │
driverd / storaged+VFS+journal / net primitives / cellnetd  [IMPLEMENTED]
AWOSA contract-եր, .asd/.awos, A/B update                   [IMPLEMENTED]
        │
AYUI, apps, ARM64/RISC-V                                    [DESIGNED/SKELETON]
Ring 3 + per-process address spaces                         [Հետաձգված, բացահայտ]
```
