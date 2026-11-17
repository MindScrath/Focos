# TEST-001：串口日志驱动的测试执行与判分

关联：[Issue #8](https://github.com/MindScrath/Focos/issues/8) · 状态：修订待审

## 要做到什么

冻结 2025 初赛非 LTP 的 11 组、427 项输入，把串口日志转换成可追溯的逐项结果。参考 Linux 用于校准判分，不计为 FocOS 成绩。构建、runner、团队判据和验证入口随本 PR 一起交付。

## 行为约定

- **采集构件**：`--capture-cmd` 必须通过可重复的 `--capture-artifact` 或参考构件参数声明被测文件；启动前记录 SHA256 并监视变化。调用方须列出实际使用的内核、镜像等文件，不自动猜测 shell 命令。
- **清单兼容**：外部清单可更改官方组期望项；所选团队组的名称顺序和模式必须与仓库冻结清单一致，不支持的团队快照在执行前退出 2，不能混用默认判据。
- **输出复用**：重新生成 `sections/`，只保留本轮段落；拒绝所有输入与受管摘要重叠（含硬链接/符号链接），受管输出彼此也不能共享路径或 inode；仅允许普通 serial.log 原地回放并保留字节。启动前与落盘前都检查。
- **Python 启动**：文件 CLI 的首次项目导入前隔离到私有空 pycache，并禁止写缓存；judge 子进程和测试启动器同样隔离，不能仅依赖 -B。
- **输入与来源**：源码、镜像和官方 judge 以 `tests/inventory.json` 为准；参考环境与历史证据以 `tests/reference.lock.json` 为准。记录输入、原始日志、判据及执行代码的 SHA256，官方 judge 必须匹配冻结文件。清单从同一份字节快照解析并计算哈希；清单、源码、判分器、参考锁和构件在采集前登记；实现身份在耗时构件读取前固定。大构件按 inode 去重，运行前后完整哈希，逐组检查路径绑定与文件属性；小型源码/判据仍逐组哈希。运行中检测到变化均为 ENV_ERROR，不能归因给新版本。原始日志逐字节保留。
- **构建**：默认校验镜像和 APK；APK 以同一字节快照校验并解包，kernel/modules 的身份和打包均使用该不可变字节，不从共享输出回读。BusyBox 读取核对实际文件描述符与冻结输入。固定 cpio 元数据、gzip 时间戳和文件名。实现、清单和锁从构建开始固定身份，输入处理后与发布 manifest 前检查未变化；输出不得覆盖输入、实现、配置或其他输出（含硬链接）。产物通过私有文件原子创建，仅复用相同内容；不同内容需新目录，manifest 最后发布。相同输入、参数及压缩工具链，在不同时间和目录构建得到相同内核/initramfs 哈希。显式允许未冻结输入时，manifest 必须标记；不能作为冻结环境的证据。
- **环境准备**：所有必需的模块、挂载、目录、链接、权限、配置、网络及 chroot 预检失败，输出 `REF_FAIL <stage>` 并停止，不执行后续组。测试脚本非零退出单独记录，不冒充环境失败或成功。
- **组切分**：严格配对 `#### OS COMP TEST GROUP START/END <group>-<libc> ####`；不带后缀时用必填的 `--libc`。重复、嵌套、错配标记是 `LOG_ERROR`，禁止选择一段覆盖其他段；EOF 前未结束是 `TIMEOUT`。
- **判分**：启动采集/回放前，把 judge 目录内的源文件、包内 helper、数据及共用清单依赖冻结为已校验字节；子进程只执行私有副本，不按原路径重开。副本只读且隔离 PYTHONPATH/pycache；原始文件仍监视变化。固定官方或团队 Python judge 用 stdin 接收组段，stdout 返回 JSON 列表；不设 fallback。非法字段、重复名字、非有限数值、非法计数、崩溃、超时均为 `JUDGE_ERROR`。官方 `total` 兼容为 `all`；iozone 按有效正 `result` 判完成。
- **判据**：lmbench 每条冻结命令（含带标签指标）或无 trace 表头先预留失败观测，带标签结果消费对应预留项；trace 后首个表头不重复计数，缺失尝试不能被后续重试覆盖。团队性能解析器只接受约定格式、单位和有效数值。错误消息中的关键词、数字、耗时或部分输出不构成成功。重复/冲突结果不能用后一次成功覆盖失败。
- **状态**：`PASS` 仅限清单内每项 `pass == all > 0`；部分断言通过为 `PARTIAL`，无通过为 `FAIL`。缺组为 `NOT_RUN`，缺项为 `MISSING`（0 分）。另列 `TIMEOUT`、`JUDGE_ERROR`、`LOG_ERROR`、`ENV_ERROR`、`CAPTURE_ERROR`；这些均不能通过。状态统一归并：明确环境/来源错误优先；capture 超时或段 EOF 截断优先于协议未完成、非零 capture 退出及判分器错误；结构错误优先于 capture 退出。合法协议前缀未完成保留诊断且 execution_valid 为 false，错误 ID/顺序及 REF_FAIL 仍失效。退出码和 judge_error 不丢失。脚本非零退出不得覆盖 JUDGE_ERROR 等不可判分状态。
- **参考协议**：新 init 输出版本、准备、运行及退出码标记；严格模式必须提供 manifest 与内核/initramfs，另须提供实际磁盘 --reference-image 并核对其 SHA256；capture 命令的 kernel/initrd/drive 必须与声明一致，仅允许直接 QEMU argv；实际可执行文件经 PATH 解析、规范化，记录版本探测输出与哈希并监视文件变化。核对构建 ID、冻结输入、构件哈希、顺序与所选组；环境错误使结果失效。旧日志只能显式使用 legacy 模式，标记环境未经新协议验证；不据此声称新构建通过。详见[参考运行说明](../scripts/ref/README.md)。
- **输出**：`summary.json`、`summary.md`、原始 `serial.log`、`sections/<group>-<libc>.log`；额外 judge 名称与未知用例另列（含 iperf/netperf/cyclictest 结束标记、Unixbench 结果名、libc-bench 标签以及 Lua/busybox testcase 行和 lmbench trace 命令），不进入分母。每项保留 `name/status/all/pass/score`，不同单位的分数不汇总。采集使用 mode=capture/captured_at；日志回放使用 mode=replay/replayed_at/replayed_from，不伪造采集时间。
- **构建身份**：严格模式使用 schema 2 manifest，构建器、inventory/manifest 实现及 BusyBox 的 SHA256 四字段必填；BusyBox 还须匹配归档内容。嵌入式 ID 绑定实际内核及 initramfs 全部解压内容，只将 /init 内唯一的 ID 字段归零以解除自引用。校验时从实际归档重算，拒绝重名、路径别名、硬链接、截断和尾随归档。改写 manifest 的输出哈希不能为旧日志换配新构件。schema 1 不作为严格证据。
- **历史证据**：整轮回放固定全部 runner/判据依赖集合与哈希，各子回放前后检查，子摘要执行哈希须与整轮一致；含邮箱的原始串口放在固定哈希外部归档，库内保留路径、逐文件哈希和归档索引，判分摘要仍绑定原始字节。隔离解包并校验全部文件后发布，拒绝目标及祖先符号链接；已有不同内容不覆盖。每次回放须生成新摘要。诊断增补绑定固定归档、样本、组和精确前后值，只允许 unexpected_in_log；不允许改状态、分数、通过数，其他差异使检查失败。
- **退出码**：显式 strict 的 manifest、内核/initramfs、image 任一缺失时，读取日志前退出 2。显式 manifest 的结构、来源字段、冻结输入、选择和实际内核/initramfs 内容在采集/回放前统一校验；非法或不匹配退出 2，先于 QEMU 版本探测、采集与判分。0 = 所选组全部通过且运行有效；1 = 非通过或运行无效；2 = 参数、文件或清单错误。捕获命令非零退出不得报告通过；超时终止整个子进程组。

## 怎样验收

| 场景 | 入口 | 应看到什么 |
| --- | --- | --- |
| 环境故障注入 | `make test-test001` | 每个必需步骤失败均停止；无后续执行/完成标记 |
| 重复构建与缺模块 | 同上 | 改时间/输出名哈希不变；缺必需模块、错误 release 构建失败 |
| 判分负例 | 同上 | 错误文本、错单位、NaN/Inf、非法计数、重复结果均不通过 |
| 执行与日志负例 | 同上 | 非零退出、超时、错 libc、嵌套/重复组、环境错误均不通过；原始字节不变 |
| 清单一致性 | `python3 scripts/sync_test_plan.py --check` | 计划、生成表格与冻结清单一致；FocOS 执行状态不冒填 |
| 历史日志回放 | `make reference-replay` | 校验归档及内部哈希，回放 15 份结果；差异必须显式列出，禁止改旧证据迎合新判据 |
| 工程回归 | `make check mm-smoke irq-smoke` | 新检查和现有内核验收均通过 |

输入和判据细节见 [test-inventory](../docs/reference/test-inventory.md)，历史测量见 [reference-run](../docs/reference/reference-run.md)。FocOS 用户态和真板执行仍依赖 #2/#4/#5/#7/#22/#23/#24。本改动不涉及 HAL/ABI/unsafe/持久化语义；普通 PR 需一名非作者批准，CI 必过集合变化另需一名成员确认。
