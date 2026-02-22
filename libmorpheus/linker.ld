/* ═══════════════════════════════════════════════════════════════════════
 * linker.ld — MorpheusX userspace ELF64 linker script
 *
 * Virtual address layout for user processes:
 *   0x0000_0000_0040_0000  .text   (entry point)
 *   ...                     .rodata
 *   ...                     .data
 *   ...                     .bss
 *   0x0000_007F_FFFF_F000  Stack top (set by kernel, grows down)
 *
 * The kernel's ELF loader maps PT_LOAD segments into the process's
 * page table with USER bit set.  The stack is separately allocated.
 * ═══════════════════════════════════════════════════════════════════════ */

OUTPUT_FORMAT("elf64-x86-64")
OUTPUT_ARCH(i386:x86-64)
ENTRY(_start)

SECTIONS
{
    . = 0x400000;

    .text ALIGN(4K) : {
        *(.text._start)
        *(.text .text.*)
    }

    .rodata ALIGN(4K) : {
        *(.rodata .rodata.*)
    }

    .data ALIGN(4K) : {
        *(.data .data.*)
    }

    .bss ALIGN(4K) : {
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        __bss_end = .;
    }

    /DISCARD/ : {
        *(.comment)
        *(.note.*)
        *(.eh_frame*)
        *(.debug_*)
    }
}
