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
    # System V AMD64: RSP must be 16-byte aligned at the CALL. The target
    # frame's saved RSP is 16n or 16n+8; pad only the 16n case so the callback
    # entry always observes RSP % 16 == 8. r9 is caller-saved and not part of
    # the saved/restore register set.
    xor r9, r9
    test rsp, 8
    jnz .Lcallback_push
    push rax                  # Alignment pad, removed after the callback.
    mov r9, 1
.Lcallback_push:
    push rax                  # Preserve target RIP and align RSP for CALL.
    mov rdi, rcx
    mov rsi, r8
    call rdx
    pop rax
    test r9, r9
    jz .Lcallback_done
    add rsp, 8
.Lcallback_done:
    jmp rax
.Ldispatch:
    jmp rax
.Lresume:
    ret
.Linvalid:
    ud2
.size awe_x86_64_switch, .-awe_x86_64_switch
.att_syntax prefix
