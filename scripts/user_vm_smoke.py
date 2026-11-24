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


def audit_copy_windows(elf, symbols):
    """检查执行段中标签旁的 RV64 指令。读回 SUM=0 无法单独证明循环内每字节都清零。"""
    try:
        headers = subprocess.check_output(["readelf", "-W", "-l", str(elf)], text=True)
    except (OSError, subprocess.CalledProcessError) as error:
        raise ValueError(f"cannot read user copy ELF: {error}") from error
    segments = []
    for line in headers.splitlines():
        fields = line.split()
        if fields[:1] == ["LOAD"] and "E" in fields[6:-1]:
            offset, addr, _, size = (int(value, 16) for value in fields[1:5])
            segments.append((addr, size, offset))
    data = elf.read_bytes()
    # csrs/csrc sstatus,t1；lbu/sb 的用户基址是 a0，内核基址是 a1；t0 传送字节。
    enable, disable = 0x10032073, 0x10033073
    windows = [(READ, -4, (enable, 0x00054283, disable, 0x00558023)),
               (WRITE, -8, (0x0005c283, enable, 0x00550023, disable)),
               *((name, 0, (0x00040337, disable)) for name in RESUME)]
    for name, displacement, expected in windows:
        start = symbols[name] + displacement
        code = struct.pack("<" + "I" * len(expected), *expected)
        for addr, size, offset in segments:
            if addr <= start and start + len(code) <= addr + size:
                index = offset + start - addr
                if data[index:index + len(code)] == code:
                    break
        else:
            raise ValueError(f"unexpected user permission window at {name}")
