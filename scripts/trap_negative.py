#!/usr/bin/env python3
"""构建专用异常负例、核对失败出口，最后恢复普通内核；不接收普通成功标记。"""
import re
import subprocess
import smoke


def validate_failure(output, returncode, mode):
    if returncode not in (0, 1):
        raise ValueError(f"unexpected QEMU termination: {returncode}")
    if "FOCOS_BOOT_OK" in output:
        raise ValueError("negative kernel reported boot success")
    lines = output.splitlines()
    marker = "FOCOS_TRAP_NEGATIVE_UNKNOWN" if mode == "unknown" else "FOCOS_TRAP_REENTRY"
    if lines.count(marker) != 1 or output.count(marker) != 1:
        raise ValueError("missing or duplicate failure marker")
    if mode == "unknown":
        faults = re.findall(r"^FOCOS_VM_UNEXPECTED scause=0x3 sepc=0x[0-9a-f]+ stval=0x[0-9a-f]+$", output, re.MULTILINE)
        if len(faults) != 1 or output.count("FOCOS_VM_UNEXPECTED") != 1 or lines.count("FOCOS_PANIC") != 1:
            raise ValueError("unregistered breakpoint did not use fatal trap path")
        if not output.index(marker) < output.index(faults[0]) < output.index("FOCOS_PANIC"):
            raise ValueError("unexpected trap failure order")
    elif mode == "reentry":
        if "FOCOS_VM_UNEXPECTED" in output or "FOCOS_PANIC" in output:
            raise ValueError("reentry reached ordinary Rust diagnostics")
    else:
        raise ValueError("unknown negative mode")


def build(feature=None):
    command = ["cargo", "build", "--locked", "--release", "--target", "riscv64gc-unknown-none-elf"]
    if feature:
        command += ["--features", feature]
    subprocess.run(command, cwd=smoke.ROOT, check=True)


def main():
    version = subprocess.check_output(["qemu-system-riscv64", "--version"], text=True).splitlines()[0]
    try:
        for mode in ("unknown", "reentry"):
            feature = f"trap-{mode}-test"
            build(feature)
            command = ["qemu-system-riscv64", "-machine", "virt", "-bios", "default",
                       "-m", "128M", "-smp", "1", "-nographic", "-no-reboot", "-kernel", str(smoke.ELF)]
            log = smoke.ROOT / f"target/evidence/trap-negative-{mode}.log"
            try:
                result = subprocess.run(command, capture_output=True, text=True, timeout=20, check=False)
                output = result.stdout + result.stderr
                validate_failure(output, result.returncode, mode)
            except subprocess.TimeoutExpired as error:
                output = error.stdout or b""
                if isinstance(output, bytes):
                    output = output.decode(errors="replace")
                smoke.write_log(log, version, command, output, f"FAIL: TIMEOUT feature={feature}")
                raise
            except ValueError as error:
                smoke.write_log(log, version, command, output, f"FAIL: {error}; feature={feature}")
                raise
            smoke.write_log(log, version, command, output, f"PASS: expected fatal exit; status={result.returncode}; feature={feature}")
            print(f"PASS: {mode} trap rejected; log: {log}")
    finally:
        build()


if __name__ == "__main__":
    main()
