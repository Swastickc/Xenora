# SPDX-License-Identifier: GPL-3.0-or-later

.text
.globl launch_entry
launch_entry:
    subq $40, %rsp
    callq *__imp_first_action(%rip)
    cmpl $7, %eax
    jne denied
    callq *__imp_second_action(%rip)
    cmpl $13, %eax
    jne denied
    xorl %eax, %eax
    addq $40, %rsp
    retq
denied:
    movl $17, %eax
    addq $40, %rsp
    retq
