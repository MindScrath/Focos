# TEST-001 参考运行摘要（2026-11-08）

参考 Linux：QEMU 9.2.4 + Alpine `linux-lts 6.18.60-r0` + 官方镜像 `pre-20250615`。**这是判分基线，不是 FOCOS 成绩。**

环境哈希、构建命令和适配项见 [docs/reference/reference-run.md](../../../docs/reference/reference-run.md) 与 [scripts/ref/README.md](../../../scripts/ref/README.md)。

| 组 | libc | 结果 | 组状态 | 说明 |
| --- | --- | --- | --- | --- |
| basic | glibc | 11 PASS / 18 PARTIAL / 3 FAIL（32） | PARTIAL | 失败：`test_dup2`、`test_mount`、`test_umount` |
| basic | musl | 0/32 | FAIL | musl basic 与 glibc 二进制相同，且 `/musl/lib` 缺 glibc loader |
| busybox | musl | 52/53 | PARTIAL | 只差 judge-only 的 `kill 10` |
| busybox | glibc | 51/53 | PARTIAL | `kill 10` + `du`（可能抖动） |
| lua | musl / glibc | 9/9 + 9/9 | PASS | 官方 judge |
| libctest | glibc | 177/220 | PARTIAL | 含镜像缺失的 `crypt`/`pleval` 与环境相关项 |
| libctest | musl | 105/220 | PARTIAL | 动态项因缺少 loader 失败 |
| iozone | glibc | 20/20 | PASS | 官方 judge，真实吞吐 |
| unixbench | glibc | 24/27 | PARTIAL | `SHELL1/8/16` 无判分行 |
| iperf | glibc | 6/6 | PASS | loopback |
| libcbench | glibc | 27/27 | PASS | 团队判据 |
| lmbench | glibc | 11/24 后 TIMEOUT | TIMEOUT | `lat_proc fork` 在最小参考环境挂住 |
| netperf | glibc | 5/5 | PASS | 需要 `dummy0` |
| cyclictest | glibc | 4/4 | PASS | 四个场景 success |

## 边界

- 只跑 glibc 的性能组是为节省时间；basic/libctest/busybox/lua 跑了双 libc。
- 原始串口日志含二进制/体积较大，未入库；本摘要与 `reference-run.md` 是提交的证据。需要原始日志时按 `scripts/ref/README.md` 重跑。
- 参考运行做了 symlink、chmod、hosts、dummy0 适配，不能当作“原版镜像直接兼容”的证据。
- 不能把 glibc 结果外推到 musl；basic/libctest 的 musl 结果被镜像构建问题污染。
