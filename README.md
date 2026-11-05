# FocOS

我们要做一个能在 VisionFive2 上运行、能读写 FAT32 文件的 Rust 操作系统，再用实验说明文件缓存为什么能提速。项目名读作 Focus。

**第一次加入：先按下面步骤跑通，再认领一个小任务。不用先读完所有设计文档。**

## 现在做到哪里了？

已完成：QEMU 中启动一个 RISC-V 裸机内核，打印启动标记并关机；CI 能自动检查这一过程。

2026-11-05 真板进展：已从 VisionFive2 的 U-Boot 加载 FocOS，通过 SBI 打印启动标记；[实测证据](docs/evidence/BOARD-001-20261105.txt)已记录。该改动待人工审阅和队友独立复现。

接下来：自写 UART 收发、定时中断、内存管理、任务与用户程序、SD 驱动、FAT32。它们目前都还没有验收通过。历史官方测例和性能实验也还没有运行结果。

## 第一天：跑起来 → 选任务 → 提第一个 PR

1. 按 [上手指南](docs/getting-started.md) 安装工具、fork 仓库并运行 `make check`。
2. 在 [任务入口](docs/backlog.md) 选一个可以现在开始的任务，在对应 Issue 认领。
3. 找一位队友确认接口和验收方法，再开始实现。提交方法见 [协作流程](docs/collaboration.md)。

跑通后的成功标志：终端出现 `PASS: BOOT-001`，串口日志中有 `FOCOS_BOOT_OK`。

## 仓库里各处放什么？

下面列出纳入版本管理的文件和目录。想看内核代码先找 `kernel/`，想查功能怎样验收先找 `specs/`。

```text
Focos/
├── README.md                          # 项目首页和成员入口
├── AGENTS.md                          # 开发、文档与审阅约定
├── kernel/                            # Rust 裸机内核
│   ├── Cargo.toml                     # 内核 crate 的构建定义
│   ├── build.rs                       # 链接脚本变化时触发重新链接
│   └── src/                           # 内核源代码
│       ├── main.rs                    # Rust 入口和 panic 处理
│       ├── hal.rs                     # 早期输出、关机与板级停机等待接口
│       └── arch/                      # 与 CPU 架构相关的实现
│           ├── mod.rs                 # 选择目标架构并导出接口
│           └── riscv64/               # RISC-V 启动与 SBI 实现
│               ├── entry.S            # 区分 QEMU/bootelf 入口，建栈并清理 BSS
│               ├── linker.ld          # 内核地址及兼容旧 U-Boot 的装载段布局
│               └── mod.rs             # 调用 SBI 输出字符和关机
├── specs/                             # 接口行为与验收条件
│   ├── BOOT-001.md                    # QEMU 启动规格
│   └── BOARD-001.md                   # VisionFive2 bootelf 启动规格与实测范围
├── scripts/                           # 构建与验证辅助脚本
│   ├── smoke.py                       # 检查 QEMU 启动标记和退出状态
│   └── smoke_bootelf.py               # ELF 兼容检查与 QEMU 模拟 bootelf 调用
├── tests/                             # 往届测例清单与来源
│   ├── catalog.json                   # 测例、优先级与验收状态
│   └── sources.lock.json              # 测例仓库和分支的固定提交
├── docs/                              # 上手、设计与验证指南
│   ├── getting-started.md             # 配环境、运行和首次提交
│   ├── visionfive2.md                 # 串口、复制 ELF 和 U-Boot 手动加载
│   ├── architecture.md                # 内核分层与跨平台扩展图
│   ├── backlog.md                     # 可认领任务与依赖入口
│   ├── collaboration.md               # 认领、测试、审阅与合入流程
│   ├── roadmap.md                     # 周目标与进度取舍
│   ├── verification.md                # 怎样接入测例并记录功能证据
│   ├── benchmarks.md                  # 怎样测性能并生成图表
│   ├── references.md                  # 赛规、测例和技术资料来源
│   ├── ai-usage.md                    # AI 辅助和人工复核记录
│   ├── evidence/                      # 可分享的实测证据
│   │   └── BOARD-001-20261105.txt      # 首次真板启动记录（已去除设备标识）
│   └── reference/                     # 按需查阅的技术细节
│       ├── delivery-details.md        # 接口、交付边界和阶段目标
│       ├── experiment-protocol.md     # 实验参数与数据记录要求
│       └── test-audit.md              # 历史测评分支与测例审计
├── .cargo/                            # Cargo 的仓库级配置
│   └── config.toml                    # RISC-V 裸机目标的链接选项
├── .github/                           # GitHub 自动检查与保护配置
│   ├── main-protection.json           # main 分支保护的可复用配置
│   └── workflows/                     # GitHub Actions 工作流
│       └── ci.yml                     # PR 与 main 的构建和启动检查
├── .gitignore                         # Git 忽略规则
├── Cargo.toml                         # Rust 工作区与构建配置
├── Cargo.lock                         # 锁定 Cargo 依赖版本
├── Makefile                           # 构建、检查和 QEMU 启动入口
└── rust-toolchain.toml                # 固定 Rust 版本、组件与目标
```

**随 PR 维护文件树。** 新增、移动或删除纳入版本管理的路径，或改变它的用途时，作者在同一 PR 更新这一节，审阅者对照变更核对路径和说明。

## 做到哪个阶段，再读哪份文档？

| 你现在要做什么 | 阅读入口 |
| --- | --- |
| 配环境、第一次提交 | [上手指南](docs/getting-started.md) |
| 用 VisionFive2 做第一次上板试验 | [上板指南](docs/visionfive2.md) |
| 看一次文件读取经过哪些层、如何扩到双板 | [架构图](docs/architecture.md) |
| 选任务、看依赖 | [任务入口](docs/backlog.md) |
| 写 spec、提交和审阅 PR | [协作流程](docs/collaboration.md) |
| 看本周目标和取舍 | [推进路线](docs/roadmap.md) |
| 接入往届官方测例 | [功能验证](docs/verification.md) |
| 做性能实验、准备图表 | [性能实验](docs/benchmarks.md) |
| 查赛规、外部来源 | [来源说明](docs/references.md) |

## 几条固定约定

- 内核用 Rust `no_std`；首块真板是 VisionFive2 / JH7110（RV64GC），全国赛再扩展龙芯 2K1000LA 星云板（LoongArch64）。
- 基础文件系统是 FAT32。CPU/板卡差异放在 HAL 和 arch，文件系统与 VFS 不写架构条件编译。
- [MindScrath/Focos](https://github.com/MindScrath/Focos) 是公有上游。成员从个人 fork 提 PR，main 必须可构建，合入需要 CI 和队友批准。
- 报名资料、邮箱、令牌不放进代码库或日志。`devdoc/` 只用于本地资料，已被 Git 忽略。

GitHub 开发仓库不能替代赛方要求的 GitLab 提交。第三方代码引入前要确认许可证并记录来源；AI 辅助记录见 [ai-usage.md](docs/ai-usage.md)。
