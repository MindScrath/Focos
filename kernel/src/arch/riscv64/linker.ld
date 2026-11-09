OUTPUT_ARCH(riscv)
ENTRY(_start)

SECTIONS {
    . = 0x80200000;
    .text : { KEEP(*(.text.entry)) *(.text .text.*) }
    . = ALIGN(4096);
    .rodata : { *(.rodata .rodata.*) *(.srodata .srodata.*) }
    . = ALIGN(4096);
    .data : { *(.data .data.*) *(.sdata .sdata.*) *(.got .got.*) }
    . = ALIGN(4096);
    __bss_start = .;
    .bss (NOLOAD) : { *(.bss .bss.*) *(.sbss .sbss.*) *(COMMON) }
    . = ALIGN(8);
    __bss_end = .;
    /* 包含启动栈和页分配标记，物理页分配从此地址向上对齐后开始。 */
    __kernel_end = .;
    /DISCARD/ : { *(.eh_frame .eh_frame_hdr) }
}
