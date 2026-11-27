# MM-003：直接模式同步入口，所有整数寄存器及启用时的浮点状态均保存。
# 544 字节栈帧保持 16 字节对齐。

# x1 至 x5 单独处理。入口先保存 x5，再借它将入栈前的 sp 写入 x2 槽。
.macro trap_gprs_x6_x31 op
    .irp n, 6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31
        \op x\n, (8 * \n)(sp)
    .endr
.endm

# 浮点寄存器的栈槽紧接 sstatus 和 fcsr，从 272(sp) 开始。
.macro trap_fprs op
    .irp n, 0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31
        \op f\n, (272 + 8 * \n)(sp)
    .endr
.endm

.section .text
.balign 4
.global focos_vm_trap_entry
.type focos_vm_trap_entry, @function
focos_vm_trap_entry:
    addi sp, sp, -544
    sd x5, 40(sp)
    addi x5, sp, 544
    sd x5, 16(sp)
    sd x1, 8(sp)
    sd x3, 24(sp)
    sd x4, 32(sp)
    trap_gprs_x6_x31 sd
    csrr t0, sstatus
    sd t0, 256(sp)
    # Rust、浮点保存及日志路径在正常内核权限下运行。
    li t1, 0x40000
    csrc sstatus, t1
    li t1, 0x6000
    and t1, t0, t1
    beqz t1, .Lsaved
    frcsr t0
    sd t0, 264(sp)
    trap_fprs fsd
.Lsaved:
    csrr a0, scause
    csrr a1, sepc
    csrr a2, stval
    call focos_vm_trap
    csrw sepc, a0
    ld t0, 256(sp)
    li t1, 0x6000
    and t1, t0, t1
    beqz t1, .Lrestore
    trap_fprs fld
    ld t0, 264(sp)
    fscsr t0
.Lrestore:
    # 用户拷贝缺页时栈帧里可能保存了 SUM=1。写回前只清这一位，出栈时保持 SUM=0。
    .option push
    .option norvc
    ld t0, 256(sp)
    li t1, 0x40000
    not t1, t1
    and t0, t0, t1
    csrw sstatus, t0
    .option pop
    ld x1, 8(sp)
    ld x3, 24(sp)
    ld x4, 32(sp)
    ld x5, 40(sp)
    trap_gprs_x6_x31 ld
    ld sp, 16(sp)
    sret
.size focos_vm_trap_entry, .-focos_vm_trap_entry
.purgem trap_gprs_x6_x31
.purgem trap_fprs

.balign 4
.global focos_probe_read
.type focos_probe_read, @function
focos_probe_read:
.global focos_probe_read_pc
focos_probe_read_pc:
    lbu a0, 0(a0)
.global focos_probe_read_resume
focos_probe_read_resume:
    ret
.size focos_probe_read, .-focos_probe_read
.balign 4
.global focos_probe_write
.type focos_probe_write, @function
focos_probe_write:
.global focos_probe_write_pc
focos_probe_write_pc:
    sb a1, 0(a0)
.global focos_probe_write_resume
focos_probe_write_resume:
    ret
.size focos_probe_write, .-focos_probe_write
.balign 4
.global focos_probe_execute
.type focos_probe_execute, @function
focos_probe_execute:
    addi sp, sp, -16
    sd ra, 8(sp)
    jalr ra, 0(a0)
.global focos_probe_execute_resume
focos_probe_execute_resume:
    ld ra, 8(sp)
    addi sp, sp, 16
    ret
.size focos_probe_execute, .-focos_probe_execute

# a0 用户地址，a1 内核缓冲，a2 长度；a3 只统计已完成字节。
# 固定宽度便于从验收 ELF 核对开关和唯一访存指令确实相邻。
.option push
.option norvc
.balign 4
.global focos_copy_from_user
.type focos_copy_from_user, @function
focos_copy_from_user:
    li a3, 0
    li t1, 0x40000
    beqz a2, .Lread_done
.Lread_byte:
    csrs sstatus, t1
.global focos_copy_from_user_load_pc
focos_copy_from_user_load_pc:
    lbu t0, 0(a0)
    csrc sstatus, t1
    sb t0, 0(a1)
    addi a0, a0, 1
    addi a1, a1, 1
    addi a3, a3, 1
    bne a3, a2, .Lread_byte
.Lread_done:
    mv a0, a3
    ret
.global focos_copy_from_user_fault_resume
focos_copy_from_user_fault_resume:
    li t1, 0x40000
    csrc sstatus, t1
    j .Lread_done
.size focos_copy_from_user, .-focos_copy_from_user

.global focos_copy_to_user
.type focos_copy_to_user, @function
focos_copy_to_user:
    li a3, 0
    li t1, 0x40000
    beqz a2, .Lwrite_done
.Lwrite_byte:
    lbu t0, 0(a1)
    csrs sstatus, t1
.global focos_copy_to_user_store_pc
focos_copy_to_user_store_pc:
    sb t0, 0(a0)
    csrc sstatus, t1
    addi a0, a0, 1
    addi a1, a1, 1
    addi a3, a3, 1
    bne a3, a2, .Lwrite_byte
.Lwrite_done:
    mv a0, a3
    ret
.global focos_copy_to_user_fault_resume
focos_copy_to_user_fault_resume:
    li t1, 0x40000
    csrc sstatus, t1
    j .Lwrite_done
.size focos_copy_to_user, .-focos_copy_to_user
.option pop
