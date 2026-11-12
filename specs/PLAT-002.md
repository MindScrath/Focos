# PLAT-002：迁移 QEMU 板级配置与启动内存接入

关联：[Issue #33](https://github.com/MindScrath/Focos/issues/33) · 父任务：[#4](https://github.com/MindScrath/Focos/issues/4) · 状态：草案

## 要做到什么

启动流程从统一板级入口取得页分配器，QEMU 固定布局不再由 CPU 架构模块提供。保持 MM-002 的配置、分配范围与错误行为；本次不增加设备树解析、多区间、真板接入、页表或通用启动资源框架。

本次只迁移已有固定布局与单区间接入。PLAT-001 的通用 RAM/保留区校验、重叠保留区取并集及多可分配区处理另拆任务，不作为 #33 的验收条件。

例：`kernel_end = 0x8021f123`、`dtb_start = 0x87e00000`，迁移前后均得到 `[0x80220000, 0x87e00000)`。

## 行为约定

下表路径相对仓库根目录，目标路径随实现创建。

| 现有内容 | 迁移后的归属 |
| --- | --- |
| `kernel/src/arch/riscv64/memory.rs` 及 `memory/tests.rs` | 移至 `kernel/src/platform/qemu_riscv64/memory.rs` 及其 `memory/tests.rs`；固定 RAM/加载地址、布局校验、静态标记和启动分配入口属于此板型 |
| `kernel/src/arch/riscv64/linker.ld` | 移至 `kernel/src/platform/qemu_riscv64/linker.ld`，作为该板型的加载布局；加载地址和全部段、符号语义保持不变 |
| `.cargo/config.toml`、`kernel/build.rs` | 更新链接脚本路径及依赖追踪；Rust 目标和 `--no-relax` 保持不变 |
| `kernel/src/arch/riscv64/entry.S`、架构模块 | 保留 CPU 入口、栈/BSS 初始化、关闭中断与分页、启动参数传递及 SBI 调用；移除板级内存模块的声明和导出 |
| 新增 `kernel/src/platform/mod.rs`、`kernel/src/platform/qemu_riscv64/mod.rs` | 前者选择并统一导出板级接口，后者组织 QEMU 实现；`MemoryInitError` 随内存实现迁移，由统一入口重新导出 |
| `kernel/src/main.rs`、`kernel/src/lib.rs` | 导出 `platform`，启动流程经其统一入口初始化内存；`mm` 分配器与页读写检查保持通用 |

- **入口与选择**：`platform` 统一导出 `unsafe fn boot_page_allocator(dtb_start: usize) -> Result<(Range<usize>, PageAllocator<'static>), MemoryInitError>`，签名沿用现有入口。`main.rs` 调用 `focos_kernel::platform::boot_page_allocator(dtb_start)`，不引用具体板型。本阶段唯一默认板型为 `qemu_riscv64`，目标为 `riscv64gc-unknown-none-elf`；构建输出标明选择，其他裸机目标明确失败。Rust 目标不用于自动识别硬件，VisionFive2 尚未接入此内存启动路径，后续另加明确的板型选择。开发机测试只编译布局计算；启动入口与静态标记仅在受支持的 RISC-V 内核构建中启用，不在开发机执行。
- **范围与错误**：沿用 MM-002 的 RAM `[0x80000000, 0x88000000)`、加载地址 `0x80200000` 和 4 KiB 页。先校验布局，再检查标记容量；布局无效仍返回 `InvalidLayout`，分配器错误仍包装为 `Allocator(PageAllocError)`。失败不修改标记或待分配页，不缩小范围；保留现有边界测试及错误优先级断言。
- **目标检查**：`kernel/build.rs` 对裸机目标检查完整目标名，仅接受 `riscv64gc-unknown-none-elf`，不支持的裸机目标明确报错；开发机库测试不受该限制。构建输出标明 `FocOS platform: qemu_riscv64`，不将开发机测试标成 QEMU 运行。
- **所有权**：仍只由启动核在关闭中断、分页时调用一次。板级静态标记计入 `__kernel_end`，返回的分配器独占借用标记并由启动流程持有；不建立第二套分配器。固件、内核、栈、标记和设备树保留规则及真实页访问的安全前提沿用 MM-002。
- **安全文档**：入口的 `# Safety` 写明启动核、关闭中断与分页、仅初始化一次、无同区间分配器及静态标记独占借用。`dtb_start` 由架构入口原样传递固件参数，本次仅作边界数值使用，不解引用；边界和对齐由函数检查，非法数值仍返回布局错误，不能改成仅由调用者保证。
- **链接符号**：`_start`、`boot_stack_bottom`、`boot_stack_top` 仍由 `entry.S` 定义；链接脚本保留 `ENTRY(_start)` 和入口段，提供 `__bss_start`、`__bss_end` 供架构入口清零。启动栈保持 64 KiB、16 字节对齐并纳入 BSS；`__kernel_end` 覆盖全部内核装载段、BSS、栈和静态标记。架构入口依赖符号契约，不依赖脚本路径或加载地址；段对齐与符号语义保持不变。板级脚本可保留 `OUTPUT_ARCH(riscv)`，本次不抽象通用链接脚本框架。
- **构建与运行**：链接布局中的加载地址须与板级校验一致；新脚本变化必须触发重新链接。保持 `virt`、128 MiB、单核、默认固件及现有检查命令，版本只记录到日志。按 PLAT-001 在 `docs/getting-started.md` 补齐平台运行配置：目标/工具链、固件、ELF 产物、控制台及 `make build`、`make check`、`make mm-smoke`；引用 `scripts/smoke.py` 的完整命令（含 `-nographic`、`-no-reboot`、`-kernel`），设备/文件系统/用户程序未接入项明确标注。
- **配套文档**：实现迁移时同步 MM-002 的布局测试链接，并将其“平台常量及启动参数处理放在 HAL/arch”明确为：arch 传递 CPU 启动参数，platform 管理 QEMU RAM、加载地址及保留区，mm 负责通用页分配。规格阶段保留对现有实现的描述，不提前改成已迁移。

## 怎样验收

以下是实现 PR 的验收要求，本规格不表示迁移或新增检查已完成。

| 场景 | 操作或测试入口 | 应看到什么 |
| --- | --- | --- |
| 分层与选择 | 审阅迁移路径、依赖和构建输出；构建支持目标，另用已安装但不受支持的裸机目标验证拒绝路径 | 固定布局归板级；arch 不依赖 platform，公共 mm 不识别板名；支持目标输出板型，其他裸机目标由构建脚本拒绝，不能把缺工具链当通过；开发机库测试可运行 |
| 正常与失败 | 开发机 `make test`，保留原布局、分配器与检查器用例和断言 | 上述例子、对齐边界、单页、耗尽、非法布局和标记不足行为不变；布局错误优先且失败不改标记 |
| 链接一致性与依赖 | 在隔离副本中 `make build`，用支持 RISC-V ELF 的 `nm -n` 记录 `__kernel_end`；将 `__kernel_end = .;` 改为 `__kernel_end = . + 4096;` 后再次构建并检查符号，恢复后重建；用 `readelf -lW` 核对加载段 | 不清理构建缓存也会重新链接；符号恰增加 `0x1000`，恢复后回到原值。最低 LOAD 段加载地址保持 `0x80200000`，与板级常量一致；栈和标记仍在内核保留区 |
| 段与保留区 | 用 `readelf -lW` 检查 PT_LOAD 的物理地址与内存大小，用 `nm -n` 核对栈及静态标记符号（必要时使用 Rust 符号解码） | 全部装载段的内存范围含 BSS 均被内核保留区覆盖；最低物理加载地址为 `0x80200000`，启动栈大小/对齐和静态标记位置符合约定 |
| 实际启动回归 | 迁移前后在同一工具链、QEMU/固件及机器配置下分别运行 `make check`、`make mm-smoke` | 两次均通过，实际分配范围一致；完成页写读和回收后依次输出唯一 MM/BOOT 成功标记，退出状态及超时判据不变 |
| 配置复现 | 同步 README 文件树、MM-002 路径与职责说明、上手指南；另一名成员按命令运行 | 文档指向实际路径，队友能复现；PR 附前后源码 SHA、版本、完整命令、分配范围和串口日志 |

先保存迁移前基线，再为新增的板级入口/构建约定补能暴露缺失行为的检查，随后迁移并回归。链接脚本的临时修改只在隔离副本中验证，不提交到正式分支。不得以删除断言或放宽判据通过验收；QEMU 结果不作为真板证据。

## 依据与依赖

依赖 [PLAT-001](PLAT-001.md)、[MM-002](MM-002.md)、[MM-001](MM-001.md) 与 [ARCH-001](ARCH-001.md)，职责遵循[架构文档](../docs/architecture.md)。本次仅落实固定 QEMU 路径的迁移，PLAT-001 的通用资源校验与其他平台另行实现。PLAT-001 和架构导航的顺序调整是本次提议，随本规格由内存 owner 与板级接口使用者共同审阅；涉及 HAL 与内存安全，需另外两名成员审阅后实施。
