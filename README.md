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

本分支已实现 [MM-001](specs/MM-001.md) 的页分配、回收与错误处理，使用调用者提供的标记管理页地址。运行 `make test` 验证开发机行为；真实内存接入、页表与用户映射尚未实现，本阶段仍待队友审阅与验收。

本分支另增：TEST-001 的输入冻结、427 项子用例清单和参考 Linux 判分基线；FOCOS 的 QEMU/真板结果尚未产生。入口见 [test-inventory](docs/reference/test-inventory.md)、[reference-run](docs/reference/reference-run.md)、[TEST-001](specs/TEST-001.md)。

接下来：VisionFive2 启动、内存管理、任务与用户程序、SD 驱动、FAT32。它们目前都还没有验收通过。历史官方测例和性能实验也还没有运行结果。

**历史测试、开发工具和文件系统性能均无通过成绩。** 本页目标不是已有功能。

## 第一天怎么做

1. 按 [上手指南](docs/getting-started.md) 运行 `make check`，看到 `PASS: BOOT-001`。
2. 到 [任务入口](docs/backlog.md) 认领一小步，约队友审阅。
3. 用 [SPEC 模板](specs/TEMPLATE.md) 约定行为，按 [六步 SOP](docs/collaboration.md) 实现并提交 PR。

| 想了解什么 | 去哪里 |
| --- | --- |
| 分层与平台扩展 | [架构图](docs/architecture.md) |
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
│   └── src/                           # 内核源代码
│       ├── main.rs                    # Rust 入口和 panic 处理
│       ├── lib.rs                     # no_std 通用组件与开发机测试入口
│       ├── block.rs                   # 块设备 trait、错误类型与块大小
│       ├── block/                     # 块设备实现与行为测试
│       │   ├── memory.rs              # 借用缓冲区的内存盘
│       │   └── tests.rs               # 正常、边界与故障注入测试
│       ├── mm.rs                      # 物理页分配、回收与错误类型
│       ├── mm/                        # 页分配行为测试
│       │   └── tests.rs               # 分配、回收、耗尽与非法输入测试
│       ├── hal.rs                     # 供内核调用的早期输出和关机接口
│       └── arch/                      # 与 CPU 架构相关的实现
│           ├── mod.rs                 # 选择目标架构并导出接口
│           └── riscv64/               # RISC-V 启动与 SBI 实现
│               ├── entry.S            # 设置栈、清理 BSS 并进入 Rust
│               ├── linker.ld          # 内核加载地址和段布局
│               └── mod.rs             # 调用 SBI 输出字符和关机
├── specs/                             # 接口行为与验收条件
├── scripts/                           # 构建与验证辅助脚本
│   └── smoke.py                       # 检查 QEMU 启动标记和退出状态
├── tests/                             # 往届测例清单与来源
│   ├── catalog.json                   # 2024 起步用 basic 清单
│   ├── inventory.json                 # TEST-001 冻结的 427 项子用例清单
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
│       ├── reference-run.md           # TEST-001 参考 Linux 判分基线与限制
│       ├── test-audit.md              # 历史测评分支与测例审计
│       └── test-inventory.md          # TEST-001 输入冻结、逐组清单与差异
├── .cargo/                            # Cargo 的仓库级配置
│   └── config.toml                    # RISC-V 裸机目标的链接选项
├── .github/                           # GitHub 模板、检查与保护配置
│   ├── ISSUE_TEMPLATE/                # 新建 Issue 的短任务模板
│   │   └── task.md                    # 目标、第一步与验收
│   ├── pull_request_template.md       # 改动、验证与限制
│   ├── main-protection.json           # main 分支保护的可复用配置
│   └── workflows/                     # GitHub Actions 工作流
│       └── ci.yml                     # PR 与 main 的构建和启动检查
├── .gitignore                         # Git 忽略规则
├── Cargo.toml                         # Rust 工作区与构建配置
├── Cargo.lock                         # 锁定 Cargo 依赖版本
├── Makefile                           # 构建、检查和 QEMU 启动入口
└── rust-toolchain.toml                # 固定 Rust 版本、组件与目标
```

新增、移动、删除路径或改变用途，作者在同一 PR 更新此树，reviewer 对照 Files changed 核对。

固定约定：Rust 内核；FAT32 基础；硬件差异只进 HAL/arch；公有 upstream + 成员 fork + PR；SPEC 与失败测试先行，main 可构建。GitHub 不能替代赛方 GitLab 提交；个人资料与凭据不入库。
