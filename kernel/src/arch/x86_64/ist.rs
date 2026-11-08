#![no_std]

//! Interrupt Stack Table (IST) support for critical x86_64 vectors.
//!
//! #NMI (vector 2), #DF (vector 8) and #MC (vector 18) must run on dedicated
//! stacks: they can arrive while the kernel stack is corrupted, exhausted, or
//! in an undefined state. Without IST entries, a double fault on a broken
//! kernel stack escalates to a triple fault and an unconditional CPU reset
//! with no diagnostics. This module owns the critical-vector IST assignment
//! and keeps the TSS and IDT sides consistent.
//!
//! This is wiring plus contract validation only: whether the stacks behave
//! correctly under real NMI/#DF/#MC delivery is a QEMU/hardware runtime
//! question and is not claimed by this module.

use super::gdt::Tss64;
use super::idt::Idt;

/// Non-maskable interrupt vector.
pub const NMI_VECTOR: u8 = 2;
/// Double-fault vector.
pub const DOUBLE_FAULT_VECTOR: u8 = 8;
/// Machine-check vector.
pub const MACHINE_CHECK_VECTOR: u8 = 18;

/// IST slot used by the NMI handler.
pub const NMI_IST_INDEX: u8 = 1;
/// IST slot used by the double-fault handler.
pub const DOUBLE_FAULT_IST_INDEX: u8 = 2;
/// IST slot used by the machine-check handler.
pub const MACHINE_CHECK_IST_INDEX: u8 = 3;

/// Size reserved for each critical IST stack. Machine-check and double-fault
/// handlers may need to serialise a lot of state on the stack before halting.
pub const IST_STACK_SIZE: usize = 16 * 1024;

#[repr(C, align(16))]
struct IstStack {
    _bytes: [u8; IST_STACK_SIZE],
}

impl IstStack {
    const EMPTY: IstStack = IstStack {
        _bytes: [0; IST_STACK_SIZE],
    };
}

static mut NMI_STACK: IstStack = IstStack::EMPTY;
static mut DOUBLE_FAULT_STACK: IstStack = IstStack::EMPTY;
static mut MACHINE_CHECK_STACK: IstStack = IstStack::EMPTY;

/// Top-of-stack addresses for the NMI, #DF and #MC IST stacks, in that order.
///
/// The CPU loads these values directly into RSP, so they must point at the
/// upper end of each stack and be 16-byte aligned.
pub fn critical_stack_tops() -> [u64; 3] {
    let bases = [
        core::ptr::addr_of_mut!(NMI_STACK) as usize,
        core::ptr::addr_of_mut!(DOUBLE_FAULT_STACK) as usize,
        core::ptr::addr_of_mut!(MACHINE_CHECK_STACK) as usize,
    ];
    bases.map(|base| (base + IST_STACK_SIZE) as u64)
}

/// Base..top address bounds of the three critical IST stacks, in the same
/// NMI/#DF/#MC order as critical_stack_tops. Runtime consumers (such as the
/// exception-injection self-test) use these to prove that a handler is
/// actually running inside its dedicated stack.
pub fn critical_stack_bounds() -> [(u64, u64); 3] {
    critical_stack_tops().map(|top| (top - IST_STACK_SIZE as u64, top))
}

/// The IST slot assigned to a critical vector, if this vector is managed here.
pub const fn ist_index_for_vector(vector: u8) -> Option<u8> {
    match vector {
        NMI_VECTOR => Some(NMI_IST_INDEX),
        DOUBLE_FAULT_VECTOR => Some(DOUBLE_FAULT_IST_INDEX),
        MACHINE_CHECK_VECTOR => Some(MACHINE_CHECK_IST_INDEX),
        _ => None,
    }
}

/// Assign the dedicated IST stacks for #NMI/#DF/#MC in the TSS and mark the
/// matching IDT entries so the CPU performs the IST stack switch.
///
/// Fails (without partial silent state) if the IDT does not already have
/// handlers installed for the critical vectors or if a TSS slot is rejected.
pub fn configure_ist_stacks(tss: &mut Tss64, idt: &mut Idt) -> Result<(), &'static str> {
    let tops = critical_stack_tops();
    let assignments = [
        (NMI_VECTOR, NMI_IST_INDEX, tops[0]),
        (DOUBLE_FAULT_VECTOR, DOUBLE_FAULT_IST_INDEX, tops[1]),
        (MACHINE_CHECK_VECTOR, MACHINE_CHECK_IST_INDEX, tops[2]),
    ];

    // Preflight every IDT gate and stack address before mutating either
    // structure. This keeps a failed configuration from leaving a partially
    // updated TSS/IDT contract behind.
    for (vector, _, _) in assignments {
        if !idt.is_present(vector) {
            return Err("critical vector handler missing");
        }
    }
    for (_, _, top) in assignments {
        let upper = top >> 48;
        if top == 0 || (top & 0xF) != 0 || (upper != 0 && upper != 0xffff) {
            return Err("critical IST stack top is invalid");
        }
    }

    for (_, index, top) in assignments {
        tss.set_ist(index, top)
            .map_err(|_| "critical IST slot rejected")?;
    }
    for (vector, index, _) in assignments {
        // This cannot fail after the preflight above unless the IDT is
        // concurrently mutated, which is forbidden during single-CPU boot.
        if !idt.set_ist(vector, index) {
            return Err("critical vector IST assignment failed");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    extern "C" fn stub() {}

    #[test]
    fn critical_vectors_map_to_distinct_ist_slots() {
        assert_eq!(ist_index_for_vector(NMI_VECTOR), Some(NMI_IST_INDEX));
        assert_eq!(
            ist_index_for_vector(DOUBLE_FAULT_VECTOR),
            Some(DOUBLE_FAULT_IST_INDEX)
        );
        assert_eq!(
            ist_index_for_vector(MACHINE_CHECK_VECTOR),
            Some(MACHINE_CHECK_IST_INDEX)
        );
        // Three distinct vectors must occupy three distinct IST slots.
        // Checked at compile time per clippy::assertions_on_constants.
        const {
            assert!(NMI_IST_INDEX != DOUBLE_FAULT_IST_INDEX);
            assert!(NMI_IST_INDEX != MACHINE_CHECK_IST_INDEX);
            assert!(DOUBLE_FAULT_IST_INDEX != MACHINE_CHECK_IST_INDEX);
        }
        // Non-critical vectors are not managed by this module.
        assert_eq!(ist_index_for_vector(0), None);
        assert_eq!(ist_index_for_vector(14), None);
        assert_eq!(ist_index_for_vector(32), None);
        assert_eq!(ist_index_for_vector(255), None);
    }

    #[test]
    fn stack_tops_are_aligned_distinct_and_ordered() {
        let tops = critical_stack_tops();
        for top in tops {
            assert!(top != 0, "IST stack top must be non-zero");
            assert_eq!(top & 0xF, 0, "IST stack top must be 16-byte aligned");
        }
        assert_ne!(tops[0], tops[1]);
        assert_ne!(tops[0], tops[2]);
        assert_ne!(tops[1], tops[2]);
    }

    #[test]
    fn configure_wires_tss_and_idt_consistently() {
        let mut tss = Tss64::new();
        let mut idt = Idt::new();
        for vector in 0..=255u8 {
            idt.set_handler(vector, stub as *const () as usize as u64, 0x08);
        }

        assert!(configure_ist_stacks(&mut tss, &mut idt).is_ok());

        let tops = critical_stack_tops();
        assert_eq!(tss.ist(NMI_IST_INDEX), tops[0]);
        assert_eq!(tss.ist(DOUBLE_FAULT_IST_INDEX), tops[1]);
        assert_eq!(tss.ist(MACHINE_CHECK_IST_INDEX), tops[2]);

        assert_eq!(idt.ist_index(NMI_VECTOR), Some(NMI_IST_INDEX));
        assert_eq!(
            idt.ist_index(DOUBLE_FAULT_VECTOR),
            Some(DOUBLE_FAULT_IST_INDEX)
        );
        assert_eq!(
            idt.ist_index(MACHINE_CHECK_VECTOR),
            Some(MACHINE_CHECK_IST_INDEX)
        );

        // Every other installed vector must remain on the interrupted stack.
        for vector in 0..=255u8 {
            if ist_index_for_vector(vector).is_none() {
                assert_eq!(
                    idt.ist_index(vector),
                    None,
                    "vector {vector} must not use an IST slot"
                );
            }
        }
    }

    #[test]
    fn configure_fails_closed_without_critical_handlers() {
        let mut tss = Tss64::new();
        let mut idt = Idt::new();
        // Empty IDT: the critical vectors have no handler to mark.
        assert!(configure_ist_stacks(&mut tss, &mut idt).is_err());
        // Failure must not leave partially assigned TSS or IDT state.
        assert_eq!(tss.ist(NMI_IST_INDEX), 0);
        assert_eq!(tss.ist(DOUBLE_FAULT_IST_INDEX), 0);
        assert_eq!(tss.ist(MACHINE_CHECK_IST_INDEX), 0);
        assert_eq!(idt.ist_index(NMI_VECTOR), None);
        assert_eq!(idt.ist_index(DOUBLE_FAULT_VECTOR), None);
        assert_eq!(idt.ist_index(MACHINE_CHECK_VECTOR), None);
    }

    #[test]
    fn boot_order_contract_configure_after_stub_install() {
        // Boot-order contract for kernel_entry: configure_ist_stacks runs
        // after init_idt_stubs and before Idt::load. Pin the "after" half on
        // the host: without installed handlers it must fail closed, and
        // once stubs are installed it must succeed and leave every critical
        // vector IST-marked.
        let mut tss = Tss64::new();
        let mut idt = Idt::new();
        assert!(
            configure_ist_stacks(&mut tss, &mut idt).is_err(),
            "must fail before handlers are installed"
        );
        for vector in 0..=255u8 {
            idt.set_handler(vector, stub as *const () as usize as u64, 0x08);
        }
        assert!(configure_ist_stacks(&mut tss, &mut idt).is_ok());
        for vector in [NMI_VECTOR, DOUBLE_FAULT_VECTOR, MACHINE_CHECK_VECTOR] {
            assert_eq!(idt.ist_index(vector), ist_index_for_vector(vector));
        }
    }
}
