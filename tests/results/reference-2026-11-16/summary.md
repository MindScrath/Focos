# TEST-001 验证链与参考 Linux 测量（2026-11-16）

本轮验证构建、采集与判分的正确性；**不是 FocOS/VF2 成绩，也不调整区域赛验收口径**。历史 11/08 记录保留在原路径。

## 输入与条件

- QEMU 10.2.2（Fedora 包），Alpine `linux-lts 6.18.63-r0`；每项 1 GiB、1 hart、`-snapshot`，22 次独立启动、串行执行。
- 官方镜像压缩/解压 SHA256 均重新核对，与 `tests/inventory.json` 一致；APK SHA256 为 `47034305e6ddde9717652ce1da25d29b51ae88b997ab7c69b0beeee00ccc9369`。
- 构建脚本默认检查冻结输入，并记录确定性 manifest；原始串口保留 CRLF 与非 UTF-8 字节。symlink/chmod/hosts/网络为参考适配，原脚本与原镜像字节不改。
- Lua 真实构件在不同目录/文件名下重建，内核、initramfs 字节及 manifest 一致；initramfs SHA256 为 `9a9dae26e604791b5710ea0ea8344016a1bc3af0f9c01de69a5edada3f793acc`。该值仅对应本次源码、glibc/Lua 选择及归档工具链。
- 当前重判源码为 `2f4a58a55acf11068982b3c7a0863770983219f1`（包内 `SOURCE.json` 同时列逐文件 SHA256）；最终文档提交不改变这些执行源码。

## 22 次运行

| 组 | libc | 有效通过/清单数 | 组状态 | 环境协议 |
| --- | --- | ---: | --- | --- |
| lua | glibc | 9/9 | PASS | VALID |
| lua | musl | 9/9 | PASS | VALID |
| netperf | glibc | 5/5 | PASS | VALID |
| netperf | musl | 0/5 | FAIL | VALID |
| iperf | glibc | 6/6 | PASS | VALID |
| iperf | musl | 6/6 | PASS | VALID |
| libcbench | glibc | 27/27 | PASS | VALID |
| libcbench | musl | 27/27 | PASS | VALID |
| iozone | glibc | 20/20 | PASS | VALID |
| iozone | musl | 0/20 | FAIL | VALID |
| busybox | glibc | 52/53 | PARTIAL | VALID |
| busybox | musl | 52/53 | PARTIAL | VALID |
| libctest | glibc | 177/220 | PARTIAL | VALID |
| libctest | musl | 105/220 | PARTIAL | VALID |
| basic | glibc | 11/32 | PARTIAL | VALID |
| basic | musl | 0/32 | FAIL | VALID |
| cyclictest | glibc | 4/4 | PASS | VALID |
| cyclictest | musl | 4/4 | PASS | VALID |
| lmbench | glibc | 0/24 | TIMEOUT | ERROR |
| lmbench | musl | 0/24 | TIMEOUT | ERROR |
| unixbench | glibc | 24/27 | PARTIAL | VALID |
| unixbench | musl | 0/27 | FAIL | VALID |

- basic-glibc 另有 18 项部分断言通过；不算完整通过。musl basic、部分性能二进制因镜像/loader 问题无法执行，原样保留失败。
- busybox 的 `kill 10` 与 libctest 的缺失项目仍计在分母，不删项取得通过。
- Unixbench-glibc 的 SHELL1/8/16 未产出判分行；lmbench 双 libc 在 45 秒捕获上限内未结束（原始日志保留具体停点）。TIMEOUT 的观察值可诊断，有效通过数为 0，未完成协议显示 ERROR。
- 这些结果只证明参考运行与判分链实际执行过。未把 glibc 结果外推到 musl；未比较性能或发布 FocOS 总通过率。

## 已运行的验证

- `make check mm-smoke irq-smoke`：52 项 Rust、15 项原有 Python、44 项 TEST-001 测试通过；BOOT-001、MM-002、ARCH-001 QEMU 检查通过。
- `make reference-replay` 的同一入口校验旧归档及 38 个文件；15 份旧摘要逐项与数量一致。旧环境显式标为 `LEGACY_UNVERIFIED`。
- 当前 runner 对本轮 22 份真实捕获按原 bytes、进程退出码与超时标记重判，逐项与采集期结果一致。回放输出标明 `replayed_from`，不声称再次运行 QEMU。
- 真实 QEMU 将虚拟盘替换为 4 MiB 非 ext4 文件：得到 ENV_ERROR/`REF_FAIL mount-ext4`，无 `REF_MOUNT_OK`、`REF RUN` 或 `REF_ALL_DONE`，有效通过为 0。
- 开发机逐项故障注入覆盖必需前置动作；另验模块缺失/错误 release、输入输出路径冲突、CRLF、构件/输入身份、非法 JSON/计数、重复/错配组、异常退出、超时后代终止及判据误报。

## 原始证据与复现

- 当前证据归档（v1 与原始 APK 缓存保留原路径）：[test-001-pr29-validation-2026-11-16-v2](https://github.com/ApolloMonasa/Focos-contrib/releases/tag/test-001-pr29-validation-2026-11-16-v2)。
- 归档 `test-001-pr29-validation-2026-11-16-v2.tar.gz` 的 SHA256：`34c0a8ecd72acedf73152df53e737ba9fad38af6b4174a78984d2fe769c5d5e4`。
- 归档包含原始串口、捕获状态、逐项结果、实际内核/initramfs、manifest、SOURCE.json、SHA256SUMS、确定性对照与坏盘负例。历史原包不覆盖。
- 构建、重跑与协议说明见 [scripts/ref/README.md](../../../scripts/ref/README.md)；包内 README 给出 `verify_current.py` 回放命令，先校验包和内部 SHA256。
- 旧 Alpine 6.18.60 滚动 URL 已不可用；本次明确使用新的冻结输入，不能把新测量说成重建了旧环境。

CI 补充：Ubuntu dash 的 echo 会展开测试桩命令中的反斜杠，现改为 printf 并核对记录条数与调用计数；测试目标和断言未放宽。v2 证据只将源码身份核对限定为实际执行代码，排除测试桩文件；原始串口、构件和判分结果不变，v1 归档不覆盖。

## 新一轮 Codex 审核修复

后续实现修订不覆盖上述旧构建、v2 归档与原始测量。新增负例先在旧实现暴露 30 个失败断言，修复后共 52 项 TEST-001 测试通过。`make check mm-smoke irq-smoke` 再次通过（52 Rust、15 原有 Python 与三个内核检查）。

- 每个已识别的匿名 lmbench 命令开始即预留失败观测；非法、缺失、重复和重试不能消除先前失败，表格测量同样处理。
- PARTIAL 只取决于通过的断言数；正 score 不再把 pass=0 改成部分通过。
- 未知团队用例从结束标记、Unixbench 结果行和 libc-bench 标签收集，不增减冻结分母。
- 清单从同一份字节快照解析和计算 SHA256；清单、源码、判据、参考锁与构件的身份在执行前登记，执行期间变化使结果失效，保留开始时的版本哈希。

当前代码回放 15 份历史摘要和 22 份已有真实捕获，逐项与计数全部一致；这次没有重新运行 22 次 QEMU 测量。源码文件哈希和逐份对照见 [review-fixes.json](review-fixes.json)。旧包内 verify_current.py 按旧 SOURCE.json 固定版本回放；不能把其旧构建哈希视作当前实现重建值。

## 最新未解决评论复核

模块加载的 P1 经实际 APK 核实为依赖方向误判：`virtio_ring` 无依赖，`virtio` 依赖它。保持正确方向，并改为从 APK 的 modules.dep 生成顺序，增加缺失与循环依赖负例；重新构建后实际 QEMU 挂载成功，Lua 双 libc 各 9/9。

另三项实现边界已修复：普通采集须声明并记录被测构件；不兼容的团队清单在采集前拒绝，避免默用默认名称；复跑时清除旧 sections，拒绝执行输入与输出重叠及受管输出符号链接。专项测试共 58 项，完整工程检查和 15 份历史回放通过。具体模块依赖、源码和构件 SHA256 见 [unresolved-fixes.json](unresolved-fixes.json)。未重新运行整轮 22 次测量，旧归档不修改。

## strict 磁盘绑定与日志别名修复

最新审核的两项问题均成立。strict 现在要求实际磁盘，取其当前 SHA256 与清单/manifest 同时绑定并监视变化；capture 仅执行经过路径与固定选项核对的直接 QEMU argv，拒绝额外挂盘、shell 包装及误指向其他构件。输出碰撞检查覆盖全部输入与硬链接别名，保留受控 serial.log 原地回放。

新增负例先在旧实现暴露 4 个失败和 1 个错误；修复后 65 项专项、52 Rust、15 原有 Python 与三个内核检查通过，15 份历史回放一致。使用实际 4 GiB 冻结磁盘重新做 strict QEMU 采集，glibc/musl Lua 均为 9/9（后者从同一双 libc 启动日志重判，非第二次启动）。源码哈希、磁盘哈希和命令核对状态见 [disk-binding.json](disk-binding.json)。没有重跑整轮 22 项测量，旧包不覆盖；旧包内的固定 SOURCE 入口继续对应旧实现。

## 执行工具、构建快照和结果来源修复

最新四项 P2 均按实现遗漏修复：strict QEMU 解析真实文件，记录和监视版本/哈希；构建源码、清单、锁与输入在处理前固定，发布前复查，且禁止输出覆盖实现；脚本退出失败保留同时发生的 JUDGE_ERROR；原生日志回放明确使用 replayed_at/replayed_from，不伪造 captured_at。

新增负例在旧实现暴露 3 个失败和 5 个错误；修复后专项共 70 项，完整工程检查与 15 份历史回放通过。重新构建真实参考环境并采集双 libc Lua，glibc 9/9；musl 从同一日志重判 9/9，原生元数据区分 capture/replay。真实 QEMU 路径、版本、哈希、构建开始时的实现身份见 [runtime-identity.json](runtime-identity.json)。旧归档不覆盖，未重跑 22 次全组测量。


### 新评审后的构建身份重构

构建 ID 未绑定实际产物是设计缺口，现改为 schema 2：ID 包含实际内核哈希及 initramfs 解压归档内容哈希，只有 /init 中唯一的 ID 字段归零。严格回放从实际归档重算，拒绝路径别名、重复、硬链接、截断与尾随归档；修改 manifest 输出字段不能把旧日志换配新构件。schema 1 保留历史记录，但不再作为严格身份证据。

构建输出和 runner 受管输出两两检查路径/inode 冲突，含启动后新建硬链接；已知截断优先于 judge 错误，诊断仍保留，并将 execution_valid 标为 false。新增测试先在旧实现暴露 25 个失败断言，另以 3 个失败断言暴露截断有效性标记遗漏。修复后 78 项专项、52 Rust、15 原有 Python、三个内核 QEMU 检查与 15 份历史回放通过。

同源同输入在不同目录/文件名下重建，内核、initramfs 与完整 manifest 一致。实际 QEMU 新采集 Lua glibc 9/9；musl 从同一双 libc 串口回放为 9/9，不算第二次启动，也没有重跑完整 22 次测量。详见 [结构化验证](payload-binding.json) 与 schema2-lua/ 下原始日志、构建 manifest、采集摘要和回放摘要。其哈希在结构化验证中；旧 Release 未覆盖。

原始 serial.log 已转存外部固定哈希归档；本目录摘要与 validation 的原始哈希保留。下载位置、逐文件路径与校验值统一见 [原始证据索引](../raw-evidence-2026-11-17.json)。库内不再保存含上游版权邮箱的日志，旧判分和原始字节不修改。
