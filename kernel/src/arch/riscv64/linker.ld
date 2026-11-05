OUTPUT_ARCH(riscv)
ENTRY(_start)

/* 旧 U-Boot bootelf -p 会处理每个程序头，只生成可装载的内存段。 */
PHDRS {
    text PT_LOAD FLAGS(5);
    rodata PT_LOAD FLAGS(4);
    data PT_LOAD FLAGS(6);
}

SECTIONS {
    . = 0x80200000;
    .text : { KEEP(*(.text.entry)) *(.text .text.*) } :text
    . = ALIGN(4096);
    .rodata : { *(.rodata .rodata.*) *(.srodata .srodata.*) } :rodata
    . = ALIGN(4096);
    .data : { *(.data .data.*) *(.sdata .sdata.*) } :data
    . = ALIGN(4096);
    __bss_start = .;
    .bss (NOLOAD) : { *(.bss .bss.*) *(.sbss .sbss.*) *(COMMON) } :data
    . = ALIGN(8);
    __bss_end = .;
    /DISCARD/ : { *(.eh_frame .eh_frame_hdr) *(.riscv.attributes) }
}
