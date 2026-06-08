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

    /* Thread-local template (variant II, local-exec — target is reloc-model
     * static). crt0 copies [__tls_start, __tdata_end) (.tdata) and zeroes the
     * rest of the block into each thread's TLS; FS base = block top (the TCB).
     *
     * crt0 can't read PT_TLS p_align at runtime (the phdrs aren't mapped: the
     * first PT_LOAD starts at file offset 0x1000). To stay align-independent we
     * pad the segment size up to 64 *inside* .tbss — lld counts an intra-section
     * `. = ALIGN` in p_memsz — so the tpoff block size `alignTo(p_memsz,p_align)`
     * equals `__tls_end - __tls_start` for any thread-local alignment <= 64.
     * When there are no thread-locals the location is already 64-aligned, the
     * pad is a no-op, and __tls_end == __tls_start (size 0) so crt0 skips TLS. */
    .tdata ALIGN(64) : {
        __tls_start = .;
        *(.tdata .tdata.*)
        *(.tbss .tbss.*)
        . = ALIGN(64);
        __tls_end = .;
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
