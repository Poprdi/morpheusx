/*
 * 32-bit protected mode trampoline
 */

.code64
.section .text
.global drop_to_protected_mode_asm

drop_to_protected_mode_asm:
    /* Save 32-bit arguments in high registers */
    movl %edi, %r14d
    movl %esi, %r15d
    
    /* Disable interrupts */
    cli
    cld
    
    /* Disable paging: CR0.PG = 0 */
    movq %cr0, %rax
    andl $0x7fffffff, %eax
    movq %rax, %cr0
    
    /* Disable long mode: EFER.LME = 0 */
    movl $0xc0000080, %ecx
    rdmsr
    andl $0xfffffeff, %eax
    wrmsr
    
    /* Load 32-bit GDT */
    lgdt gdt32_ptr(%rip)
    
    /* Move arguments to stack (will survive mode switch) */
    pushq %r15  /* boot_params */
    pushq %r14  /* entry_point */
    
    /* Far jump to 32-bit code */
    pushq $0x08
    leaq pm32(%rip), %rax
    pushq %rax
    lretq

.code32
pm32:
    /* Set up 32-bit segments */
    movw $0x10, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %fs
    movw %ax, %gs
    movw %ax, %ss
    
    /* Pop arguments (4 bytes each in 32-bit) */
    popl %edi       /* entry_point */
    addl $4, %esp   /* skip high 32 bits */
    popl %esi       /* boot_params */
    addl $4, %esp   /* skip high 32 bits */
    
    /* Zero registers */
    xorl %eax, %eax
    xorl %ebx, %ebx
    xorl %ecx, %ecx
    xorl %edx, %edx
    xorl %ebp, %ebp
    
    /* Jump to kernel */
    jmp *%edi

.align 16
gdt32:
    .quad 0x0000000000000000
    .quad 0x00cf9a000000ffff
    .quad 0x00cf92000000ffff

gdt32_ptr:
    .word gdt32_ptr - gdt32 - 1
    .quad gdt32
