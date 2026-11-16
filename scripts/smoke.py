#!/usr/bin/env python3
"""验收启动；--mm 检查真实页，--irq 检查中断状态，--vm 检查 MM-003 分页。"""
import argparse
import hashlib
import pathlib
import re
import shlex
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
ELF = ROOT / "target/riscv64gc-unknown-none-elf/release/focos-kernel"
LOG = ROOT / "target/evidence/boot-qemu-rv.log"
MM_LOG = ROOT / "target/evidence/mm-qemu-rv.log"
IRQ_LOG = ROOT / "target/evidence/irq-qemu-rv.log"
VM_LOG = ROOT / "target/evidence/vm-qemu-rv.log"

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
LOG_PATHS = {"boot": LOG, "mm": MM_LOG, "irq": IRQ_LOG, "vm": VM_LOG}


def validate_result(output, returncode, contract="boot"):
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
    if contract == "vm":
        events = re.findall(
            r"^FOCOS_VM_FAULT scause=(0x[0-9a-f]+) sepc=(0x[0-9a-f]+) "
            r"stval=(0x[0-9a-f]+)$", output, re.MULTILINE,
        )
        expected = ((15, 0xffffffc000001000), (12, 0xffffffc000002000),
                    (13, 0xffffffc000000000))
        if len(events) != 3 or output.count("FOCOS_VM_FAULT") != 3:
            raise ValueError("expected exactly three raw page fault records")
        for event, (cause, address), before, after in zip(
                events, expected, ("ALIAS", "READONLY", "NOEXEC"),
                ("READONLY", "NOEXEC", "UNMAP")):
            actual_cause, pc, actual_address = (int(item, 16) for item in event)
            if (actual_cause, actual_address) != (cause, address) or not pc:
                raise ValueError("page fault record mismatch")
            if cause == 12 and pc != address:
                raise ValueError("instruction fault PC mismatch")
            record = f"FOCOS_VM_FAULT scause={event[0]} sepc={event[1]} stval={event[2]}"
            if not (output.index(f"FOCOS_VM_{before}_OK") < output.index(record)
                    < output.index(f"FOCOS_VM_{after}_OK")):
                raise ValueError("page fault records are out of order")


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
    parser.set_defaults(contract="boot")
    run_check(parser.parse_args().contract)


if __name__ == "__main__":
    main()
