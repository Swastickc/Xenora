# SPDX-License-Identifier: GPL-3.0-or-later

.text
.globl launch_entry
launch_entry:
    movq %rsp, %rax
    andl $15, %eax
    cmpl $8, %eax
    jne wrong_frame
#ifdef WRONG_CALLBACK_COUNT
    cmpl $3, callback_sequence(%rip)
#else
    cmpl $2, callback_sequence(%rip)
#endif
    jne wrong_sequence

    leaq pointer_table(%rip), %r8
    leaq large_region_end-8(%rip), %r9
    movl $8192, %ecx
check_pointer:
    cmpq %r9, (%r8)
    jne wrong_relocation
    addq $8, %r8
    decl %ecx
    jne check_pointer
    cmpq $0, (%r9)
    jne wrong_zero_fill
    movq $123, (%r9)
    cmpq $123, (%r9)
    jne wrong_zero_fill
    xorl %eax, %eax
    retq
wrong_frame:
    movl $11, %eax
    retq
wrong_sequence:
    movl $12, %eax
    retq
wrong_relocation:
    movl $13, %eax
    retq
wrong_zero_fill:
    movl $14, %eax
    retq

callback_one:
    movq %rsp, %rax
    andl $15, %eax
    cmpl $8, %eax
    jne callback_done
    leaq __ImageBase(%rip), %rax
    cmpq %rax, %rcx
    jne callback_done
    cmpl $1, %edx
    jne callback_done
    testq %r8, %r8
    jne callback_done
    cmpl $0, callback_sequence(%rip)
    jne callback_done
    movl $1, callback_sequence(%rip)
callback_done:
    retq

callback_two:
    movq %rsp, %rax
    andl $15, %eax
    cmpl $8, %eax
    jne second_done
    leaq __ImageBase(%rip), %rax
    cmpq %rax, %rcx
    jne second_done
    cmpl $1, %edx
    jne second_done
    testq %r8, %r8
    jne second_done
    cmpl $1, callback_sequence(%rip)
    jne second_done
    movl $2, callback_sequence(%rip)
second_done:
    retq

.data
.p2align 3
pointer_table:
.rept 8192
    .quad large_region_end-8
.endr
tls_index:
    .long 0xf00d
callback_sequence:
    .long 0

.bss
.p2align 3
large_region:
    .zero 134217728
large_region_end:

.section .tls,"dw"
tls_first:
    .quad 0x175123
tls_end:

.section .rdata,"dr"
.p2align 3
.globl _tls_used
_tls_used:
    .quad tls_first
    .quad tls_end
    .quad tls_index
    .quad callbacks
    .long 8
    .long 0
callbacks:
    .quad callback_one
    .quad callback_two
    .quad 0
