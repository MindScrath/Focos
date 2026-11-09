# 参考运行 harness

用途：在**参考 Linux**（Alpine 内核）上启动官方 2025 测试镜像，产出可判分的串口日志。它不是 FOCOS，也不替代 FOCOS 验收。

## 依赖

- Python 3.10+
- `pip install ext4`（仅用于从镜像提取 `/musl/busybox`；也可用 `--busybox` 指定）
- QEMU 9.2.4（xPack 或发行版包）

## 输入

| 输入 | 来源 | SHA256 |
| --- | --- | --- |
| 测试镜像 | `oscomp/testsuits-for-oskernel` release `pre-20250615`，`sdcard-rv.img.xz` | 压缩包 `2b59aee4d26681b2d78a8faa615ac216f723a8ea157544e7867bbc8c77aac087`；解压后 `95973543db6b84a9a5e70f30da466ce292867aff5b689fb14c88dc9406e378b8` |
| Alpine 内核 | USTC 镜像 `alpine/edge/main/riscv64/linux-lts-6.18.60-r0.apk` | `06454180443143009db6c36e1bcd9300871c65c5b2d9f41697007db51455994c` |

## 构建

```bash
python scripts/ref/build_initramfs.py \
  --image sdcard-rv.img \
  --kernel-apk linux-lts-6.18.60-r0.apk \
  --out ref/initramfs.cpio.gz \
  --kernel-out ref/Image \
  --modules-dir ref/modules \
  --libcs glibc musl \
  --scripts lua_testcode.sh
```

脚本会：

1. 从 apk 解压 `boot/vmlinuz-lts`（gzip）为 `Image`；
2. 解压 `virtio_ring/virtio/virtio_mmio/virtio_blk/crc16/mbcache/jbd2/ext4` 模块；
3. 从测试镜像提取 `/musl/busybox`，或使用 `--busybox`；
4. 生成 newc cpio + gzip 的 initramfs；
5. 打印可直接运行的 QEMU 命令。

需要 netperf 时加 `--network-dummy`：加载 `dummy.ko` 并配置 `dummy0 10.0.0.1/24`。只有 loopback 时，`getaddrinfo("127.0.0.1")` 可能因 `AI_ADDRCONFIG` 返回 `EAI_NONAME`。

## 运行

```bash
qemu-system-riscv64 \
  -machine virt -m 1G -smp 1 -nographic -no-reboot -snapshot \
  -bios default -kernel ref/Image -initrd ref/initramfs.cpio.gz \
  -append "console=ttyS0 rdinit=/init" \
  -drive file=sdcard-rv.img,if=none,format=raw,id=x0 \
  -device virtio-blk-device,drive=x0
```

`-snapshot` 把写入放进临时 overlay，原始镜像不会被修改。串口日志用 `>` 重定向到文件，再交给 `scripts/run_tests.py --log` 判分。

## 参考环境适配

initramfs 会自动做这些**仅用于参考运行**的适配：

- `/bin/sh`、`/bin/busybox`、`/bin/ls` 指向 `/busybox`；
- `chmod +x basic/run-all.sh`（原镜像 0644）；
- `/etc/hosts` + `/etc/nsswitch.conf`；
- 可选 `dummy0`。

这些不是官方脚本的一部分，结果中必须单独标注。

## 结果

2026-11-08 的 11 组摘要见 [tests/results/reference-2026-11-08/summary.md](../../tests/results/reference-2026-11-08/summary.md)。该摘要是判分基线，不是 FOCOS 成绩。
