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

需要 netperf 时加 `--network-dummy`：从 APK 额外提取 `dummy.ko`，加载并配置 `dummy0 10.0.0.1/24`。只有 loopback 时，`getaddrinfo("127.0.0.1")` 可能因 `AI_ADDRCONFIG` 返回 `EAI_NONAME`。APK 里没有 `dummy.ko.gz` 时，脚本会在构建阶段直接报错退出，不会生成缺模块的 initramfs。

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

## 启动标记

init 只在对应步骤**确实成功**后才打印标记，失败会先打印 `REF_FAIL <stage>` 再 `poweroff -f`：

| 标记 | 含义 |
| --- | --- |
| `REF_MODULES_OK` | `virtio_ring/virtio/virtio_mmio/virtio_blk/crc16/mbcache/jbd2/ext4` 全部 `insmod` 成功 |
| `REF_MOUNT_OK` | `/dev/vda` 已成功挂载为 ext4 到 `/mnt` |
| `REF RUN <libc> <script>` | 即将在 chroot 中执行某组脚本（不代表该组通过） |
| `REF_ALL_DONE` | 所有 libc × 脚本循环已结束 |
| `REF_FAIL <stage>` | 某步失败，例如 `module-load:ext4`、`mount-ext4`、`bind-proc:glibc` |

判分时只认组段内的 `OS COMP TEST GROUP` 标记；`REF_*` 只用于判断参考环境是否正常启动。

## 参考环境适配

initramfs 会自动做这些**仅用于参考运行**的适配：

- `/bin/sh`、`/bin/busybox`、`/bin/ls` 指向 `/busybox`；
- `chmod +x basic/run-all.sh`（原镜像 0644）；
- `/etc/hosts` + `/etc/nsswitch.conf`；
- 可选 `dummy0`。

这些不是官方脚本的一部分，结果中必须单独标注。

## 结果

2026-11-08 的 11 组摘要见 [tests/results/reference-2026-11-08/summary.md](../../tests/results/reference-2026-11-08/summary.md)。该摘要是判分基线，不是 FOCOS 成绩。
