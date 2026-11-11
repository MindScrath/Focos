#!/usr/bin/env python3
"""Build a reference-run initramfs for the official 2025 test image.

This is a reference harness, not part of FOCOS. It extracts a static busybox
from the official image, extracts the Alpine kernel/modules from a pinned apk,
and writes a newc cpio initramfs that mounts the test image and runs the
selected group scripts inside a chroot.

Requirements:
    pip install ext4        # only needed when --busybox is omitted
    QEMU 9.2.4 (xPack or distro package)

Example:
    python scripts/ref/build_initramfs.py \
      --image sdcard-rv.img \
      --kernel-apk linux-lts-6.18.60-r0.apk \
      --out ref/initramfs.cpio.gz --kernel-out ref/Image \
      --libcs glibc musl --scripts basic_testcode.sh lua_testcode.sh
"""

from __future__ import annotations

import argparse
import gzip
import io
import os
import re
import tarfile
from pathlib import Path

MODULES = [
    "failover",
    "net_failover",
    "virtio_ring",
    "virtio",
    "virtio_net",
    "virtio_mmio",
    "virtio_blk",
    "crc16",
    "mbcache",
    "jbd2",
    "ext4",
]
NAME_RE = re.compile(r"^[A-Za-z0-9_.+-]+$")


def extract_busybox(image: Path, explicit: Path | None) -> bytes:
    if explicit:
        return explicit.read_bytes()
    try:
        from ext4 import Volume  # type: ignore
    except ImportError as exc:
        raise SystemExit("install ext4 (pip install ext4) or pass --busybox") from exc
    volume = Volume(image.open("rb"))
    handle = volume.inode_at("/musl/busybox").open()
    try:
        return handle.read()
    finally:
        handle.close()


def extract_kernel_and_modules(apk: Path, release: str, kernel_out: Path, modules_dir: Path) -> list[str]:
    kernel_out.parent.mkdir(parents=True, exist_ok=True)
    modules_dir.mkdir(parents=True, exist_ok=True)
    found: list[str] = []
    with tarfile.open(apk, "r:gz") as tar:
        names = tar.getnames()
        kernel_name = next((n for n in names if n.endswith("boot/vmlinuz-lts")), None)
        if not kernel_name:
            raise SystemExit("boot/vmlinuz-lts not found in apk")
        kernel = gzip.decompress(tar.extractfile(kernel_name).read())
        kernel_out.write_bytes(kernel)

        for module in MODULES:
            candidates = [n for n in names if n.endswith(f"/{module}.ko.gz")]
            if not candidates:
                continue
            data = gzip.decompress(tar.extractfile(candidates[0]).read())
            (modules_dir / f"{module}.ko").write_bytes(data)
            found.append(module)

    if "virtio_ring" not in found or "virtio_blk" not in found or "ext4" not in found:
        raise SystemExit(f"required modules missing, found: {found}")
    return found


def entry(name: str, mode: int, data: bytes, ino: int) -> bytes:
    name_bytes = name.encode() + b"\0"
    header = b"070701" + b"".join(
        f"{value:08x}".encode()
        for value in (ino, mode, 0, 0, 1, 0, len(data), 0, 0, 0, 0, len(name_bytes), 0)
    )
    out = header + name_bytes
    out += b"\0" * ((4 - len(out) % 4) % 4)
    out += data
    out += b"\0" * ((4 - len(data) % 4) % 4)
    return out


def build_init(libcs: list[str], scripts: list[str], modules: list[str], release: str, network: bool) -> str:
    module_lines = "\n".join(
        f'  [ -f /lib/modules/{release}/{m}.ko ] || fail "module-missing:{m}"\n'
        f'  /busybox insmod /lib/modules/{release}/{m}.ko || fail "module-load:{m}"'
        for m in modules
    )
    libc_lines = []
    for libc in libcs:
        libc_lines.append(
            f"""  /busybox mkdir -p /mnt/{libc}/proc /mnt/{libc}/dev /mnt/{libc}/tmp /mnt/{libc}/var/tmp /mnt/{libc}/bin || fail "mkdir:{libc}"
  /busybox mount --bind /proc /mnt/{libc}/proc || fail "bind-proc:{libc}"
  /busybox mount --bind /dev /mnt/{libc}/dev || fail "bind-dev:{libc}"
  /busybox ln -sf /busybox /mnt/{libc}/bin/sh
  /busybox ln -sf /busybox /mnt/{libc}/bin/busybox
  /busybox ln -sf /busybox /mnt/{libc}/bin/ls
  [ -f /mnt/{libc}/basic/run-all.sh ] && /busybox chmod +x /mnt/{libc}/basic/run-all.sh
  /busybox mkdir -p /mnt/{libc}/etc
  /busybox sh -c "echo '127.0.0.1 localhost' > /mnt/{libc}/etc/hosts"
  /busybox sh -c "echo 'hosts: files dns' > /mnt/{libc}/etc/nsswitch.conf"
"""
        )
        if network:
            libc_lines[-1] += "  /busybox ifconfig dummy0 10.0.0.1 netmask 255.255.255.0 up\n"
        for script in scripts:
            libc_lines[-1] += f"  /busybox echo \"REF RUN {libc} {script}\"\n"
            libc_lines[-1] += f"  /busybox chroot /mnt/{libc} /busybox sh /{script}\n"
    body = "\n".join(libc_lines)
    return f"""#!/busybox sh
fail() {{
  /busybox echo "REF_FAIL $1"
  /busybox poweroff -f
  exit 1
}}
/busybox mkdir -p /proc /sys /dev /mnt || fail mkdir-root
/busybox mount -t proc proc /proc || fail mount-proc
/busybox mount -t sysfs sysfs /sys || fail mount-sys
/busybox mount -t devtmpfs devtmpfs /dev || fail mount-dev
{module_lines}
/busybox echo "REF_MODULES_OK"
/busybox mount -t ext4 /dev/vda /mnt || fail mount-ext4
/busybox echo "REF_MOUNT_OK"
{body}
/busybox echo "REF_ALL_DONE"
/busybox poweroff -f
"""


def main() -> int:
    parser = argparse.ArgumentParser(description="build a TEST-001 reference initramfs")
    parser.add_argument("--image", required=True, type=Path, help="official sdcard-rv.img")
    parser.add_argument("--kernel-apk", required=True, type=Path, help="Alpine linux-lts apk")
    parser.add_argument("--out", required=True, type=Path, help="output initramfs.cpio.gz")
    parser.add_argument("--kernel-out", type=Path, default=Path("Image"), help="decompressed kernel Image")
    parser.add_argument("--modules-dir", type=Path, default=Path("modules"), help="extracted .ko directory")
    parser.add_argument("--busybox", type=Path, help="static busybox; default extract /musl/busybox from image")
    parser.add_argument("--release", default="6.18.60-0-lts", help="kernel module directory name")
    parser.add_argument("--libcs", nargs="+", default=["glibc"], help="glibc / musl")
    parser.add_argument("--scripts", nargs="+", required=True, help="group scripts, e.g. basic_testcode.sh")
    parser.add_argument("--network-dummy", action="store_true", help="load dummy0 for netperf getaddrinfo")
    args = parser.parse_args()

    for value in args.libcs + args.scripts:
        if not NAME_RE.match(value):
            raise SystemExit(f"unsafe value: {value!r}")
    if not all(libc in ("glibc", "musl") for libc in args.libcs):
        raise SystemExit("--libcs must be glibc and/or musl")

    modules = extract_kernel_and_modules(args.kernel_apk, args.release, args.kernel_out, args.modules_dir)
    if args.network_dummy and "dummy" not in modules:
        raise SystemExit("--network-dummy requires dummy.ko in the apk")

    busybox = extract_busybox(args.image, args.busybox)
    init = build_init(args.libcs, args.scripts, modules, args.release, args.network_dummy)

    files = {"init": (0o100755, init.encode()), "busybox": (0o100755, busybox)}
    for module in modules:
        path = args.modules_dir / f"{module}.ko"
        files[f"lib/modules/{args.release}/{module}.ko"] = (0o100644, path.read_bytes())

    dirs = {"lib", "lib/modules", f"lib/modules/{args.release}"}
    for path in files:
        parts = path.split("/")[:-1]
        for i in range(1, len(parts) + 1):
            dirs.add("/".join(parts[:i]))

    buffer = io.BytesIO()
    ino = 1
    for directory in sorted(dirs):
        buffer.write(entry(directory, 0o040755, b"", ino))
        ino += 1
    for path, (mode, data) in files.items():
        buffer.write(entry(path, mode, data, ino))
        ino += 1
    buffer.write(entry("TRAILER!!!", 0, b"", 0))

    args.out.parent.mkdir(parents=True, exist_ok=True)
    with gzip.open(args.out, "wb") as handle:
        handle.write(buffer.getvalue())

    print(f"kernel: {args.kernel_out} ({args.kernel_out.stat().st_size} bytes)")
    print(f"initramfs: {args.out} ({args.out.stat().st_size} bytes)")
    print(f"modules: {', '.join(modules)}")
    print("QEMU:")
    print(
        "qemu-system-riscv64 -machine virt -m 1G -smp 1 -nographic -no-reboot -snapshot "
        f"-bios default -kernel {args.kernel_out} -initrd {args.out} "
        '-append "console=ttyS0 rdinit=/init" '
        f"-drive file={args.image},if=none,format=raw,id=x0 -device virtio-blk-device,drive=x0"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
