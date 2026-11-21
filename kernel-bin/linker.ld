ENTRY(_start)

SECTIONS
{
    . = 1M;
    __kernel_start = .;

    .note.Xen : ALIGN(4)
    {
        KEEP(*(.note.Xen))
    }

    .multiboot2_header : ALIGN(8)
    {
        KEEP(*(.multiboot2_header))
    }

    .multiboot_header : ALIGN(8)
    {
        KEEP(*(.multiboot_header))
    }

    .text : ALIGN(4K)
    {
        KEEP(*(.text.boot))
        *(.text .text.*)
    }

    .rodata : ALIGN(4K)
    {
        *(.rodata .rodata.*)
    }

    .data : ALIGN(4K)
    {
        *(.data .data.*)
    }

    .bss : ALIGN(4K)
    {
        *(COMMON)
        *(.bss .bss.*)
    }

    . = ALIGN(4K);
    __kernel_end = .;

    /DISCARD/ :
    {
        *(.eh_frame*)
        *(.comment)
        *(.note*)
    }
}
