# FocOS

FocOS（读作 Focus）是 Rust `no_std` 操作系统。我们要在 VisionFive2 上运行常用 Linux 程序，做好 FAT32 文件访问，再研究文件缓存如何帮助有限内存下的 AI 记忆检索。

## 区域赛要交什么

- **兼容性**：2025 初赛全部测试（原 11 组及比赛冻结版本 LTP），RISC-V 的 musl/glibc 都通过。
- **可用性**：vim 编辑保存，gcc 编译并运行新程序，make 完整与增量构建；在 QEMU RISC-V 和 VisionFive2 分别演示。
- **内核与存储**：真板自有设备驱动、内存与任务、用户态；FAT32 同步后重启读回。Ext4 后端运行历史原版镜像。
- **实验**：真板 FAT32 基线与缓存对照，有原始数据和可重画的图表。

准确范围见 [REGION-001](specs/REGION-001.md)。LoongArch 双平台、当届新增兼容测试、2026 复杂负载和真实磁盘 HNSW 查询留到全国赛。新目标工期风险较高，检查点见 [路线](docs/roadmap.md)。


## 当前到哪里了

main 已有 QEMU 启动、SBI 输出、关机和 CI。2026-11-05 的首次 VisionFive2 启动证据见 [PR #16](https://github.com/MindScrath/Focos/pull/16)，它不代表自有 UART/SD 已完成。

本分支新增：统一块设备接口与内存盘，开发机测试覆盖单块读写、边界和同步错误。运行 `make test` 单独验证；`make check` 同时执行这些测试和原有启动检查。内存盘不提供断电持久化，真实 SD 驱动另行实现。行为约定见 [HAL-001](specs/HAL-001.md)。

共享块接口检查按 [TEST-002](specs/TEST-002.md) 接收独立镜像模型，复用已知数据、边界、写后读和同步判据。当前仅完成 host 接入；有限读回观察不替代真实驱动、DMA 安全或重启持久化验收。

本分支已实现 [MM-001](specs/MM-001.md) 的页分配、回收与错误处理，并按 [MM-002](specs/MM-002.md) 接入 QEMU 真实内存，完成整页写入、读回、释放和再次分配。运行 `make test` 验证开发机行为，`make mm-smoke` 验证 QEMU 内存自检。用户映射与真板内存接入尚未实现，本阶段仍待队友审阅与验收。

本分支新增可嵌套的本核临界区 `with_local_irqs_disabled`：保存并恢复 RISC-V `sstatus.SIE`，闭包只执行一次并透传返回值。单层/两层与初态开/关由 `make irq-smoke` 的 QEMU 状态自检核对；自检入口为仅限启动期的 `unsafe` 接口，`make test` 编译调用方验证此边界。编译器屏障的静态审阅和负例见[参考页](docs/reference/local-irq-critical-section.md)。真实定时中断屏蔽仍待 #2/#5 的时钟与异常入口联调。

本分支按 [MM-003](specs/MM-003.md) 实现 QEMU 内核 Sv39 分页，新增 `make vm-smoke` 验证非恒等地址写读、只读与不可执行保护、撤销、重映射及页表回收。接口与验收约定见[参考页](docs/reference/sv39-paging.md)，运行方法见[上手指南](docs/getting-started.md)。实现待人工审阅；用户态隔离与真板分页另行验收。

测试接入目标已调整为提供官方测试入口，沿用比赛对应版本的脚本与原始输出；旧自研解析和判分路线已停用。范围与停用说明见 [TEST-001](specs/TEST-001.md)，后续工作见 [Issue #8](https://github.com/MindScrath/Focos/issues/8)。官方流程核实和新入口仍待完成，参考 Linux 结果不能计为 FocOS 成绩。

接下来：VisionFive2 启动、内存管理、任务与用户程序、SD 驱动、FAT32。它们目前都还没有验收通过。历史官方测例和性能实验也还没有运行结果。

**历史测试、开发工具和文件系统性能均无通过成绩。** 本页目标不是已有功能。

## 第一天怎么做

1. 按 [上手指南](docs/getting-started.md) 运行 `make check`，看到 `PASS: BOOT-001`。
2. 从 [任务导航](docs/backlog.md) 找到对应 Issue，认领一小步并约队友审阅。
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
│       ├── mm.rs                      # 页分配器与公共分页接口入口
│       ├── mm/                        # 物理页、地址空间与启动验收
│       │   ├── tests.rs               # 分配、回收、耗尽与非法输入测试
│       │   ├── boot_check.rs          # 启动时整页写读、释放后再分配与测试页清理
│       │   ├── boot_check/            # 启动页读写检查的开发机验证
│       │   │   └── tests.rs           # 页内容、回收与读回错误测试
│       │   ├── vm.rs                  # 地址、权限、布局与页资源契约
│       │   ├── vm/                    # 公共页表资源与生命周期
│       │   │   ├── memory.rs          # 复用页分配器的恒等访问适配
│       │   │   ├── space.rs           # 基础映射、窗口、回滚与显式销毁
│       │   │   └── tests.rs           # 对齐缓冲区、故障注入与所有权测试
│       │   └── vm_check.rs            # 实际虚拟访问、保护探针和生命周期验收
│       ├── hal.rs                     # 控制台、中断、分页与同步异常能力入口
│       ├── platform/                  # 板级资源、加载布局与启动装配
│       │   ├── mod.rs                 # 统一导出当前板型接口
│       │   └── qemu_riscv64/          # QEMU virt 固定配置
│       │       ├── mod.rs             # 组织 QEMU 板级实现
│       │       ├── linker.ld          # QEMU 内核加载地址和段布局
│       │       ├── memory.rs          # 布局校验、启动页分配与分页布局
│       │       └── memory/            # 板级内存布局测试
│       │           └── tests.rs       # 保留区、地址边界与标记容量测试
│       └── arch/                      # 与 CPU 架构相关的实现
│           ├── mod.rs                 # 选择目标架构并导出接口
│           ├── vm.rs                  # 页表操作与单次异常探针契约
│           └── riscv64/               # RISC-V 启动、分页、异常与中断开关
│               ├── entry.S            # 设置栈、清理 BSS 并进入 Rust
│               ├── irq.rs             # 保存/关闭/恢复 sstatus.SIE 与编译器屏障
│               ├── irq_check.rs       # QEMU 单层/嵌套中断状态自检
│               ├── paging.rs          # Sv39 编码、遍历与发布顺序
│               ├── paging/            # 实际页表字节的独立验证
│               │   └── tests.rs       # 权限、页号、非法项及 CPU/资源失败注入
│               ├── vm_cpu.rs          # satp 读写、状态校验与本核同步
│               ├── fault.S            # 寄存器保存、异常返回与探针标签
│               ├── fault.rs           # 单次异常匹配器及注入测试
│               ├── fault/             # 真实异常入口装配
│               │   └── native.rs      # stvec、固定记录、探针执行与返回代码
│               └── mod.rs             # 调用 SBI 输出字符和关机
├── specs/                             # 接口行为与验收条件
│   ├── ARCH-001.md                    # CPU 机制与架构 ABI 边界（本核临界区已实现，待审阅）
│   ├── ARCH-002.md                    # QEMU 异常与单次定时中断（草案）
│   ├── PLAT-001.md                    # 板型选择、启动资源与装配（草案）
│   ├── PLAT-002.md                    # QEMU 板级配置与启动内存迁移
│   ├── DRV-001.md                     # 设备接入、DMA/IRQ 与错误（草案）
│   ├── HAL-001.md                     # 块设备接口与内存盘
│   ├── TEST-001.md                    # 官方测试入口目标与旧方案停用说明
│   ├── TEST-002.md                    # 独立镜像模型的共享块接口检查（草案）
│   ├── BOOT-001.md                    # QEMU 启动验收
│   ├── MM-001.md                      # 页分配与回收
│   ├── MM-002.md                      # QEMU 真实内存接入规格
│   ├── MM-003.md                      # QEMU 内核分页与真实访问验收（实现待审阅）
│   ├── MM-004.md                      # QEMU 用户映射、权限隔离与安全拷贝（草案）
│   ├── MM-005.md                      # 用户匿名映射与堆顶管理（草案）
│   ├── REGION-001.md                  # 区域赛交付范围
│   └── TEMPLATE.md                    # 新 SPEC 的简短模板
├── scripts/                           # 构建与验证辅助脚本
│   ├── smoke.py                       # 检查启动、物理页、中断状态与分页专项
│   ├── test_build.py                  # 构建目标拒绝、板型选择与开发机边界测试
│   ├── test_irq_api.py                # 编译调用方，验证启动自检的 unsafe 边界
│   ├── test_vm_api.py                 # 编译调用方，验证分页借用与 unsafe 边界
│   └── test_smoke.py                  # 检查器标记、退出状态与超时测试
├── tests/                             # 往届测例清单与来源
│   ├── catalog.json                   # 2024 起步用 basic 清单
│   ├── regional-plan.json             # 2025 区域赛范围与来源参考（非执行计划）
│   └── sources.lock.json              # 测例仓库和分支的固定提交
├── docs/                              # 上手、设计与验证指南
│   ├── getting-started.md             # 配环境、运行和首次提交
│   ├── architecture.md                # 内核分层与跨平台扩展图
│   ├── backlog.md                     # 模块与 Issue 导航，实时进度在 Issue
│   ├── collaboration.md               # 认领、测试、审阅与合入流程
│   ├── roadmap.md                     # 周目标与进度取舍
│   ├── verification.md                # 怎样接入测例并记录功能证据
│   ├── benchmarks.md                  # 怎样测性能并生成图表
│   ├── references.md                  # 赛规、测例和技术资料来源
│   ├── ai-usage.md                    # AI 辅助和人工复核记录
│   └── reference/                     # 按需查阅的技术细节
│       ├── delivery-details.md        # 模块接口与兼容风险
│       ├── experiment-protocol.md     # 实验参数与数据记录要求
│       ├── trap-timer.md              # 异常入口共存、时钟接口与分阶段验收（草案）
│       ├── local-irq-critical-section.md # 临界区实现、屏障审阅与限制
│       ├── sv39-paging.md             # 页表接口、启动映射与分页验收细节
│       ├── user-mapping.md            # 用户地址空间、权限隔离与安全拷贝细节
│       ├── anon-memory.md             # 匿名映射、堆页回收与验收例子
│       └── test-audit.md              # 历史测评分支与测例审计
├── .cargo/                            # Cargo 的仓库级配置
│   └── config.toml                    # RISC-V 裸机目标的链接选项
├── .github/                           # GitHub 模板、检查与保护配置
│   ├── ISSUE_TEMPLATE/                # 新建 Issue 的短任务模板
│   │   └── task.md                    # 目标、第一步与验收
│   ├── pull_request_template.md       # 改动、验证与限制
│   ├── main-protection.json           # main 分支保护的可复用配置
│   └── workflows/                     # GitHub Actions 工作流
│       └── ci.yml                     # PR 与 main 的构建、启动、内存与中断检查
├── .gitignore                         # Git 忽略规则
├── Cargo.toml                         # Rust 工作区与构建配置
├── Cargo.lock                         # 锁定 Cargo 依赖版本
├── Makefile                           # 构建、检查和 QEMU 启动入口
└── rust-toolchain.toml                # 固定 Rust 版本、组件与目标
```

新增、移动、删除路径或改变用途，作者在同一 PR 更新此树，reviewer 对照 Files changed 核对。

固定约定：Rust 内核；FAT32 基础；硬件差异按架构、板级、驱动分层，通过统一接口提供给公共内核；公有 upstream + 成员 fork + PR；SPEC 与失败测试先行，main 可构建。GitHub 不能替代赛方 GitLab 提交；个人资料与凭据不入库。
