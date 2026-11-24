"""按 MM-004 核对原始硬件记录与本次 ELF 的用户拷贝权限窗口。"""
import re
import struct
import subprocess

READ = "focos_copy_from_user_load_pc"
WRITE = "focos_copy_to_user_store_pc"
RESUME = ("focos_copy_from_user_fault_resume", "focos_copy_to_user_fault_resume")
SYMBOLS = (READ, WRITE, *RESUME)
MARKERS = ("FOCOS_USER_VM_ISOLATION_OK", "FOCOS_USER_VM_COPY_OK",
           "FOCOS_USER_VM_SWITCH_OK", "FOCOS_USER_VM_CLEAN_OK", "FOCOS_USER_VM_OK")


def records(output, tag, pattern, count):
    """带标识的每一行都必须能完整解析；残缺或额外行同样导致失败。"""
    matches = list(re.finditer("^" + re.escape(tag) + pattern + "$", output, re.MULTILINE))
    if len(matches) != count or output.count(tag) != count:
        raise ValueError(f"invalid {tag} records")
    return matches


def validate(output, symbols, memory_start, memory_end, kernel_root, base_pages):
    """用独立的预期原因、地址、阶段、当前根和页数核对每一次观测。"""
    vm, isolated, copied, switched, clean = (
        output.index(marker) for marker in ("FOCOS_VM_OK", *MARKERS[:4]))
    raw = r" scause=(0x[0-9a-f]+) sepc=(0x[0-9a-f]+) stval=(0x[0-9a-f]+)"
    isolation = records(output, "FOCOS_USER_ISOLATION_FAULT", raw, 3)
    copies = records(output, "FOCOS_USER_COPY_FAULT", raw + r" copied=(\d+)", 4)
    for name in SYMBOLS:
        if not symbols["__code_start"] <= symbols[name] < symbols["__code_end"] or symbols[name] % 2:
            raise ValueError("user copy label outside kernel code")
    if len({symbols[name] for name in SYMBOLS}) != len(SYMBOLS):
        raise ValueError("duplicate user copy instruction or resume address")
    expected = [(13, symbols["focos_probe_read_pc"], 0x10000),
                (15, symbols["focos_probe_write_pc"], 0x10000), (12, 0x12000, 0x12000),
                (13, symbols[READ], 0x13000, 0), (15, symbols[WRITE], 0x11000, 0),
                (13, symbols[READ], 0x15000, 8), (15, symbols[WRITE], 0x15000, 8)]
    for match, values in zip(isolation + copies, expected):
        if tuple(int(v, 0) for v in match.groups()) != values:
            raise ValueError("user fault kind, instruction, address or progress mismatch")
    if not (vm < isolation[0].start() < isolation[-1].start() < isolated
            < copies[0].start() < copies[-1].start() < copied):
        raise ValueError("user faults outside their stages")
    cpus = records(output, "FocOS USER VM CPU:",
                   r" satp=(0x[0-9a-f]+) mode=Sv39 asid=0 root=(0x[0-9a-f]+)", 4)
    roots = []
    for match in cpus:
        satp, root = (int(v, 16) for v in match.groups())
        if not memory_start <= root < memory_end or root % 4096 or satp != (8 << 60) | (root >> 12):
            raise ValueError("invalid user root or satp")
        roots.append(root)
    a, b, again, kernel = roots
    if len({a, b, kernel_root}) != 3 or again != a or kernel != kernel_root:
        raise ValueError("wrong user root switch sequence")
    retained, = records(output, "FocOS USER VM retained table pages:", r" (\d+)", 1)
    if (int(retained.group(1)) != base_pages
            or not vm < cpus[0].start() < isolation[0].start()
            or not copied < cpus[1].start() < cpus[2].start() < switched
            or not switched < cpus[3].start() < retained.start() < clean):
        raise ValueError("user switch or retained pages outside expected stage")


def audit_trap_return(code):
    """从实际异常入口走到 sret，检查可能将 SUM 置位的 sstatus 写入。"""
    # ld t0,256(sp)；li t1,SUM；not t1,t1；and t0,t0,t1；csrw sstatus,t0。
    restore = struct.pack("<5I", 0x10013283, 0x00040337, 0xfff34313, 0x0062f2b3, 0x10029073)
    masked_restore = False
    pos = 0
    while pos + 2 <= len(code):
        # RV64GC 的 16/32 位指令可以交错，不能把指令的后半字当成 CSR 写入。
        width = 4 if code[pos] & 3 == 3 else 2
        if pos + width > len(code):
            break
        insn = int.from_bytes(code[pos:pos + width], "little")
        pos += width
        if insn == 0x10200073:  # 此入口的 sret 结束公共返回路径。
            if masked_restore and pos == len(code):
                return
            break
        if insn & 0xfff0007f == 0x10000073:  # SYSTEM，CSR 地址为 sstatus
            operation = (insn >> 12) & 7
            source = (insn >> 15) & 31
            # CSRRW 或使用非零寄存器的 CSRRS 可重新开放 SUM。CSRRC 只能清位，
            # CSR 立即数只有 5 位，读 sstatus 的 CSRRS 使用 x0。
            if operation == 1 or (operation == 2 and source != 0):
                if masked_restore or code[pos - len(restore):pos] != restore:
                    raise ValueError("trap return can re-enable SUM")
                masked_restore = True
    raise ValueError("missing masked sstatus restore before sret")


def audit_copy_windows(elf, symbols):
    """核对实际 ELF 中的异常返回、逐字节访存和故障恢复指令。"""
    try:
        headers = subprocess.check_output(["readelf", "-W", "-l", "-s", str(elf)], text=True)
    except (OSError, subprocess.CalledProcessError) as error:
        raise ValueError(f"cannot read user copy ELF: {error}") from error
    segments = []
    trap_functions = []
    for line in headers.splitlines():
        fields = line.split()
        if fields[:1] == ["LOAD"] and "E" in fields[6:-1]:
            offset, addr, _, size = (int(value, 16) for value in fields[1:5])
            segments.append((addr, size, offset))
        elif (len(fields) == 8 and fields[-1] == "focos_vm_trap_entry"
              and fields[3] == "FUNC" and fields[6] != "UND"):
            trap_functions.append((int(fields[1], 16), int(fields[2], 0)))
    data = elf.read_bytes()

    def executable_bytes(start, length):
        for addr, size, offset in segments:
            if addr <= start and start + length <= addr + size:
                index = offset + start - addr
                if index + length <= len(data):
                    return data[index:index + length]
        raise ValueError("permission audit outside executable ELF segment")

    if len(trap_functions) != 1:
        raise ValueError("expected one ELF function focos_vm_trap_entry")
    audit_trap_return(executable_bytes(*trap_functions[0]))
    # csrs/csrc sstatus,t1；lbu/sb 的用户基址是 a0，内核基址是 a1；t0 传送字节。
    enable, disable = 0x10032073, 0x10033073
    windows = [(READ, -4, (enable, 0x00054283, disable, 0x00558023)),
               (WRITE, -8, (0x0005c283, enable, 0x00550023, disable)),
               *((name, 0, (0x00040337, disable)) for name in RESUME)]
    for name, displacement, expected in windows:
        start = symbols[name] + displacement
        code = struct.pack("<" + "I" * len(expected), *expected)
        if executable_bytes(start, len(code)) != code:
            raise ValueError(f"unexpected user permission window at {name}")
