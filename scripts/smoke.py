#!/usr/bin/env python3
"""验收 BOOT-001；--mm 额外验收 MM-002 的 QEMU 真实页读写。"""
import argparse
import hashlib
import pathlib
import shlex
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
ELF = ROOT / "target/riscv64gc-unknown-none-elf/release/focos-kernel"
LOG = ROOT / "target/evidence/boot-qemu-rv.log"
MM_LOG = ROOT / "target/evidence/mm-qemu-rv.log"


def validate_result(output, returncode, memory=False):
    """核对退出状态、panic 和唯一成功标记；内存验收还要求先自检后报启动成功。"""
    if returncode != 0:
        raise ValueError(f"QEMU exited with status {returncode}")
    if "FOCOS_PANIC" in output:
        raise ValueError("kernel reported FOCOS_PANIC")

    markers = ("FOCOS_MM_OK", "FOCOS_BOOT_OK") if memory else ("FOCOS_BOOT_OK",)
    lines = output.splitlines()
    for marker in markers:
        if output.count(marker) != 1 or lines.count(marker) != 1:
            raise ValueError(f"expected exactly one standalone {marker}")
    if memory:
        if lines.index("FOCOS_MM_OK") > lines.index("FOCOS_BOOT_OK"):
            raise ValueError("memory self-test must finish before boot success")


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


def run_check(memory=False):
    """在单核 QEMU 中运行内核；失败或超时保存日志并以非零状态结束检查。"""
    if not ELF.is_file():
        sys.exit("FAIL: kernel ELF missing; run make build")
    log = MM_LOG if memory else LOG
    contract = "MM-002" if memory else "BOOT-001"
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
        sys.exit(f"FAIL: {contract} timed out; see {log}")
    try:
        validate_result(result.stdout, result.returncode, memory)
    except ValueError as error:
        write_log(log, version, command, result.stdout, f"FAIL: {error}")
        sys.exit(f"FAIL: {error}; see {log}")
    write_log(log, version, command, result.stdout, "PASS")
    print(f"PASS: {contract} ({version}); log: {log}")


def main():
    """读取检查类型；默认保持 BOOT-001 的命令入口。"""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mm", action="store_true", help="同时验收真实物理页读写")
    run_check(parser.parse_args().mm)


if __name__ == "__main__":
    main()
