.intel_syntax noprefix
.section .text
.global awe_syscall_entry
.type awe_syscall_entry, @function

# Minimal single-CPU bootstrap syscall entry.
# SYSCALL does not switch stacks, so save the user RSP and move to the
# kernel stack configured during early boot. Interrupts are masked by FMASK.
# This entry is intentionally a bring-up path; per-CPU state and full
# address-space isolation are required before SMP or untrusted applications.
awe_syscall_entry:
    mov qword ptr [rip + AWE_SYSCALL_USER_RSP], rsp
    mov rsp, qword ptr [rip + AWE_SYSCALL_KERNEL_STACK_TOP]
    and rsp, -16

    # Save SYSRET return state and preserve user registers across the Rust ABI.
    push r11
    push rcx
    push rbx
    push rbp
    push r12
    push r13
    push r14
    push r15
    push rdi
    push rsi
    push rdx
    push r10
    push r8
    push r9

    mov rdi, rax
    call awe_syscall_dispatch_entry

    pop r9
    pop r8
    pop r10
    pop rdx
    pop rsi
    pop rdi
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbp
    pop rbx
    pop rcx
    pop r11

    mov rsp, qword ptr [rip + AWE_SYSCALL_USER_RSP]
    sysretq

.size awe_syscall_entry, .-awe_syscall_entry
.att_syntax prefix
