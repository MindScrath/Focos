#!/usr/bin/env python3
"""在 QEMU 模拟 bootelf 的 argc/argv 调用；不代表 U-Boot 或真板通过。"""
import argparse
import pathlib
import struct
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[1]


def check_elf(path):
    # U-Boot v2021.10 的 phdr loader 不过滤 p_type；只交付 PT_LOAD，
    # 否则 attributes 等元数据头可能被当成内存段复制到地址 0。
    data = path.read_bytes()
    if len(data) < 64 or data[:6] != b"\x7fELF\x02\x01":
        sys.exit("FAIL: expected little-endian ELF64")
    header = struct.unpack_from("<16sHHIQQQIHHHHHH", data)
    _, elf_type, machine, _, entry, phoff, _, _, _, phsize, phnum, *_ = header
    if (elf_type != 2 or machine != 243 or entry != 0x80200000
            or phsize != 56 or not phnum or phoff + phsize * phnum > len(data)):
        sys.exit("FAIL: unexpected bootelf ELF header")
    ranges = []
    executable_entry = False
    for i in range(phnum):
        kind, flags, offset, virtual, physical, filesz, memsz, _ = struct.unpack_from(
            "<IIQQQQQQ", data, phoff + i * phsize
        )
        if kind != 1:
            sys.exit("FAIL: old U-Boot requires PT_LOAD-only program headers")
        if (virtual != physical or filesz > memsz or offset + filesz > len(data)
                or not memsz or not 0x80200000 <= physical < physical + memsz <= 0x80300000):
            sys.exit("FAIL: unsafe bootelf segment bounds")
        ranges.append((physical, physical + memsz))
        executable_entry |= bool(flags & 1 and physical <= entry < physical + filesz)
    ranges.sort()
    if not executable_entry or any(a[1] > b[0] for a, b in zip(ranges, ranges[1:])):
        sys.exit("FAIL: overlapping segments or non-executable entry")
    print("PASS: ELF fits old U-Boot loader; PT_LOAD only, bounded segments")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--elf", type=pathlib.Path, default=ROOT / "target/vf2/focos-vf2.elf")
    parser.add_argument("--check-elf-only", action="store_true")
    args = parser.parse_args()
    if not args.elf.is_file():
        sys.exit("FAIL: bootelf ELF missing; run make vf2")
    check_elf(args.elf)
    if args.check_elf_only:
        return
    evidence = ROOT / "target/evidence"
    evidence.mkdir(parents=True, exist_ok=True)
    version = subprocess.check_output(["qemu-system-riscv64", "--version"], text=True).splitlines()[0]
    # QEMU/OpenSBI 进入跳板；跳板模拟 U-Boot 的普通函数调用参数。
    # 非零 argc 能发现把 a0 当作 hart ID 的错误；argv 不被内核使用。
    with tempfile.TemporaryDirectory(prefix="bootelf-", dir=evidence) as directory:
        work = pathlib.Path(directory)
        for argc in (0, 2):
            source = work / "entry.S"
            source.write_text(
                ".section .text\n.globl _start\n_start:\n"
                f"  li a0, {argc}\n  li a1, 0\n"
                "  li t0, 0x80200000\n  jalr ra, t0, 0\n"
                "1: wfi\n  j 1b\n"
            )
            subprocess.run([
                "riscv64-linux-gnu-as", "-march=rv64gc", "-mabi=lp64d",
                "-o", str(work / "entry.o"), str(source),
            ], check=True)
            (work / "linker.ld").write_text(
                "SECTIONS { . = 0x80400000; .text : { *(.text) } }\n"
            )
            subprocess.run([
                "riscv64-linux-gnu-ld", "--no-relax", "-T", str(work / "linker.ld"),
                "-e", "_start", "-o", str(work / "entry.elf"), str(work / "entry.o"),
            ], check=True)
            command = [
                "qemu-system-riscv64", "-machine", "virt", "-bios", "default",
                "-m", "128M", "-smp", "1", "-nographic", "-no-reboot",
                "-kernel", str(work / "entry.elf"),
                "-device", f"loader,file={args.elf.resolve()}",
            ]
            timed_out = False
            try:
                result = subprocess.run(command, stdout=subprocess.PIPE,
                                        stderr=subprocess.STDOUT, timeout=3, check=False)
                output = result.stdout
            except subprocess.TimeoutExpired as error:
                timed_out = True
                output = error.stdout or b""
            log = evidence / f"bootelf-qemu-argc{argc}.log"
            log.write_bytes((version + "\n").encode() + output)
            if (not timed_out or output.count(b"FOCOS_VF2_BOOT_OK") != 1
                    or b"FOCOS_PANIC" in output):
                sys.exit(f"FAIL: simulated bootelf argc={argc}; see {log}")
            print(f"PASS: simulated bootelf argc={argc} (QEMU only); log: {log}")


if __name__ == "__main__":
    main()
