# TEST-001 参考运行：Linux/QEMU 判分基线

关联：[Issue #8](https://github.com/MindScrath/Focos/issues/8) · 日期：2026-11-08 · 性质：判分管线验证，不是 FOCOS 成绩

本文记录用**参考 Linux**（Alpine 内核 + 官方测试镜像）跑 11 组的真实串口输出，以及 runner/judge 在真实输出上的行为。目的是确认“判分能不能信”，不是证明 FOCOS 兼容性。FOCOS 的结果必须另跑、另列。

## 一句话结论

11 组都能在参考环境产出可判分的真实输出；5 个可作 stdin 判分的官方 judge + 6 个团队判据均未崩溃，缺项按 0 计（lmbench 使用团队解析器；官方 `lmbench.sh` 是运行器，不是 stdin 判分器）。发现 3 个镜像/判分侧的硬问题：basic 的 musl 二进制实际是 glibc 构建、busybox judge 多一个 `kill 10`、libctest 镜像少 3 项。参考环境的完整结果见第 3 节。

## 1. 参考环境

| 项 | 值 |
| --- | --- |
| QEMU | xPack QEMU 9.2.4（win32-x64），OpenSBI 1.5.1 |
| QEMU zip SHA256 | `05df8a50c2109605c7ab382667aa4680d41cb262383053e5bb07d29de616eb1f` |
| 内核 | Alpine `linux-lts 6.18.60-r0`，从 USTC 镜像取 apk |
| 内核 apk SHA256 | `06454180443143009db6c36e1bcd9300871c65c5b2d9f41697007db51455994c` |
| 解压后 `Image` SHA256 | `d80d8505dfd15de0f656b4a4b5c2e3213dcbc1dc427898288cb1527751084928` |
| 测试镜像 | `pre-20250615/sdcard-rv.img`，解压后 SHA256 `95973543db6b84a9a5e70f30da466ce292867aff5b689fb14c88dc9406e378b8` |
| initramfs | `scripts/ref/build_initramfs.py` 生成；2026-11-08 结果的构建 SHA256 `f579c71226da0039bf309853454ac8c7b2369da93972219bfaad0eecfbabaed8`，2026-11-11 修正启动标记后重建为 `9aea76713981d385b7243b2921e2957d280f3eaffb3153736b65e367cfb1fc3b`（lua 双 libc 复跑 9/9） |

启动命令（关键参数）：

```bash
qemu-system-riscv64 \
  -machine virt -m 1G -smp 1 -nographic -no-reboot -snapshot \
  -bios default -kernel Image -initrd initramfs.cpio.gz \
  -append "console=ttyS0 rdinit=/init" \
  -drive file=sdcard-rv.img,if=none,format=raw,id=x0 \
  -device virtio-blk-device,drive=x0
```

initramfs 内含：测试镜像里的静态 `busybox`、Alpine 6.18.60 的 `virtio_ring/virtio/virtio_mmio/virtio_blk/crc16/mbcache/jbd2/ext4/dummy` 模块，以及一个 `/init`：挂 proc/sys/dev，`insmod` 模块，挂载 `/dev/vda` 到 `/mnt`，`chroot` 进 `/mnt/glibc` 或 `/mnt/musl` 执行官方脚本。每个步骤都带 `|| fail`：只有确实成功才打印 `REF_MODULES_OK` / `REF_MOUNT_OK`，失败打印 `REF_FAIL <stage>` 并关机，不会出现“模块加载或挂载失败但仍然打印 OK”的情况（标记含义见 [scripts/ref/README.md](../../scripts/ref/README.md)）。

## 2. 参考环境做的打包适配（仅用于参考运行）

这些不是官方脚本的一部分，必须在结果里单独标注：

- `/bin/sh`、`/bin/busybox`、`/bin/ls` 指向 `/busybox`：镜像只提供根目录的 `busybox`，而 `test.sh` 的 shebang 是 `#!/bin/busybox sh`，`basic/run-all.sh` 也没有执行位（0644）。
- `chmod +x basic/run-all.sh`：镜像内该文件权限为 0644，Linux 直接执行会 `Permission denied`。
- `/etc/hosts` + `/etc/nsswitch.conf`：netperf 的 `getaddrinfo` 需要。
- `dummy0` + `10.0.0.1/24`：xPack QEMU 没有编译 `user` 网络后端；只有 loopback 时 `AI_ADDRCONFIG` 会让 `getaddrinfo("127.0.0.1")` 返回 `EAI_NONAME`。加一个 dummy 网卡后 netperf 才通过。
- `-snapshot`：所有写入进临时 overlay，原始镜像不被修改。

## 3. 结果

只跑 glibc 的组是为节省时间；basic/libctest/busybox/lua 跑了 musl 和 glibc。

| 组 | libc | 结果 | 组状态 | 说明 |
| --- | --- | --- | --- | --- |
| basic | glibc | 11 PASS / 18 PARTIAL / 3 FAIL（32 项） | PARTIAL | 失败：`test_dup2`、`test_mount`、`test_umount`；PARTIAL 是部分断言通过，不算通过 |
| basic | musl | 0/32 | FAIL | musl 目录里的 basic 二进制与 glibc 完全相同，且 `/musl/lib` 没有 glibc loader，无法执行 |
| busybox | musl | 52/53 | PARTIAL | 只差 judge-only 的 `kill 10`；这是当前镜像 + judge 的理论上限 |
| busybox | glibc | 51/53 | PARTIAL | `kill 10` + `du`（`du` 可能是环境抖动，复跑有机会到 52） |
| lua | musl | 9/9 | PASS | 官方 judge |
| lua | glibc | 9/9 | PASS | 官方 judge |
| libctest | glibc | 177/220 | PARTIAL | 43 项失败，含镜像缺失的 `crypt`/`pleval`，以及依赖 locale/网络/环境的项 |
| libctest | musl | 105/220 | PARTIAL | 静态项多数通过；动态项因 musl 目录缺 glibc loader 失败 |
| iozone | glibc | 20/20 | PASS | 官方 judge；真实吞吐全部为正 |
| unixbench | glibc | 24/27 | PARTIAL | `SHELL1/8/16` 未产出有效判分行；其余 24 项有正数 |
| iperf | glibc | 6/6 | PASS | loopback |
| libcbench | glibc | 27/27 | PASS | 团队 judge；27 个内部 benchmark 都有 `time:` |
| lmbench | glibc | 11/24 后 TIMEOUT | TIMEOUT | 前 11 项（syscall/read/write/stat/fstat/open/select/sig×3/pipe）解析正确；`lat_proc fork` 在参考环境挂住 |
| netperf | glibc | 5/5 | PASS | 加 `dummy0` 后 loopback 全部成功 |
| cyclictest | glibc | 4/4 | PASS | 四个场景均 success |

## 4. 关键发现

1. **basic 的 musl 是假的 musl**：`/musl/basic/{brk,chdir,getpid}` 与 `/glibc/basic/...` 的 SHA256 完全相同；`/musl/lib` 只有 `libc.so`，没有 `ld-linux-riscv64-lp64d.so.1`。参考运行 musl basic 0/32。这是镜像构建问题，不是 FOCOS 问题；验收时不能把 musl basic 当成独立一套。
2. **镜像缺执行位/路径**：`basic/run-all.sh` 0644；`test.sh` 的 shebang 指向 `/bin/busybox`，镜像没有 `/bin/busybox`。参考运行用 symlink + chmod 适配；这说明“原版镜像直接跑”在 Linux 上也不成立，必须记录适配。
3. **busybox judge 与镜像不同步**：judge 期望 `kill 10`（镜像没有），镜像多 `printf` 和 `sh -c 'sleep 5' & ./busybox kill $!`（judge 不计分）。musl 实测 52/53，正好是上限。
4. **libctest 分母缺口**：镜像 107 static + 110 dynamic = 217；judge 109 + 111 = 220；`crypt`/`pleval` 在 `entry-static.exe` 中也搜不到。glibc 177/220、musl 105/220 是真实参考结果，不是 FOCOS 成绩。
5. **lmbench 的官方 judge 不是 stdin 判分器**：`judge/lmbench.sh` 会执行命令；团队解析器按真实标签校准后，前 11 项正确。`lat_proc fork` 在最小参考环境挂住，说明后续项需要完整进程/文件环境。
6. **unixbench 的 SHELL 项**：`SHELL1/8/16` 没有输出 `Unixbench ... test(lpm):` 行；团队判据按缺项 0 分处理，24/27。需要真实 FOCOS 输出再确认是环境问题还是实现问题。
7. **netperf 的 `AI_ADDRCONFIG`**：只有 loopback 时 `getaddrinfo("127.0.0.1")` 返回 `EAI_NONAME`；加 `dummy0` 后 5/5。这说明运行环境需要至少一个非 loopback IPv4 地址。
8. **判分器稳定性**：11 组、5 个 stdin 官方 judge + 6 个团队判据（lmbench 用团队解析器），全部无 `JUDGE_ERROR`；缺组/缺输出/START 无 END 的负向行为由 PR #2 的 `scripts/tests/test_runner.py` 覆盖（该文件不在本 PR，以标签 `test-001-runner-2026-11-08` 为准）。

## 5. 这些结果不能怎么用

- 不能当作 FOCOS 的通过率；FOCOS 还没有在 QEMU 里跑这些组。
- 不能当作“原版镜像直接兼容”的证据；参考运行做了 symlink、chmod、hosts、dummy0 等适配。
- 不能把 glibc 的结果外推到 musl；basic/libctest 的 musl 结果被镜像构建问题污染。
- 不能把 lmbench 的 11/24 当作完整判分；它是 TIMEOUT 前的部分结果。

## 6. 复现与后续

参考运行的构建脚本已入库：[scripts/ref/build_initramfs.py](../../scripts/ref/build_initramfs.py)，使用说明见 [scripts/ref/README.md](../../scripts/ref/README.md)；2026-11-08 的 11 组摘要见 [tests/results/reference-2026-11-08/summary.md](../../tests/results/reference-2026-11-08/summary.md)。第三方 QEMU/内核二进制不入库，只锁 URL/SHA256。原始串口日志、逐项判分结果（`summary.json`）、运行参数和校验值已归档到 [test-001-evidence-2026-11-08](https://github.com/Weibo-Hd/Focos/releases/tag/test-001-evidence-2026-11-08)，归档 SHA256 `8e55f993507a9cd7eb4ad9d01dd6f4f0c2b243db5e470604c3b9836bb703c8fd`；重跑会产生新测量值，不能替代这份历史记录。

FOCOS 侧的下一步：

- [ ] QEMU 中启动 FOCOS + 官方镜像，先跑 basic 单例（P5.1，依赖 #5/#7）。
- [ ] 用同一 runner 跑 basic/busybox/lua，产出第一张 FOCOS 结果表。
- [ ] 失败项按能力 owner 分派；不要用参考 Linux 的分数替代 FOCOS 结果。
