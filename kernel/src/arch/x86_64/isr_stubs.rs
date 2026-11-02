#![no_std]
#![allow(bad_asm_style)]

use super::interrupts::pic_send_eoi;
use super::isr::InterruptFrame;
use super::serial_write_str;
use core::arch::global_asm;
use core::sync::atomic::{AtomicU64, Ordering};

static TIMER_IRQ_COUNT: AtomicU64 = AtomicU64::new(0);

global_asm!(
    r#".intel_syntax noprefix
.section .text
.global awe_isr_common
.type awe_isr_common, @function
awe_isr_common:
    push rax
    push rbx
    push rcx
    push rdx
    push rbp
    push rsi
    push rdi
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15

    // Keep the original frame pointer in a callee-saved register.
    // The Rust ABI requires RSP to be 16-byte aligned immediately before CALL.
    mov rbx, rsp
    mov rdi, rsp
    and rsp, -16
    cld
    call awe_interrupt_handler
    mov rsp, rbx

    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rdi
    pop rsi
    pop rbp
    pop rdx
    pop rcx
    pop rbx
    pop rax

    add rsp, 16
    iretq

.macro ISR_NOERR num
.global awe_isr_\num
awe_isr_\num:
    push 0
    push \num
    jmp awe_isr_common
.endm

.macro ISR_ERR num
.global awe_isr_\num
awe_isr_\num:
    push \num
    jmp awe_isr_common
.endm

// CPU Exceptions (0-31)
ISR_NOERR 0   // Divide-by-zero
ISR_NOERR 1   // Debug
ISR_NOERR 2   // Non-maskable Interrupt
ISR_NOERR 3   // Breakpoint
ISR_NOERR 4   // Overflow
ISR_NOERR 5   // Bound Range Exceeded
ISR_NOERR 6   // Invalid Opcode
ISR_NOERR 7   // Device Not Available
ISR_ERR   8   // Double Fault
ISR_NOERR 9   // Coprocessor Segment Overrun
ISR_ERR   10  // Invalid TSS
ISR_ERR   11  // Segment Not Present
ISR_ERR   12  // Stack Segment Fault
ISR_ERR   13  // General Protection Fault
ISR_ERR   14  // Page Fault
ISR_NOERR 15  // Reserved
ISR_NOERR 16  // x87 Floating-Point Exception
ISR_ERR   17  // Alignment Check
ISR_NOERR 18  // Machine Check
ISR_NOERR 19  // SIMD Floating-Point Exception
ISR_NOERR 20  // Virtualization Exception
ISR_ERR   21  // Control Protection (#CP)
ISR_NOERR 22  // Reserved
ISR_NOERR 23  // Reserved
ISR_NOERR 24  // Reserved
ISR_NOERR 25  // Reserved
ISR_NOERR 26  // Reserved
ISR_NOERR 27  // Reserved
ISR_NOERR 28  // Hypervisor Injection Exception
ISR_ERR   29  // VMM Communication Exception (#VC)
ISR_ERR   30  // Security Exception
ISR_NOERR 31  // Reserved

// IRQs and other interrupts (32-255)
ISR_NOERR 32  // PIT Timer (IRQ0)
ISR_NOERR 33  // Keyboard (IRQ1)
ISR_NOERR 34  // Cascaded PIC (IRQ2)
ISR_NOERR 35  // Serial Port 2 (IRQ3)
ISR_NOERR 36  // Serial Port 1 (IRQ4)
ISR_NOERR 37  // Parallel Port 2/3 (IRQ5)
ISR_NOERR 38  // Floppy Disk (IRQ6)
ISR_NOERR 39  // Parallel Port 1 (IRQ7)
ISR_NOERR 40  // CMOS RTC (IRQ8)
ISR_NOERR 41  // Reserved (IRQ9)
ISR_NOERR 42  // Reserved (IRQ10)
ISR_NOERR 43  // Reserved (IRQ11)
ISR_NOERR 44  // PS/2 Mouse (IRQ12)
ISR_NOERR 45  // FPU (IRQ13)
ISR_NOERR 46  // Primary ATA (IRQ14)
ISR_NOERR 47  // Secondary ATA (IRQ15)

// Additional IRQs for APIC (48-255)
ISR_NOERR 48
ISR_NOERR 49
ISR_NOERR 50
ISR_NOERR 51
ISR_NOERR 52
ISR_NOERR 53
ISR_NOERR 54
ISR_NOERR 55
ISR_NOERR 56
ISR_NOERR 57
ISR_NOERR 58
ISR_NOERR 59
ISR_NOERR 60
ISR_NOERR 61
ISR_NOERR 62
ISR_NOERR 63
ISR_NOERR 64
ISR_NOERR 65
ISR_NOERR 66
ISR_NOERR 67
ISR_NOERR 68
ISR_NOERR 69
ISR_NOERR 70
ISR_NOERR 71
ISR_NOERR 72
ISR_NOERR 73
ISR_NOERR 74
ISR_NOERR 75
ISR_NOERR 76
ISR_NOERR 77
ISR_NOERR 78
ISR_NOERR 79
ISR_NOERR 80
ISR_NOERR 81
ISR_NOERR 82
ISR_NOERR 83
ISR_NOERR 84
ISR_NOERR 85
ISR_NOERR 86
ISR_NOERR 87
ISR_NOERR 88
ISR_NOERR 89
ISR_NOERR 90
ISR_NOERR 91
ISR_NOERR 92
ISR_NOERR 93
ISR_NOERR 94
ISR_NOERR 95
ISR_NOERR 96
ISR_NOERR 97
ISR_NOERR 98
ISR_NOERR 99
ISR_NOERR 100
ISR_NOERR 101
ISR_NOERR 102
ISR_NOERR 103
ISR_NOERR 104
ISR_NOERR 105
ISR_NOERR 106
ISR_NOERR 107
ISR_NOERR 108
ISR_NOERR 109
ISR_NOERR 110
ISR_NOERR 111
ISR_NOERR 112
ISR_NOERR 113
ISR_NOERR 114
ISR_NOERR 115
ISR_NOERR 116
ISR_NOERR 117
ISR_NOERR 118
ISR_NOERR 119
ISR_NOERR 120
ISR_NOERR 121
ISR_NOERR 122
ISR_NOERR 123
ISR_NOERR 124
ISR_NOERR 125
ISR_NOERR 126
ISR_NOERR 127
ISR_NOERR 128
ISR_NOERR 129
ISR_NOERR 130
ISR_NOERR 131
ISR_NOERR 132
ISR_NOERR 133
ISR_NOERR 134
ISR_NOERR 135
ISR_NOERR 136
ISR_NOERR 137
ISR_NOERR 138
ISR_NOERR 139
ISR_NOERR 140
ISR_NOERR 141
ISR_NOERR 142
ISR_NOERR 143
ISR_NOERR 144
ISR_NOERR 145
ISR_NOERR 146
ISR_NOERR 147
ISR_NOERR 148
ISR_NOERR 149
ISR_NOERR 150
ISR_NOERR 151
ISR_NOERR 152
ISR_NOERR 153
ISR_NOERR 154
ISR_NOERR 155
ISR_NOERR 156
ISR_NOERR 157
ISR_NOERR 158
ISR_NOERR 159
ISR_NOERR 160
ISR_NOERR 161
ISR_NOERR 162
ISR_NOERR 163
ISR_NOERR 164
ISR_NOERR 165
ISR_NOERR 166
ISR_NOERR 167
ISR_NOERR 168
ISR_NOERR 169
ISR_NOERR 170
ISR_NOERR 171
ISR_NOERR 172
ISR_NOERR 173
ISR_NOERR 174
ISR_NOERR 175
ISR_NOERR 176
ISR_NOERR 177
ISR_NOERR 178
ISR_NOERR 179
ISR_NOERR 180
ISR_NOERR 181
ISR_NOERR 182
ISR_NOERR 183
ISR_NOERR 184
ISR_NOERR 185
ISR_NOERR 186
ISR_NOERR 187
ISR_NOERR 188
ISR_NOERR 189
ISR_NOERR 190
ISR_NOERR 191
ISR_NOERR 192
ISR_NOERR 193
ISR_NOERR 194
ISR_NOERR 195
ISR_NOERR 196
ISR_NOERR 197
ISR_NOERR 198
ISR_NOERR 199
ISR_NOERR 200
ISR_NOERR 201
ISR_NOERR 202
ISR_NOERR 203
ISR_NOERR 204
ISR_NOERR 205
ISR_NOERR 206
ISR_NOERR 207
ISR_NOERR 208
ISR_NOERR 209
ISR_NOERR 210
ISR_NOERR 211
ISR_NOERR 212
ISR_NOERR 213
ISR_NOERR 214
ISR_NOERR 215
ISR_NOERR 216
ISR_NOERR 217
ISR_NOERR 218
ISR_NOERR 219
ISR_NOERR 220
ISR_NOERR 221
ISR_NOERR 222
ISR_NOERR 223
ISR_NOERR 224
ISR_NOERR 225
ISR_NOERR 226
ISR_NOERR 227
ISR_NOERR 228
ISR_NOERR 229
ISR_NOERR 230
ISR_NOERR 231
ISR_NOERR 232
ISR_NOERR 233
ISR_NOERR 234
ISR_NOERR 235
ISR_NOERR 236
ISR_NOERR 237
ISR_NOERR 238
ISR_NOERR 239
ISR_NOERR 240
ISR_NOERR 241
ISR_NOERR 242
ISR_NOERR 243
ISR_NOERR 244
ISR_NOERR 245
ISR_NOERR 246
ISR_NOERR 247
ISR_NOERR 248
ISR_NOERR 249
ISR_NOERR 250
ISR_NOERR 251
ISR_NOERR 252
ISR_NOERR 253
ISR_NOERR 254
ISR_NOERR 255

.att_syntax prefix
"#
);

// Auto-generate extern declarations for all ISRs
unsafe extern "C" {
    pub fn awe_isr_0();
    pub fn awe_isr_1();
    pub fn awe_isr_2();
    pub fn awe_isr_3();
    pub fn awe_isr_4();
    pub fn awe_isr_5();
    pub fn awe_isr_6();
    pub fn awe_isr_7();
    pub fn awe_isr_8();
    pub fn awe_isr_9();
    pub fn awe_isr_10();
    pub fn awe_isr_11();
    pub fn awe_isr_12();
    pub fn awe_isr_13();
    pub fn awe_isr_14();
    pub fn awe_isr_15();
    pub fn awe_isr_16();
    pub fn awe_isr_17();
    pub fn awe_isr_18();
    pub fn awe_isr_19();
    pub fn awe_isr_20();
    pub fn awe_isr_21();
    pub fn awe_isr_22();
    pub fn awe_isr_23();
    pub fn awe_isr_24();
    pub fn awe_isr_25();
    pub fn awe_isr_26();
    pub fn awe_isr_27();
    pub fn awe_isr_28();
    pub fn awe_isr_29();
    pub fn awe_isr_30();
    pub fn awe_isr_31();
    pub fn awe_isr_32();
    pub fn awe_isr_33();
    pub fn awe_isr_34();
    pub fn awe_isr_35();
    pub fn awe_isr_36();
    pub fn awe_isr_37();
    pub fn awe_isr_38();
    pub fn awe_isr_39();
    pub fn awe_isr_40();
    pub fn awe_isr_41();
    pub fn awe_isr_42();
    pub fn awe_isr_43();
    pub fn awe_isr_44();
    pub fn awe_isr_45();
    pub fn awe_isr_46();
    pub fn awe_isr_47();
    pub fn awe_isr_48();
    pub fn awe_isr_49();
    pub fn awe_isr_50();
    pub fn awe_isr_51();
    pub fn awe_isr_52();
    pub fn awe_isr_53();
    pub fn awe_isr_54();
    pub fn awe_isr_55();
    pub fn awe_isr_56();
    pub fn awe_isr_57();
    pub fn awe_isr_58();
    pub fn awe_isr_59();
    pub fn awe_isr_60();
    pub fn awe_isr_61();
    pub fn awe_isr_62();
    pub fn awe_isr_63();
    pub fn awe_isr_64();
    pub fn awe_isr_65();
    pub fn awe_isr_66();
    pub fn awe_isr_67();
    pub fn awe_isr_68();
    pub fn awe_isr_69();
    pub fn awe_isr_70();
    pub fn awe_isr_71();
    pub fn awe_isr_72();
    pub fn awe_isr_73();
    pub fn awe_isr_74();
    pub fn awe_isr_75();
    pub fn awe_isr_76();
    pub fn awe_isr_77();
    pub fn awe_isr_78();
    pub fn awe_isr_79();
    pub fn awe_isr_80();
    pub fn awe_isr_81();
    pub fn awe_isr_82();
    pub fn awe_isr_83();
    pub fn awe_isr_84();
    pub fn awe_isr_85();
    pub fn awe_isr_86();
    pub fn awe_isr_87();
    pub fn awe_isr_88();
    pub fn awe_isr_89();
    pub fn awe_isr_90();
    pub fn awe_isr_91();
    pub fn awe_isr_92();
    pub fn awe_isr_93();
    pub fn awe_isr_94();
    pub fn awe_isr_95();
    pub fn awe_isr_96();
    pub fn awe_isr_97();
    pub fn awe_isr_98();
    pub fn awe_isr_99();
    pub fn awe_isr_100();
    pub fn awe_isr_101();
    pub fn awe_isr_102();
    pub fn awe_isr_103();
    pub fn awe_isr_104();
    pub fn awe_isr_105();
    pub fn awe_isr_106();
    pub fn awe_isr_107();
    pub fn awe_isr_108();
    pub fn awe_isr_109();
    pub fn awe_isr_110();
    pub fn awe_isr_111();
    pub fn awe_isr_112();
    pub fn awe_isr_113();
    pub fn awe_isr_114();
    pub fn awe_isr_115();
    pub fn awe_isr_116();
    pub fn awe_isr_117();
    pub fn awe_isr_118();
    pub fn awe_isr_119();
    pub fn awe_isr_120();
    pub fn awe_isr_121();
    pub fn awe_isr_122();
    pub fn awe_isr_123();
    pub fn awe_isr_124();
    pub fn awe_isr_125();
    pub fn awe_isr_126();
    pub fn awe_isr_127();
    pub fn awe_isr_128();
    pub fn awe_isr_129();
    pub fn awe_isr_130();
    pub fn awe_isr_131();
    pub fn awe_isr_132();
    pub fn awe_isr_133();
    pub fn awe_isr_134();
    pub fn awe_isr_135();
    pub fn awe_isr_136();
    pub fn awe_isr_137();
    pub fn awe_isr_138();
    pub fn awe_isr_139();
    pub fn awe_isr_140();
    pub fn awe_isr_141();
    pub fn awe_isr_142();
    pub fn awe_isr_143();
    pub fn awe_isr_144();
    pub fn awe_isr_145();
    pub fn awe_isr_146();
    pub fn awe_isr_147();
    pub fn awe_isr_148();
    pub fn awe_isr_149();
    pub fn awe_isr_150();
    pub fn awe_isr_151();
    pub fn awe_isr_152();
    pub fn awe_isr_153();
    pub fn awe_isr_154();
    pub fn awe_isr_155();
    pub fn awe_isr_156();
    pub fn awe_isr_157();
    pub fn awe_isr_158();
    pub fn awe_isr_159();
    pub fn awe_isr_160();
    pub fn awe_isr_161();
    pub fn awe_isr_162();
    pub fn awe_isr_163();
    pub fn awe_isr_164();
    pub fn awe_isr_165();
    pub fn awe_isr_166();
    pub fn awe_isr_167();
    pub fn awe_isr_168();
    pub fn awe_isr_169();
    pub fn awe_isr_170();
    pub fn awe_isr_171();
    pub fn awe_isr_172();
    pub fn awe_isr_173();
    pub fn awe_isr_174();
    pub fn awe_isr_175();
    pub fn awe_isr_176();
    pub fn awe_isr_177();
    pub fn awe_isr_178();
    pub fn awe_isr_179();
    pub fn awe_isr_180();
    pub fn awe_isr_181();
    pub fn awe_isr_182();
    pub fn awe_isr_183();
    pub fn awe_isr_184();
    pub fn awe_isr_185();
    pub fn awe_isr_186();
    pub fn awe_isr_187();
    pub fn awe_isr_188();
    pub fn awe_isr_189();
    pub fn awe_isr_190();
    pub fn awe_isr_191();
    pub fn awe_isr_192();
    pub fn awe_isr_193();
    pub fn awe_isr_194();
    pub fn awe_isr_195();
    pub fn awe_isr_196();
    pub fn awe_isr_197();
    pub fn awe_isr_198();
    pub fn awe_isr_199();
    pub fn awe_isr_200();
    pub fn awe_isr_201();
    pub fn awe_isr_202();
    pub fn awe_isr_203();
    pub fn awe_isr_204();
    pub fn awe_isr_205();
    pub fn awe_isr_206();
    pub fn awe_isr_207();
    pub fn awe_isr_208();
    pub fn awe_isr_209();
    pub fn awe_isr_210();
    pub fn awe_isr_211();
    pub fn awe_isr_212();
    pub fn awe_isr_213();
    pub fn awe_isr_214();
    pub fn awe_isr_215();
    pub fn awe_isr_216();
    pub fn awe_isr_217();
    pub fn awe_isr_218();
    pub fn awe_isr_219();
    pub fn awe_isr_220();
    pub fn awe_isr_221();
    pub fn awe_isr_222();
    pub fn awe_isr_223();
    pub fn awe_isr_224();
    pub fn awe_isr_225();
    pub fn awe_isr_226();
    pub fn awe_isr_227();
    pub fn awe_isr_228();
    pub fn awe_isr_229();
    pub fn awe_isr_230();
    pub fn awe_isr_231();
    pub fn awe_isr_232();
    pub fn awe_isr_233();
    pub fn awe_isr_234();
    pub fn awe_isr_235();
    pub fn awe_isr_236();
    pub fn awe_isr_237();
    pub fn awe_isr_238();
    pub fn awe_isr_239();
    pub fn awe_isr_240();
    pub fn awe_isr_241();
    pub fn awe_isr_242();
    pub fn awe_isr_243();
    pub fn awe_isr_244();
    pub fn awe_isr_245();
    pub fn awe_isr_246();
    pub fn awe_isr_247();
    pub fn awe_isr_248();
    pub fn awe_isr_249();
    pub fn awe_isr_250();
    pub fn awe_isr_251();
    pub fn awe_isr_252();
    pub fn awe_isr_253();
    pub fn awe_isr_254();
    pub fn awe_isr_255();
}

pub fn init_idt_stubs(idt: &mut super::idt::Idt, cs: u16) {
    // Initialize all 256 IDT entries
    for i in 0..=255u8 {
        // Get the function pointer dynamically using a match
        // This is safe because we have all the ISR functions defined
        let handler = match i {
            0 => awe_isr_0 as *const (),
            1 => awe_isr_1 as *const (),
            2 => awe_isr_2 as *const (),
            3 => awe_isr_3 as *const (),
            4 => awe_isr_4 as *const (),
            5 => awe_isr_5 as *const (),
            6 => awe_isr_6 as *const (),
            7 => awe_isr_7 as *const (),
            8 => awe_isr_8 as *const (),
            9 => awe_isr_9 as *const (),
            10 => awe_isr_10 as *const (),
            11 => awe_isr_11 as *const (),
            12 => awe_isr_12 as *const (),
            13 => awe_isr_13 as *const (),
            14 => awe_isr_14 as *const (),
            15 => awe_isr_15 as *const (),
            16 => awe_isr_16 as *const (),
            17 => awe_isr_17 as *const (),
            18 => awe_isr_18 as *const (),
            19 => awe_isr_19 as *const (),
            20 => awe_isr_20 as *const (),
            21 => awe_isr_21 as *const (),
            22 => awe_isr_22 as *const (),
            23 => awe_isr_23 as *const (),
            24 => awe_isr_24 as *const (),
            25 => awe_isr_25 as *const (),
            26 => awe_isr_26 as *const (),
            27 => awe_isr_27 as *const (),
            28 => awe_isr_28 as *const (),
            29 => awe_isr_29 as *const (),
            30 => awe_isr_30 as *const (),
            31 => awe_isr_31 as *const (),
            32 => awe_isr_32 as *const (),
            33 => awe_isr_33 as *const (),
            34 => awe_isr_34 as *const (),
            35 => awe_isr_35 as *const (),
            36 => awe_isr_36 as *const (),
            37 => awe_isr_37 as *const (),
            38 => awe_isr_38 as *const (),
            39 => awe_isr_39 as *const (),
            40 => awe_isr_40 as *const (),
            41 => awe_isr_41 as *const (),
            42 => awe_isr_42 as *const (),
            43 => awe_isr_43 as *const (),
            44 => awe_isr_44 as *const (),
            45 => awe_isr_45 as *const (),
            46 => awe_isr_46 as *const (),
            47 => awe_isr_47 as *const (),
            48 => awe_isr_48 as *const (),
            49 => awe_isr_49 as *const (),
            50 => awe_isr_50 as *const (),
            51 => awe_isr_51 as *const (),
            52 => awe_isr_52 as *const (),
            53 => awe_isr_53 as *const (),
            54 => awe_isr_54 as *const (),
            55 => awe_isr_55 as *const (),
            56 => awe_isr_56 as *const (),
            57 => awe_isr_57 as *const (),
            58 => awe_isr_58 as *const (),
            59 => awe_isr_59 as *const (),
            60 => awe_isr_60 as *const (),
            61 => awe_isr_61 as *const (),
            62 => awe_isr_62 as *const (),
            63 => awe_isr_63 as *const (),
            64 => awe_isr_64 as *const (),
            65 => awe_isr_65 as *const (),
            66 => awe_isr_66 as *const (),
            67 => awe_isr_67 as *const (),
            68 => awe_isr_68 as *const (),
            69 => awe_isr_69 as *const (),
            70 => awe_isr_70 as *const (),
            71 => awe_isr_71 as *const (),
            72 => awe_isr_72 as *const (),
            73 => awe_isr_73 as *const (),
            74 => awe_isr_74 as *const (),
            75 => awe_isr_75 as *const (),
            76 => awe_isr_76 as *const (),
            77 => awe_isr_77 as *const (),
            78 => awe_isr_78 as *const (),
            79 => awe_isr_79 as *const (),
            80 => awe_isr_80 as *const (),
            81 => awe_isr_81 as *const (),
            82 => awe_isr_82 as *const (),
            83 => awe_isr_83 as *const (),
            84 => awe_isr_84 as *const (),
            85 => awe_isr_85 as *const (),
            86 => awe_isr_86 as *const (),
            87 => awe_isr_87 as *const (),
            88 => awe_isr_88 as *const (),
            89 => awe_isr_89 as *const (),
            90 => awe_isr_90 as *const (),
            91 => awe_isr_91 as *const (),
            92 => awe_isr_92 as *const (),
            93 => awe_isr_93 as *const (),
            94 => awe_isr_94 as *const (),
            95 => awe_isr_95 as *const (),
            96 => awe_isr_96 as *const (),
            97 => awe_isr_97 as *const (),
            98 => awe_isr_98 as *const (),
            99 => awe_isr_99 as *const (),
            100 => awe_isr_100 as *const (),
            101 => awe_isr_101 as *const (),
            102 => awe_isr_102 as *const (),
            103 => awe_isr_103 as *const (),
            104 => awe_isr_104 as *const (),
            105 => awe_isr_105 as *const (),
            106 => awe_isr_106 as *const (),
            107 => awe_isr_107 as *const (),
            108 => awe_isr_108 as *const (),
            109 => awe_isr_109 as *const (),
            110 => awe_isr_110 as *const (),
            111 => awe_isr_111 as *const (),
            112 => awe_isr_112 as *const (),
            113 => awe_isr_113 as *const (),
            114 => awe_isr_114 as *const (),
            115 => awe_isr_115 as *const (),
            116 => awe_isr_116 as *const (),
            117 => awe_isr_117 as *const (),
            118 => awe_isr_118 as *const (),
            119 => awe_isr_119 as *const (),
            120 => awe_isr_120 as *const (),
            121 => awe_isr_121 as *const (),
            122 => awe_isr_122 as *const (),
            123 => awe_isr_123 as *const (),
            124 => awe_isr_124 as *const (),
            125 => awe_isr_125 as *const (),
            126 => awe_isr_126 as *const (),
            127 => awe_isr_127 as *const (),
            128 => awe_isr_128 as *const (),
            129 => awe_isr_129 as *const (),
            130 => awe_isr_130 as *const (),
            131 => awe_isr_131 as *const (),
            132 => awe_isr_132 as *const (),
            133 => awe_isr_133 as *const (),
            134 => awe_isr_134 as *const (),
            135 => awe_isr_135 as *const (),
            136 => awe_isr_136 as *const (),
            137 => awe_isr_137 as *const (),
            138 => awe_isr_138 as *const (),
            139 => awe_isr_139 as *const (),
            140 => awe_isr_140 as *const (),
            141 => awe_isr_141 as *const (),
            142 => awe_isr_142 as *const (),
            143 => awe_isr_143 as *const (),
            144 => awe_isr_144 as *const (),
            145 => awe_isr_145 as *const (),
            146 => awe_isr_146 as *const (),
            147 => awe_isr_147 as *const (),
            148 => awe_isr_148 as *const (),
            149 => awe_isr_149 as *const (),
            150 => awe_isr_150 as *const (),
            151 => awe_isr_151 as *const (),
            152 => awe_isr_152 as *const (),
            153 => awe_isr_153 as *const (),
            154 => awe_isr_154 as *const (),
            155 => awe_isr_155 as *const (),
            156 => awe_isr_156 as *const (),
            157 => awe_isr_157 as *const (),
            158 => awe_isr_158 as *const (),
            159 => awe_isr_159 as *const (),
            160 => awe_isr_160 as *const (),
            161 => awe_isr_161 as *const (),
            162 => awe_isr_162 as *const (),
            163 => awe_isr_163 as *const (),
            164 => awe_isr_164 as *const (),
            165 => awe_isr_165 as *const (),
            166 => awe_isr_166 as *const (),
            167 => awe_isr_167 as *const (),
            168 => awe_isr_168 as *const (),
            169 => awe_isr_169 as *const (),
            170 => awe_isr_170 as *const (),
            171 => awe_isr_171 as *const (),
            172 => awe_isr_172 as *const (),
            173 => awe_isr_173 as *const (),
            174 => awe_isr_174 as *const (),
            175 => awe_isr_175 as *const (),
            176 => awe_isr_176 as *const (),
            177 => awe_isr_177 as *const (),
            178 => awe_isr_178 as *const (),
            179 => awe_isr_179 as *const (),
            180 => awe_isr_180 as *const (),
            181 => awe_isr_181 as *const (),
            182 => awe_isr_182 as *const (),
            183 => awe_isr_183 as *const (),
            184 => awe_isr_184 as *const (),
            185 => awe_isr_185 as *const (),
            186 => awe_isr_186 as *const (),
            187 => awe_isr_187 as *const (),
            188 => awe_isr_188 as *const (),
            189 => awe_isr_189 as *const (),
            190 => awe_isr_190 as *const (),
            191 => awe_isr_191 as *const (),
            192 => awe_isr_192 as *const (),
            193 => awe_isr_193 as *const (),
            194 => awe_isr_194 as *const (),
            195 => awe_isr_195 as *const (),
            196 => awe_isr_196 as *const (),
            197 => awe_isr_197 as *const (),
            198 => awe_isr_198 as *const (),
            199 => awe_isr_199 as *const (),
            200 => awe_isr_200 as *const (),
            201 => awe_isr_201 as *const (),
            202 => awe_isr_202 as *const (),
            203 => awe_isr_203 as *const (),
            204 => awe_isr_204 as *const (),
            205 => awe_isr_205 as *const (),
            206 => awe_isr_206 as *const (),
            207 => awe_isr_207 as *const (),
            208 => awe_isr_208 as *const (),
            209 => awe_isr_209 as *const (),
            210 => awe_isr_210 as *const (),
            211 => awe_isr_211 as *const (),
            212 => awe_isr_212 as *const (),
            213 => awe_isr_213 as *const (),
            214 => awe_isr_214 as *const (),
            215 => awe_isr_215 as *const (),
            216 => awe_isr_216 as *const (),
            217 => awe_isr_217 as *const (),
            218 => awe_isr_218 as *const (),
            219 => awe_isr_219 as *const (),
            220 => awe_isr_220 as *const (),
            221 => awe_isr_221 as *const (),
            222 => awe_isr_222 as *const (),
            223 => awe_isr_223 as *const (),
            224 => awe_isr_224 as *const (),
            225 => awe_isr_225 as *const (),
            226 => awe_isr_226 as *const (),
            227 => awe_isr_227 as *const (),
            228 => awe_isr_228 as *const (),
            229 => awe_isr_229 as *const (),
            230 => awe_isr_230 as *const (),
            231 => awe_isr_231 as *const (),
            232 => awe_isr_232 as *const (),
            233 => awe_isr_233 as *const (),
            234 => awe_isr_234 as *const (),
            235 => awe_isr_235 as *const (),
            236 => awe_isr_236 as *const (),
            237 => awe_isr_237 as *const (),
            238 => awe_isr_238 as *const (),
            239 => awe_isr_239 as *const (),
            240 => awe_isr_240 as *const (),
            241 => awe_isr_241 as *const (),
            242 => awe_isr_242 as *const (),
            243 => awe_isr_243 as *const (),
            244 => awe_isr_244 as *const (),
            245 => awe_isr_245 as *const (),
            246 => awe_isr_246 as *const (),
            247 => awe_isr_247 as *const (),
            248 => awe_isr_248 as *const (),
            249 => awe_isr_249 as *const (),
            250 => awe_isr_250 as *const (),
            251 => awe_isr_251 as *const (),
            252 => awe_isr_252 as *const (),
            253 => awe_isr_253 as *const (),
            254 => awe_isr_254 as *const (),
            255 => awe_isr_255 as *const (),
        };
        idt.set_handler(i, handler as usize as u64, cs);
    }
}

fn print_u64_hex(mut val: u64) {
    let hex = b"0123456789ABCDEF";
    let mut buf = [b'0'; 18];
    buf[0] = b'0';
    buf[1] = b'x';
    for i in (2..18).rev() {
        buf[i] = hex[(val & 0xF) as usize];
        val >>= 4;
    }
    for b in buf {
        super::serial_write_byte(b);
    }
}

fn print_stack_context(frame: &InterruptFrame) {
    serial_write_str("  RSP: 0x");
    print_u64_hex(frame.interrupted_stack_pointer());
    if let Some(ss) = frame.interrupted_stack_segment() {
        serial_write_str(" SS: 0x");
        print_u64_hex(ss);
    }
    serial_write_str("\r\n");
}

fn halt_after_exception() -> ! {
    loop {
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}

// Exception names for debugging
const EXCEPTION_NAMES: [&str; 32] = [
    "Divide-by-zero",
    "Debug",
    "Non-maskable Interrupt",
    "Breakpoint",
    "Overflow",
    "Bound Range Exceeded",
    "Invalid Opcode",
    "Device Not Available",
    "Double Fault",
    "Coprocessor Segment Overrun",
    "Invalid TSS",
    "Segment Not Present",
    "Stack Segment Fault",
    "General Protection Fault",
    "Page Fault",
    "Reserved",
    "x87 Floating-Point Exception",
    "Alignment Check",
    "Machine Check",
    "SIMD Floating-Point Exception",
    "Virtualization Exception",
    "Control Protection",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
    "Hypervisor Injection Exception",
    "VMM Communication Exception",
    "Security Exception",
    "Reserved",
];

/// Handle divide error (#DE, vector 0). It has no CPU error-code word.
fn handle_divide_error(frame: &InterruptFrame) -> ! {
    serial_write_str("AWEOS: Divide Error (#DE)\r\n");
    serial_write_str("  RIP: 0x");
    print_u64_hex(frame.rip);
    serial_write_str("\r\n");
    print_stack_context(frame);
    halt_after_exception()
}

/// Handle page fault exception (vector 14)
/// The error code contains information about the fault:
/// - Bit 0 (P): Page-level protection violation
/// - Bit 1 (W/R): Write/Read access
/// - Bit 2 (U/S): User/Supervisor mode
/// - Bit 3 (RSVD): Reserved bit violation
/// - Bit 4 (I/D): Instruction/Data fetch
/// - Bit 5 (PK): Protection key violation
/// - Bit 15 (SGX): SGX violation
fn handle_page_fault(frame: &mut InterruptFrame) {
    let error_code = frame.error_code as u64;
    let fault_address: u64;
    
    unsafe {
        // Read the CR2 register which contains the faulting address
        core::arch::asm!("mov {}, cr2", out(reg) fault_address, options(nomem, nostack, preserves_flags));
    }
    
    let present = (error_code & 0x01) != 0;
    let write = (error_code & 0x02) != 0;
    let user = (error_code & 0x04) != 0;
    let reserved = (error_code & 0x08) != 0;
    let instruction = (error_code & 0x10) != 0;
    
    serial_write_str("AWEOS: Page Fault at address: 0x");
    print_u64_hex(fault_address);
    serial_write_str("\r\n");
    serial_write_str("  Error: ");
    if !present { serial_write_str("Not Present "); }
    if write { serial_write_str("Write "); }
    if user { serial_write_str("User "); }
    if reserved { serial_write_str("Reserved "); }
    if instruction { serial_write_str("Instruction "); }
    serial_write_str("\r\n");
    serial_write_str("  RIP: 0x");
    print_u64_hex(frame.rip);
    serial_write_str(" CS: 0x");
    print_u64_hex(frame.cs);
    print_stack_context(frame);
    
    // For now, just panic. In the future, we'll handle page faults by:
    // 1. Checking if it's a demand paging request
    // 2. Checking if it's a copy-on-write request
    // 3. Handling stack growth
    // 4. Returning from the fault if handled
    serial_write_str("AWEOS: Page fault not handled - halting\r\n");
    
    loop {
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}

/// Handle general protection fault (vector 13)
fn handle_gpf(frame: &mut InterruptFrame) {
    serial_write_str("AWEOS: General Protection Fault\r\n");
    serial_write_str("  Error Code: 0x");
    print_u64_hex(frame.error_code);
    serial_write_str("\r\n");
    serial_write_str("  RIP: 0x");
    print_u64_hex(frame.rip);
    serial_write_str(" CS: 0x");
    print_u64_hex(frame.cs);
    print_stack_context(frame);
    
    loop {
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}

/// Handle double fault (vector 8)
fn handle_double_fault(frame: &mut InterruptFrame) {
    serial_write_str("AWEOS: Double Fault\r\n");
    serial_write_str("  Error Code: 0x");
    print_u64_hex(frame.error_code);
    serial_write_str("\r\n");
    serial_write_str("  RIP: 0x");
    print_u64_hex(frame.rip);
    serial_write_str(" CS: 0x");
    print_u64_hex(frame.cs);
    serial_write_str("\r\n");
    
    // Double fault is unrecoverable - reboot
    serial_write_str("AWEOS: Double fault - system reboot required\r\n");
    
    loop {
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}

/// Handle invalid opcode (vector 6)
fn handle_invalid_opcode(frame: &mut InterruptFrame) {
    serial_write_str("AWEOS: Invalid Opcode\r\n");
    serial_write_str("  RIP: 0x");
    print_u64_hex(frame.rip);
    serial_write_str("\r\n");
    
    // Do not dereference the faulting RIP here: it may be unmapped or
    // otherwise unsafe to read, which could recursively fault in the handler.
    print_stack_context(frame);
    
    loop {
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}

/// Handle segment not present (vector 11)
fn handle_segment_not_present(frame: &mut InterruptFrame) {
    serial_write_str("AWEOS: Segment Not Present\r\n");
    serial_write_str("  Error Code: 0x");
    print_u64_hex(frame.error_code);
    serial_write_str("\r\n");
    
    // Error code contains segment selector index
    let segment_selector = frame.error_code & 0xFFF;
    let external = (frame.error_code & (1 << 1)) != 0;
    let table = if (frame.error_code & (1 << 2)) != 0 { "LDT" } else { "GDT" };
    
    serial_write_str("  Segment: ");
    print_u64_hex(segment_selector);
    serial_write_str(" in ");
    serial_write_str(table);
    if external {
        serial_write_str(" (External)");
    }
    serial_write_str("\r\n");
    
    loop {
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}

/// Handle stack segment fault (vector 12)
fn handle_stack_fault(frame: &mut InterruptFrame) {
    serial_write_str("AWEOS: Stack Segment Fault\r\n");
    serial_write_str("  Error Code: 0x");
    print_u64_hex(frame.error_code);
    serial_write_str("\r\n");
    
    loop {
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn awe_interrupt_handler(frame: &mut InterruptFrame) {
    let vector = frame.vector as u8;
    
    // Handle hardware interrupts (IRQs)
    if vector >= 32 && vector < 48 {
        // PIC interrupts
        if vector == 32 {
            unsafe {
                pic_send_eoi(0);
            }
            TIMER_IRQ_COUNT.fetch_add(1, Ordering::Relaxed);
            super::timer::interrupt_tick();
        } else if vector == 33 {
            unsafe {
                let _scancode = super::io_in8(0x60);
                pic_send_eoi(1);
            }
        } else {
            // Other PIC interrupts
            unsafe {
                if vector >= 40 {
                    pic_send_eoi(1);
                } else {
                    pic_send_eoi(0);
                }
            }
        }
        return;
    }
    
    // APIC vectors are reserved here, but this boot path has not yet
    // initialized a local-APIC MMIO mapping/EOI backend. Returning without
    // an EOI could wedge delivery, so fail visibly instead of pretending
    // that the interrupt was acknowledged.
    if vector == u8::MAX {
        // The architectural APIC spurious vector does not require an EOI.
        return;
    }
    if vector >= 48 {
        serial_write_str("AWEOS: APIC vector arrived without an EOI backend: 0x");
        print_u64_hex(vector as u64);
        serial_write_str("\r\n");
        halt_after_exception();
    }
    
    // Handle CPU exceptions
    if vector < 32 {
        let name = if (vector as usize) < EXCEPTION_NAMES.len() {
            EXCEPTION_NAMES[vector as usize]
        } else {
            "Unknown Exception"
        };
        
        serial_write_str("AWEOS: ");
        serial_write_str(name);
        serial_write_str(" (vector ");
        print_u64_hex(vector as u64);
        serial_write_str(")\r\n");
        
        // Call specific handlers for common exceptions
        match vector {
            0 => handle_divide_error(frame), // Divide Error (#DE)
            6 => handle_invalid_opcode(frame), // Invalid Opcode
            8 => handle_double_fault(frame),   // Double Fault
            11 => handle_segment_not_present(frame), // Segment Not Present
            12 => handle_stack_fault(frame),     // Stack Segment Fault
            13 => handle_gpf(frame),             // General Protection Fault
            14 => handle_page_fault(frame),      // Page Fault
            _ => {
                // Generic exception handler
                serial_write_str("  Error Code: 0x");
                print_u64_hex(frame.error_code);
                serial_write_str("\r\n");
                serial_write_str("  RIP: 0x");
                print_u64_hex(frame.rip);
                serial_write_str(" CS: 0x");
                print_u64_hex(frame.cs);
                print_stack_context(frame);
                
                loop {
                    unsafe {
                        core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
                    }
                }
            }
        }
        return;
    }
    
    // Unknown interrupt
    serial_write_str("AWEOS: Unknown interrupt vector=");
    print_u64_hex(frame.vector);
    serial_write_str("\r\n");
}

pub fn timer_irq_count() -> u64 {
    TIMER_IRQ_COUNT.load(Ordering::Acquire)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::x86_64::isr::is_exception_with_error_code;

    fn has_stub(source: &str, kind: &str, vector: u8) -> bool {
        source.lines().any(|line| {
            let mut parts = line.split_whitespace();
            parts.next() == Some(kind)
                && parts.next().and_then(|value| value.parse::<u8>().ok()) == Some(vector)
        })
    }

    #[test]
    fn all_256_idt_vectors_are_installed() {
        let mut idt = super::super::idt::Idt::new();
        init_idt_stubs(&mut idt, super::super::gdt::KERNEL_CODE_SELECTOR);
        for vector in 0..=u8::MAX {
            assert!(idt.is_present(vector), "IDT vector {vector} is missing");
        }
    }

    #[test]
    fn assembly_error_code_stubs_match_x86_exception_contract() {
        let source = include_str!("isr_stubs.rs");
        for vector in 0..32 {
            let kind = if is_exception_with_error_code(vector) {
                "ISR_ERR"
            } else {
                "ISR_NOERR"
            };
            assert!(
                has_stub(source, kind, vector),
                "vector {vector} must use {kind}"
            );
        }
    }

    #[test]
    fn common_entry_aligns_rust_call_and_restores_interrupt_frame() {
        let source = include_str!("isr_stubs.rs");
        let common = source
            .split("awe_isr_common:")
            .nth(1)
            .expect("common ISR label")
            .split(".macro ISR_NOERR")
            .next()
            .expect("common ISR body");

        assert!(common.contains("mov rbx, rsp"));
        assert!(common.contains("and rsp, -16"));
        assert!(common.contains("cld"));
        assert!(common.contains("call awe_interrupt_handler"));
        assert!(common.contains("mov rsp, rbx"));
    }
}
