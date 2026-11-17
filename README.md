# FocOS

FocOS（读作 Focus）是 Rust `no_std` 操作系统。我们要在 VisionFive2 上运行常用 Linux 程序，做好 FAT32 文件访问，再研究文件缓存如何帮助有限内存下的 AI 记忆检索。

## 区域赛要交什么

- **兼容性**：2025 初赛除 LTP 的 11 组测试，RISC-V 的 musl/glibc 都通过。
- **可用性**：vim 编辑保存，gcc 编译并运行新程序，make 完整与增量构建；在 QEMU RISC-V 和 VisionFive2 分别演示。
- **内核与存储**：真板自有设备驱动、内存与任务、用户态；FAT32 同步后重启读回。Ext4 后端运行历史原版镜像。
- **实验**：真板 FAT32 基线与缓存对照，有原始数据和可重画的图表。

准确范围见 [REGION-001](specs/REGION-001.md)。LoongArch 双平台、完整 LTP、2026 复杂负载和真实磁盘 HNSW 查询留到全国赛。新目标工期风险较高，检查点见 [路线](docs/roadmap.md)。


## 当前到哪里了

main 已有 QEMU 启动、SBI 输出、关机和 CI。2026-11-05 的首次 VisionFive2 启动证据见 [PR #16](https://github.com/MindScrath/Focos/pull/16)，它不代表自有 UART/SD 已完成。

本分支新增：统一块设备接口与内存盘，开发机测试覆盖单块读写、边界和同步错误。运行 `make test` 单独验证；`make check` 同时执行这些测试和原有启动检查。内存盘不提供断电持久化，真实 SD 驱动另行实现。行为约定见 [HAL-001](specs/HAL-001.md)。

共享块接口检查按 [TEST-002](specs/TEST-002.md) 接收独立镜像模型，复用已知数据、边界、写后读和同步判据。当前仅完成 host 接入；有限读回观察不替代真实驱动、DMA 安全或重启持久化验收。

本分支已实现 [MM-001](specs/MM-001.md) 的页分配、回收与错误处理，并按 [MM-002](specs/MM-002.md) 接入 QEMU 真实内存，完成整页写入、读回、释放和再次分配。运行 `make test` 验证开发机行为，`make mm-smoke` 验证 QEMU 内存自检。页表、用户映射与真板内存接入尚未实现，本阶段仍待队友审阅与验收。

TEST-001 已冻结 11 组、427 项输入，并提供确定性参考构建、串口 runner 和 6 个团队判据。`make test-test001` 检查故障处理、日志与判分契约，`make reference-replay` 校验并回放历史证据。使用见[参考运行入口](scripts/ref/README.md)，判分边界见 [TEST-001](specs/TEST-001.md)；FocOS 的 QEMU/真板测例结果仍未产生。

本分支新增可嵌套的本核临界区 `with_local_irqs_disabled`：保存并恢复 RISC-V `sstatus.SIE`，闭包只执行一次并透传返回值。单层/两层与初态开/关由 `make irq-smoke` 的 QEMU 状态自检核对；自检入口为仅限启动期的 `unsafe` 接口，`make test` 编译调用方验证此边界。编译器屏障的静态审阅和负例见[参考页](docs/reference/local-irq-critical-section.md)。真实定时中断屏蔽仍待 #2/#5 的时钟与异常入口联调。

内核分页的下一阶段约定见 [MM-003 草案](specs/MM-003.md)，页表、映射和异常验收细节见[参考页](docs/reference/sv39-paging.md)。内核分页与 `make vm-smoke` 均待实现。

接下来：VisionFive2 启动、内存管理、任务与用户程序、SD 驱动、FAT32。它们目前都还没有验收通过。历史官方测例和性能实验也还没有运行结果。

**历史测试、开发工具和文件系统性能均无通过成绩。** 本页目标不是已有功能。

## 第一天怎么做

1. 按 [上手指南](docs/getting-started.md) 运行 `make check`，看到 `PASS: BOOT-001`。
2. 到 [任务入口](docs/backlog.md) 认领一小步，约队友审阅。
3. 用 [SPEC 模板](specs/TEMPLATE.md) 约定行为，按 [六步 SOP](docs/collaboration.md) 实现并提交 PR。

| 想了解什么 | 去哪里 |
| --- | --- |
| 分层与平台扩展 | [架构与职责](docs/architecture.md) · [平台推进顺序](docs/roadmap.md#多平台怎样推进) |
| 测试版本和通过标准 | [功能验证](docs/verification.md) |
| 缓存实验与图表 | [性能实验](docs/benchmarks.md) |
| 赛规、来源与 AI 记录 | [来源](docs/references.md) · [AI 使用](docs/ai-usage.md) |

## 仓库结构

代码看 `kernel/`，行为看 `specs/`，操作入口看 `docs/`。以下列出本分支纳入版本管理的路径；待合入的其他 PR 以各自文件树为准。

```text
Focos/
├── README.md                          # 项目首页和成员入口
├── AGENTS.md                          # 开发、文档与审阅约定
├── kernel/                            # Rust 裸机内核
│   ├── Cargo.toml                     # 内核可执行文件与通用库的构建定义
│   ├── build.rs                       # 校验裸机目标、选择板型并跟踪链接脚本
│   └── src/                           # 内核源代码
│       ├── main.rs                    # Rust 入口和 panic 处理
│       ├── lib.rs                     # no_std 组件、平台接口与开发机测试入口
│       ├── block.rs                   # 块设备 trait、错误类型与块大小
│       ├── block/                     # 块设备实现与共享接口检查
│       │   ├── memory.rs              # 借用缓冲区的内存盘
│       │   ├── tests.rs               # 保留原内存盘行为、构造、借用和故障断言
│       │   ├── contract.rs            # 独立镜像模型、共享检查与转发故障注入
│       │   └── contract/              # 共享检查的 host 验证
│       │       └── tests.rs           # 内存盘适配与非合规设备负例
│       ├── mm.rs                      # 物理页分配、回收与错误类型
│       ├── mm/                        # 页分配测试与真实页读写自检
│       │   ├── tests.rs               # 分配、回收、耗尽与非法输入测试
│       │   ├── boot_check.rs          # 启动时整页写读、释放后再分配与测试页清理
│       │   └── boot_check/            # 启动页读写检查的开发机验证
│       │       └── tests.rs           # 页内容、回收与读回错误测试
│       ├── hal.rs                     # 内核使用的控制台、关机、本核中断与自检接口
│       ├── platform/                  # 板级资源、加载布局与启动装配
│       │   ├── mod.rs                 # 统一导出当前板型接口
│       │   └── qemu_riscv64/          # QEMU virt 固定配置
│       │       ├── mod.rs             # 组织 QEMU 板级实现
│       │       ├── linker.ld          # QEMU 内核加载地址和段布局
│       │       ├── memory.rs          # 布局校验、静态标记与启动页分配
│       │       └── memory/            # 板级内存布局测试
│       │           └── tests.rs       # 保留区、地址边界与标记容量测试
│       └── arch/                      # 与 CPU 架构相关的实现
│           ├── mod.rs                 # 选择目标架构并导出接口
│           └── riscv64/               # RISC-V 启动、SBI 与中断开关
│               ├── entry.S            # 设置栈、清理 BSS 并进入 Rust
│               ├── irq.rs             # 保存/关闭/恢复 sstatus.SIE 与编译器屏障
│               ├── irq_check.rs       # QEMU 单层/嵌套中断状态自检
│               └── mod.rs             # 调用 SBI 输出字符和关机
├── specs/                             # 接口行为与验收条件
│   ├── ARCH-001.md                    # CPU 机制与架构 ABI 边界（本核临界区已实现，待审阅）
│   ├── PLAT-001.md                    # 板型选择、启动资源与装配（草案）
│   ├── PLAT-002.md                    # QEMU 板级配置与启动内存迁移
│   ├── DRV-001.md                     # 设备接入、DMA/IRQ 与错误（草案）
│   ├── HAL-001.md                     # 块设备接口与内存盘
│   ├── TEST-002.md                    # 独立镜像模型的共享块接口检查（草案）
│   ├── BOOT-001.md                    # QEMU 启动验收
│   ├── MM-001.md                      # 页分配与回收
│   ├── MM-002.md                      # QEMU 真实内存接入规格
│   ├── MM-003.md                      # QEMU 内核分页与真实访问验收（草案）
│   ├── REGION-001.md                  # 区域赛交付范围
│   ├── TEST-001.md                    # 2025 初赛 11 组测试接入、判分与结果
│   └── TEMPLATE.md                    # 新 SPEC 的简短模板
├── scripts/                           # 构建与验证辅助脚本
│   ├── smoke.py                       # QEMU 启动、真实页读写与中断状态检查
│   ├── test_build.py                  # 构建目标、板型与 host 边界测试
│   ├── test_irq_api.py                # 启动自检 unsafe 边界的调用方编译测试
│   ├── test_smoke.py                  # 启动检查器标记、退出状态与超时测试
│   ├── inventory.py                   # 冻结输入的共用读取、校验与哈希
│   ├── lmbench_trace.py               # 冻结 lmbench 命令映射与未知 trace 名称
│   ├── input_snapshot.py              # 构件哈希去重、路径绑定与分层变化检查
│   ├── sync_test_plan.py              # 从清单生成区域计划与文档表格
│   ├── run_tests.py                   # 串口切组、判分、原始日志与结果
│   ├── fetch_judges.py                # 拉取并核对固定官方 judge 文件
│   ├── reference_protocol.py          # 参考环境准备与逐组执行协议
│   ├── reference_manifest.py          # 构建身份、冻结输入、构件与日志绑定
│   ├── judge_team/                    # 6 组团队判据
│   │   ├── common.py                 # 冻结名称、数值与重复结果规则
│   │   ├── judge_unixbench.py         # Unixbench 完整正数结果行
│   │   ├── judge_iperf.py             # iperf 结束状态
│   │   ├── judge_libcbench.py         # libc-bench 标签与完整 time 行
│   │   ├── judge_lmbench.py           # lmbench 标签、单位与命令上下文
│   │   ├── judge_netperf.py           # netperf 结束状态
│   │   └── judge_cyclictest.py        # cyclictest 结束状态
│   ├── ref/                           # TEST-001 参考 Linux 验证链
│   │   ├── README.md                 # 构建、运行与回放命令
│   │   ├── build_initramfs.py         # 输入校验、确定性构建与 manifest
│   │   └── replay_evidence.py         # 历史归档校验与逐项结果对比
│   └── tests/                         # TEST-001 开发机测试
│       ├── test_input_snapshot.py     # 大构件读取次数、别名与最终内容复核
│       ├── test_inputs.py             # 清单投影、来源与归档边界
│       ├── test_judges.py             # 性能格式与误报负例
│       ├── test_python_sources.py     # 陈旧 pyc、真实执行源码与 judge 子进程
│       ├── test_reference.py          # 确定性构建、来源与并发发布
│       └── test_runner.py             # 切组、捕获与跨阶段状态组合
├── tests/                             # 往届测例清单与来源
│   ├── catalog.json                   # 2024 起步用 basic 清单
│   ├── inventory.json                 # TEST-001 冻结的 427 项清单与官方 judge 哈希
│   ├── reference.lock.json            # 当前/历史参考内核与证据归档的固定输入
│   ├── reference-replay-updates.json  # 与固定历史归档绑定的两处未知名称诊断增补
│   ├── results/                       # TEST-001 参考运行摘要
│   │   ├── raw-evidence-2026-11-17.json # 外部原始串口归档、逐文件哈希与来源
│   │   ├── reference-2026-11-08/       # 原始历史基线
│   │   │   └── summary.md             # 旧镜像差异与参考结果
│   │   ├── reference-2026-11-16/       # 验证链重构后的独立测量
│   │   │   ├── summary.md             # 22 次参考运行、回放、故障与证据
│   │   │   ├── review-fixes.json       # 后续审核修复的源码哈希与回放对照
│   │   │   ├── unresolved-fixes.json   # 模块依赖核实、实际启动与本轮证据
│   │   │   ├── disk-binding.json       # strict 磁盘绑定及输出别名的修复证据
│   │   │   ├── runtime-identity.json   # QEMU 身份、构建快照及回放时间证据
│   │   │   ├── payload-binding.json       # schema 2 内容身份、双构建与回归验证
│   │   │   └── schema2-lua/             # 新协议 manifest 与摘要；原始串口见外部索引
│   │   └── review-2026-11-17/           # 后续审核边界修复与历史诊断差异
│   │       ├── summary.md             # 问题分析、验证结果与限制
│   │       ├── validation.json        # 逐份历史对照、严格回放及源码哈希
│   │       ├── preflight/             # 后续 strict 构件预检、来源字段与有界哈希验证
│   │       │   ├── summary.md         # 问题分析、检查与性能计数的边界
│   │       │   ├── validation.json    # 当前源码哈希、读取次数与结果对照
│   │       │   └── lua/               # 本轮 manifest 与摘要；原始串口见外部索引
│   │       ├── ownership/             # 状态归并、输入来源与不可覆盖发布
│   │       │   ├── summary.md         # 多轮遗漏根因、约束与并发验证
│   │       │   ├── validation.json    # 组合/竞争测试及最终运行证据哈希
│   │       │   └── lua/               # 最终 manifest 与摘要；原始串口见外部索引
│   │       ├── startup/               # Python 缓存隔离与 strict 必填参数
│   │       │   ├── summary.md         # 后续评审发现与补充修复
│   │       │   ├── validation.json    # 缓存投毒/参数组合及实际运行校验值
│   │       │   └── lua/               # 最终 manifest 与摘要；原始串口见外部索引
│   │       └── judges/                # 判据字节快照、trace 与外部证据验证
│   │           ├── summary.md         # 根因、回归及证据归档使用
│   │           ├── validation.json    # 测试与当前执行源码校验值
│   │           ├── round14.json       # 全命令尝试记录与整轮回放来源修复
│   │           └── lua/               # 当前 runner 对既有原始采集的严格回放摘要
│   ├── regional-plan.json             # 2025 区域赛组清单与待锁定输入
│   └── sources.lock.json              # 测例仓库和分支的固定提交
├── docs/                              # 上手、设计与验证指南
│   ├── getting-started.md             # 配环境、运行和首次提交
│   ├── architecture.md                # 内核分层与跨平台扩展图
│   ├── backlog.md                     # 可认领任务与依赖入口
│   ├── collaboration.md               # 认领、测试、审阅与合入流程
│   ├── roadmap.md                     # 周目标与进度取舍
│   ├── verification.md                # 怎样接入测例并记录功能证据
│   ├── benchmarks.md                  # 怎样测性能并生成图表
│   ├── references.md                  # 赛规、测例和技术资料来源
│   ├── ai-usage.md                    # AI 辅助和人工复核记录
│   └── reference/                     # 按需查阅的技术细节
│       ├── delivery-details.md        # 模块接口与兼容风险
│       ├── experiment-protocol.md     # 实验参数与数据记录要求
│       ├── local-irq-critical-section.md # 临界区实现、屏障与限制
│       ├── sv39-paging.md             # 页表接口、启动映射与分页验收
│       ├── test-audit.md              # 历史测评分支与测例审计
│       ├── reference-run.md           # TEST-001 2026-11-08 历史参考测量
│       └── test-inventory.md          # TEST-001 输入冻结、逐组清单与差异
├── .cargo/                            # Cargo 的仓库级配置
│   └── config.toml                    # RISC-V 裸机目标的链接选项
├── .github/                           # GitHub 模板、检查与保护配置
│   ├── ISSUE_TEMPLATE/                # 新建 Issue 的短任务模板
│   │   └── task.md                    # 目标、第一步与验收
│   ├── pull_request_template.md       # 改动、验证与限制
│   ├── main-protection.json           # main 分支保护的可复用配置
│   └── workflows/                     # GitHub Actions 工作流
│       └── ci.yml                     # 内核门禁、TEST-001 契约与历史证据回放
├── .gitignore                         # Git 忽略规则
├── Cargo.toml                         # Rust 工作区与构建配置
├── Cargo.lock                         # 锁定 Cargo 依赖版本
├── Makefile                           # 构建、检查和 QEMU 启动入口
└── rust-toolchain.toml                # 固定 Rust 版本、组件与目标
```

新增、移动、删除路径或改变用途，作者在同一 PR 更新此树，reviewer 对照 Files changed 核对。

固定约定：Rust 内核；FAT32 基础；硬件差异按架构、板级、驱动分层，通过统一接口提供给公共内核；公有 upstream + 成员 fork + PR；SPEC 与失败测试先行，main 可构建。GitHub 不能替代赛方 GitLab 提交；个人资料与凭据不入库。
