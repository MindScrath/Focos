#!/usr/bin/env python3
"""在 QEMU 中验收启动、物理页、本核中断状态、内核分页和用户映射。"""
import argparse
import hashlib
import pathlib
import re
import shlex
import subprocess
import sys
import user_vm_smoke

ROOT = pathlib.Path(__file__).resolve().parents[1]
ELF = ROOT / "target/riscv64gc-unknown-none-elf/release/focos-kernel"
LOG = ROOT / "target/evidence/boot-qemu-rv.log"
MM_LOG = ROOT / "target/evidence/mm-qemu-rv.log"
IRQ_LOG = ROOT / "target/evidence/irq-qemu-rv.log"
VM_LOG = ROOT / "target/evidence/vm-qemu-rv.log"
USER_VM_LOG = ROOT / "target/evidence/user-vm-qemu-rv.log"

# 合同名、必须唯一出现且按顺序排列的成功标记。
CONTRACTS = {
    "boot": ("BOOT-001", ("FOCOS_BOOT_OK",)),
    "mm": ("MM-002", ("FOCOS_MM_OK", "FOCOS_BOOT_OK")),
    "irq": ("ARCH-001", ("FOCOS_IRQ_OK", "FOCOS_BOOT_OK")),
    "vm": ("MM-003", (
        "FOCOS_MM_OK", "FOCOS_VM_ENABLED", "FOCOS_VM_ALIAS_OK",
        "FOCOS_VM_READONLY_OK", "FOCOS_VM_NOEXEC_OK", "FOCOS_VM_UNMAP_OK",
        "FOCOS_VM_REMAP_OK", "FOCOS_VM_CLEAN_OK", "FOCOS_VM_LIFECYCLE_OK",
        "FOCOS_VM_OK", "FOCOS_BOOT_OK",
    )),
}
CONTRACTS["user-vm"] = ("MM-004", (*CONTRACTS["vm"][1][:-1], *user_vm_smoke.MARKERS, "FOCOS_BOOT_OK"))
LOG_PATHS = {"boot": LOG, "mm": MM_LOG, "irq": IRQ_LOG, "vm": VM_LOG, "user-vm": USER_VM_LOG}


# 从本次加载的 ELF 取边界和探针位置，不能固定为某一次构建的地址。
VM_SYMBOL_NAMES = (
    "__code_start", "__code_end", "__rodata_start", "__rodata_end",
    "__writable_start", "__kernel_end", "focos_probe_read_pc", "focos_probe_write_pc",
)


def load_vm_symbols(elf, names=VM_SYMBOL_NAMES):
    """读取 ELF 符号；缺工具、缺符号、重复或未定义符号都不能验收成功。"""
    try:
        output = subprocess.check_output(["readelf", "--wide", "--symbols", str(elf)],
                                         text=True, stderr=subprocess.STDOUT)
    except (OSError, subprocess.CalledProcessError) as error:
        raise ValueError(f"cannot read VM ELF symbols: {error}") from error
    symbols = {}
    for line in output.splitlines():
        fields = line.split()
        if len(fields) != 8 or fields[-1] not in names:
            continue
        name = fields[-1]
        if fields[6] == "UND" or name in symbols:
            raise ValueError(f"invalid or duplicate ELF symbol: {name}")
        symbols[name] = int(fields[1], 16)
    if set(symbols) != set(names):
        raise ValueError("missing VM ELF symbols")
    return symbols


def validate_vm_state(output, symbols):
    """独立核对实际布局、两次启用的 CPU 状态和最终保留页数。"""
    fields = ("code", "rodata", "writable", "allocatable", "test_window")
    parts = []
    for field in fields:
        wrapper = "VirtAddr" if field == "test_window" else "PhysAddr"
        parts.append(rf"{field}: {wrapper}\((\d+)\)\.\.{wrapper}\((\d+)\)")
    layout = re.findall(r"^FocOS VM layout: KernelLayout \{ " + ", ".join(parts)
                        + r" \}$", output, re.MULTILINE)
    if len(layout) != 1 or output.count("FocOS VM layout:") != 1:
        raise ValueError("expected exactly one VM layout")
    values = tuple(int(value) for value in layout[0])
    ranges = list(zip(values[::2], values[1::2]))
    if any(start >= end or start % 4096 or end % 4096 for start, end in ranges):
        raise ValueError("invalid VM layout boundaries")
    bounds = tuple(symbols[name] for name in VM_SYMBOL_NAMES[:6])
    if values[:6] != bounds or any(ranges[i][1] != ranges[i + 1][0] for i in range(3)):
        raise ValueError("VM layout does not match ELF")
    if ranges[-1] != (0xffffffc000000000, 0xffffffc000004000):
        raise ValueError("invalid VM test window")
    start, end = ranges[3]
    memory = re.findall(r"^FocOS memory: \[(0x[0-9a-f]+), (0x[0-9a-f]+)\)$",
                        output, re.MULTILINE)
    if (len(memory) != 1 or output.count("FocOS memory:") != 1
            or tuple(int(value, 16) for value in memory[0]) != (start, end)
            or not 0x80000000 <= ranges[0][0] < end <= 0x88000000):
        raise ValueError("VM layout does not match managed RAM")
    for name in VM_SYMBOL_NAMES[-2:]:
        if not ranges[0][0] <= symbols[name] < ranges[0][1] or symbols[name] % 2:
            raise ValueError("VM probe symbol outside code")
    cpus = list(re.finditer(
        r"^FocOS VM CPU: satp=(0x[0-9a-f]+) mode=Sv39 asid=0 root=(0x[0-9a-f]+)$",
        output, re.MULTILINE))
    if len(cpus) != 2 or output.count("FocOS VM CPU:") != 2:
        raise ValueError("expected exactly two VM CPU records")
    for cpu in cpus:
        satp, root = (int(value, 16) for value in cpu.groups())
        if (not start <= root < end or root % 4096
                or satp != (8 << 60) | (root >> 12)):
            raise ValueError("VM CPU root or mode mismatch")
    if not (output.index("FocOS VM layout:") < cpus[0].start()
            < output.index("FOCOS_VM_ENABLED")
            < output.index("FOCOS_VM_CLEAN_OK") < cpus[1].start()
            < output.index("FOCOS_VM_LIFECYCLE_OK")):
        raise ValueError("VM CPU records are out of order")
    retained = list(re.finditer(r"^FocOS VM retained table pages: (\d+)$", output, re.MULTILINE))
    # 三级 4 KiB 页表：根页、中层覆盖的 1 GiB 区间、末级覆盖的 2 MiB 区间。
    expected_pages = 1 + ((end - 1) >> 30) - (ranges[0][0] >> 30) + 1
    expected_pages += ((end - 1) >> 21) - (ranges[0][0] >> 21) + 1
    if (len(retained) != 1 or output.count("FocOS VM retained table pages:") != 1
            or int(retained[0].group(1)) != expected_pages
            or not output.index("FOCOS_VM_LIFECYCLE_OK") < retained[0].start()
            < output.index("FOCOS_VM_OK")):
        raise ValueError("VM retained page evidence mismatch")
    return start, end, int(cpus[-1].group(2), 16), expected_pages


def validate_result(output, returncode, contract="boot", vm_symbols=None):
    """核对退出状态、panic 和合同要求的唯一成功标记及其顺序。"""
    if returncode != 0:
        raise ValueError(f"QEMU exited with status {returncode}")
    if "FOCOS_PANIC" in output:
        raise ValueError("kernel reported FOCOS_PANIC")
    if "FOCOS_VM_UNEXPECTED" in output:
        raise ValueError("kernel reported FOCOS_VM_UNEXPECTED")

    _, markers = CONTRACTS[contract]
    lines = output.splitlines()
    for marker in markers:
        if output.count(marker) != 1 or lines.count(marker) != 1:
            raise ValueError(f"expected exactly one standalone {marker}")

    positions = [lines.index(marker) for marker in markers]
    if positions != sorted(positions):
        raise ValueError(f"{contract} success markers are out of order")
    if contract in ("vm", "user-vm"):
        names = VM_SYMBOL_NAMES + (user_vm_smoke.SYMBOLS if contract == "user-vm" else ())
        symbols = load_vm_symbols(ELF, names) if vm_symbols is None else vm_symbols
        state = validate_vm_state(output, symbols)
        events = re.findall(
            r"^FOCOS_VM_FAULT scause=(0x[0-9a-f]+) sepc=(0x[0-9a-f]+) "
            r"stval=(0x[0-9a-f]+)$", output, re.MULTILINE,
        )
        expected = ((15, 0xffffffc000001000, symbols["focos_probe_write_pc"]),
                    (12, 0xffffffc000002000, 0xffffffc000002000),
                    (13, 0xffffffc000000000, symbols["focos_probe_read_pc"]))
        if len(events) != 3 or output.count("FOCOS_VM_FAULT") != 3:
            raise ValueError("expected exactly three raw page fault records")
        for event, (cause, address, expected_pc), before, after in zip(
                events, expected, ("ALIAS", "READONLY", "NOEXEC"),
                ("READONLY", "NOEXEC", "UNMAP")):
            actual_cause, pc, actual_address = (int(item, 16) for item in event)
            if (actual_cause, actual_address, pc) != (cause, address, expected_pc):
                raise ValueError("page fault record mismatch")
            record = f"FOCOS_VM_FAULT scause={event[0]} sepc={event[1]} stval={event[2]}"
            if not (output.index(f"FOCOS_VM_{before}_OK") < output.index(record)
                    < output.index(f"FOCOS_VM_{after}_OK")):
                raise ValueError("page fault records are out of order")
        if contract == "user-vm":
            user_vm_smoke.validate(output, symbols, *state)
            if vm_symbols is None:
                user_vm_smoke.audit_copy_windows(ELF, symbols)


def write_log(path, version, command, output, result):
    """保存源码版本、未提交状态、ELF 摘要、完整命令及本次运行结果和原始输出。"""
    source = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True,
    ).strip()
    dirty = subprocess.check_output(
        ["git", "status", "--porcelain"], cwd=ROOT, text=True,
    ).strip()
    digest = hashlib.sha256(ELF.read_bytes()).hexdigest()
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        f"Source: {source}{' (dirty)' if dirty else ''}\n"
        f"ELF SHA256: {digest}\n{version}\n"
        f"Command: {shlex.join(command)}\nResult: {result}\n\n{output}",
        encoding="utf-8",
    )


def run_check(contract="boot"):
    """在单核 QEMU 中运行内核；失败或超时保存日志并以非零状态结束检查。"""
    if not ELF.is_file():
        sys.exit("FAIL: kernel ELF missing; run make build")
    name, _ = CONTRACTS[contract]
    log = LOG_PATHS[contract]
    command = [
        "qemu-system-riscv64", "-machine", "virt", "-bios", "default",
        "-m", "128M", "-smp", "1", "-nographic", "-no-reboot",
        "-kernel", str(ELF),
    ]
    version = subprocess.check_output(
        [command[0], "--version"], text=True
    ).splitlines()[0]
    try:
        result = subprocess.run(
            command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            text=True, timeout=20, check=False,
        )
    except subprocess.TimeoutExpired as error:
        output = error.stdout or b""
        if isinstance(output, bytes):
            output = output.decode(errors="replace")
        write_log(log, version, command, output, "TIMEOUT")
        sys.exit(f"FAIL: {name} timed out; see {log}")
    try:
        validate_result(result.stdout, result.returncode, contract)
    except ValueError as error:
        write_log(log, version, command, result.stdout, f"FAIL: {error}")
        sys.exit(f"FAIL: {error}; see {log}")
    write_log(log, version, command, result.stdout, "PASS")
    print(f"PASS: {CONTRACTS[contract][0]} ({version}); log: {log}")


def main():
    """读取检查类型；默认保持 BOOT-001 的命令入口。"""
    parser = argparse.ArgumentParser(description=__doc__)
    group = parser.add_mutually_exclusive_group()
    group.add_argument("--mm", dest="contract", action="store_const", const="mm",
                       help="验收 MM-002 真实物理页读写")
    group.add_argument("--irq", dest="contract", action="store_const", const="irq",
                       help="验收 ARCH-001 本核中断状态保存与恢复")
    group.add_argument("--vm", dest="contract", action="store_const", const="vm",
                       help="验收 MM-003 分页、保护、撤销与生命周期")
    group.add_argument("--user-vm", dest="contract", action="store_const", const="user-vm",
                       help="验收 MM-004 用户根切换、权限、拷贝和表页回收")
    parser.set_defaults(contract="boot")
    run_check(parser.parse_args().contract)


if __name__ == "__main__":
    main()
