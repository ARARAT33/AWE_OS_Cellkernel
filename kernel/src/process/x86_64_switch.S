.intel_syntax noprefix
.section .text
.global awe_x86_64_switch
.type awe_x86_64_switch,@function
# void awe_x86_64_switch(SwitchFrame *current, const SwitchFrame *next)
# System V AMD64 ABI: RDI=current, RSI=next.
# Frame: rbx, rbp, r12, r13, r14, r15, rsp, rip.
awe_x86_64_switch:
    test rdi, rdi
    jz .Linvalid
    test rsi, rsi
    jz .Linvalid
    mov [rdi + 0], rbx
    mov [rdi + 8], rbp
    mov [rdi + 16], r12
    mov [rdi + 24], r13
    mov [rdi + 32], r14
    mov [rdi + 40], r15
    mov [rdi + 48], rsp
    lea rax, [rip + .Lresume]
    mov [rdi + 56], rax
    mov rax, [rsi + 56]
    mov rdx, [rsi + 64]       # Optional target-stack completion callback
    mov rcx, [rsi + 72]       # Opaque callback context
    mov r8,  [rsi + 80]       # Opaque callback argument
    mov rbx, [rsi + 0]
    mov rbp, [rsi + 8]
    mov r12, [rsi + 16]
    mov r13, [rsi + 24]
    mov r14, [rsi + 32]
    mov r15, [rsi + 40]
    mov rsp, [rsi + 48]

    # All target-frame fields are loaded before changing stacks. If present,
    # the callback runs on the new stack and must return before target RIP.
    test rdx, rdx
    jz .Ldispatch
    push rax                  # Preserve target RIP and align RSP for CALL.
    mov rdi, rcx
    mov rsi, r8
    call rdx
    pop rax
.Ldispatch:
    jmp rax
.Lresume:
    ret
.Linvalid:
    ud2
.size awe_x86_64_switch, .-awe_x86_64_switch
.att_syntax prefix
