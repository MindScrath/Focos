# MM-003：直接模式同步入口，所有整数寄存器及启用时的浮点状态均保存。
# 544 字节栈帧保持 16 字节对齐；x2 槽保存进入前的 sp。
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
    sd x6, 48(sp)
    sd x7, 56(sp)
    sd x8, 64(sp)
    sd x9, 72(sp)
    sd x10, 80(sp)
    sd x11, 88(sp)
    sd x12, 96(sp)
    sd x13, 104(sp)
    sd x14, 112(sp)
    sd x15, 120(sp)
    sd x16, 128(sp)
    sd x17, 136(sp)
    sd x18, 144(sp)
    sd x19, 152(sp)
    sd x20, 160(sp)
    sd x21, 168(sp)
    sd x22, 176(sp)
    sd x23, 184(sp)
    sd x24, 192(sp)
    sd x25, 200(sp)
    sd x26, 208(sp)
    sd x27, 216(sp)
    sd x28, 224(sp)
    sd x29, 232(sp)
    sd x30, 240(sp)
    sd x31, 248(sp)
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
    fsd f0, 272(sp)
    fsd f1, 280(sp)
    fsd f2, 288(sp)
    fsd f3, 296(sp)
    fsd f4, 304(sp)
    fsd f5, 312(sp)
    fsd f6, 320(sp)
    fsd f7, 328(sp)
    fsd f8, 336(sp)
    fsd f9, 344(sp)
    fsd f10, 352(sp)
    fsd f11, 360(sp)
    fsd f12, 368(sp)
    fsd f13, 376(sp)
    fsd f14, 384(sp)
    fsd f15, 392(sp)
    fsd f16, 400(sp)
    fsd f17, 408(sp)
    fsd f18, 416(sp)
    fsd f19, 424(sp)
    fsd f20, 432(sp)
    fsd f21, 440(sp)
    fsd f22, 448(sp)
    fsd f23, 456(sp)
    fsd f24, 464(sp)
    fsd f25, 472(sp)
    fsd f26, 480(sp)
    fsd f27, 488(sp)
    fsd f28, 496(sp)
    fsd f29, 504(sp)
    fsd f30, 512(sp)
    fsd f31, 520(sp)
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
    fld f0, 272(sp)
    fld f1, 280(sp)
    fld f2, 288(sp)
    fld f3, 296(sp)
    fld f4, 304(sp)
    fld f5, 312(sp)
    fld f6, 320(sp)
    fld f7, 328(sp)
    fld f8, 336(sp)
    fld f9, 344(sp)
    fld f10, 352(sp)
    fld f11, 360(sp)
    fld f12, 368(sp)
    fld f13, 376(sp)
    fld f14, 384(sp)
    fld f15, 392(sp)
    fld f16, 400(sp)
    fld f17, 408(sp)
    fld f18, 416(sp)
    fld f19, 424(sp)
    fld f20, 432(sp)
    fld f21, 440(sp)
    fld f22, 448(sp)
    fld f23, 456(sp)
    fld f24, 464(sp)
    fld f25, 472(sp)
    fld f26, 480(sp)
    fld f27, 488(sp)
    fld f28, 496(sp)
    fld f29, 504(sp)
    fld f30, 512(sp)
    fld f31, 520(sp)
    ld t0, 264(sp)
    fscsr t0
.Lrestore:
    ld t0, 256(sp)
    csrw sstatus, t0
    ld x1, 8(sp)
    ld x3, 24(sp)
    ld x4, 32(sp)
    ld x5, 40(sp)
    ld x6, 48(sp)
    ld x7, 56(sp)
    ld x8, 64(sp)
    ld x9, 72(sp)
    ld x10, 80(sp)
    ld x11, 88(sp)
    ld x12, 96(sp)
    ld x13, 104(sp)
    ld x14, 112(sp)
    ld x15, 120(sp)
    ld x16, 128(sp)
    ld x17, 136(sp)
    ld x18, 144(sp)
    ld x19, 152(sp)
    ld x20, 160(sp)
    ld x21, 168(sp)
    ld x22, 176(sp)
    ld x23, 184(sp)
    ld x24, 192(sp)
    ld x25, 200(sp)
    ld x26, 208(sp)
    ld x27, 216(sp)
    ld x28, 224(sp)
    ld x29, 232(sp)
    ld x30, 240(sp)
    ld x31, 248(sp)
    ld sp, 16(sp)
    sret
.size focos_vm_trap_entry, .-focos_vm_trap_entry
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
