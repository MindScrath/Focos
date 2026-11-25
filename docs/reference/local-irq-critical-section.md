# 本核临界区：实现与编译器屏障审阅

关联：[Issue #35](https://github.com/MindScrath/Focos/issues/35)、[ARCH-001](../../specs/ARCH-001.md)。范围：QEMU RISC-V 单 hart；不含跨核互斥和真实定时中断验收。

## 接口与状态

```rust
pub fn with_local_irqs_disabled<R>(f: impl FnOnce() -> R) -> R
```

[实现](../../kernel/src/arch/riscv64/irq.rs)只用 `sstatus.SIE` 表示本核可屏蔽中断状态。进入时用 `csrrc` 原子读取并清零 SIE，保存旧 `sstatus`；闭包正常返回后按保存值用 `csrs` 或 `csrc` 只恢复 SIE 位，不覆盖其他状态位。闭包执行一次，返回值原样返回。

| 进入时 SIE | 单层闭包内 | 退出单层 | 两层闭包内 | 退出内层 | 退出外层 |
| --- | --- | --- | --- | --- | --- |
| 1 | 0 | 1 | 0 | 0 | 1 |
| 0 | 0 | 0 | 0 | 0 | 0 |

嵌套不需要计数器：内层进入时保存到的 SIE 已经是 0，退出内层只恢复为 0；最外层恢复进入前的值。若闭包自行开启中断，恢复分支会按保存值重新设置 SIE，但闭包仍违反调用前提。

## 编译器屏障

关中断指令本身不访问内存，因此 `asm!` 使用 `nomem` 是准确的。但这也意味着编译器可以把闭包内外的普通访存移过关中断边界。实现必须在两处显式约束：

1. `csrrc` 之后、调用闭包之前：阻止闭包内访存上移到关中断之前。
2. 闭包返回之后、恢复 SIE 之前：阻止闭包内访存下移到重新开中断之后。

代码使用两次 `compiler_fence(Ordering::SeqCst)`。`compiler_fence` 只约束编译器，不产生硬件 `fence` 指令，也不代替设备或跨核内存屏障；这是本接口需要的语义。

## 被拒绝的负例

下面写法只有 `nomem` 汇编，没有其他编译器屏障，**审阅不通过**：

```rust
// 负例：不得合入
unsafe { asm!("csrci sstatus, 2", options(nomem, nostack)); }
protected_memory_access();
unsafe { asm!("csrsi sstatus, 2", options(nomem, nostack)); }
```

`nomem` 明确告诉编译器汇编不读写内存，编译器可以把 `protected_memory_access()` 提升到关中断之前，或下沉到重新开中断之后。寄存器值或定时器计数测试通过，不能证明受保护访存仍在临界区内；仅有 `nomem` 的负例必须判失败。去掉 `nomem` 会形成隐式内存 clobber，但本实现选择显式屏障，便于把约束写在边界处并审阅。

## QEMU 状态自检

[启动自检](../../kernel/src/arch/riscv64/irq_check.rs)依次验证初态开/关与单层/两层闭包，核对每次读到的 SIE、闭包执行次数和返回值。启动流程经 HAL 入口调用；`make irq-smoke` 在 QEMU 中运行，要求 `FOCOS_IRQ_OK` 唯一出现且在 `FOCOS_BOOT_OK` 之前；日志写入 `target/evidence/irq-qemu-rv.log`。

自检入口为 `pub unsafe fn check_local_irq_state() -> Result<(), IrqCheckError>`，其单层/嵌套辅助函数也要求相同的启动期前提。调用方须保证启动流程独占当前 hart、整个调用期间 `sie` 始终为 0。`kernel_main` 的唯一调用点依据 `entry.S` 清零 `sie`、后续尚未启用中断源满足前提；仅有 SIE 为 0 不足以安全调用。成功和错误返回均保持 SIE 为 0；若入口 SIE 已开，先关闭再返回 `InitialStateEnabled`。

[调用方编译测试](../../scripts/test_irq_api.py)随 `make test` 执行：从本次 Cargo 构建产物取真实 RISC-V 库，分别通过 HAL 和架构层编译调用方。普通调用必须报 E0133，显式 `unsafe` 调用与安全的普通临界区调用必须可编译；不运行裸机代码。修复前两个导出的普通调用都编译成功，新增回归测试失败。

实现前先加入自检，`make build` 因缺少 `irq` 模块失败；实现后通过。另做过两次临时变异核对测试有效性：

- 去掉关中断（只读取 SIE 作为保存值，不执行 `csrrc`）：`single-on` 在闭包内的状态检查报告 `expected: false, actual: true` 并 panic。
- 恢复时总是开中断：`single-off` 场景报告 `expected: false, actual: true` 并 panic。

两次变异均已还原。QEMU 的 panic 路径以 `FOCOS_PANIC` 被检查器判失败，不依赖 QEMU 退出码。

## 调用前提与未覆盖项

- 闭包不得阻塞、让出 CPU、迁移到其他核或自行修改中断状态。
- 只屏蔽当前 hart 的可屏蔽中断，不提供跨核互斥。
- 入口汇编已清 `sie`；自检的 `unsafe` 调用方须保证整个调用期间 `sie` 为 0，返回前保持 SIE 关闭。
- 内核 panic 的终止路径不承诺恢复后继续运行。
- 真实定时中断验收仍须与 #2/#5 的 QEMU 时钟/异常入口联调；本次未实现，Issue #35 保持开放。
- QEMU 结果不代表真板。

依据：RISC-V Privileged Specification 的 `sstatus.SIE` 与 `csrrc`/`csrs`/`csrc` 语义；Rust `core::arch::asm!` 的 `nomem`/`nostack` 选项说明；Rust `core::sync::atomic::compiler_fence` 文档。涉及 HAL 与内存安全，需另外两名成员审阅。

## 真实定时中断联调

本分支按 [ARCH-002](../../specs/ARCH-002.md) 接入 `make irq-timer-smoke`：在嵌套临界区内观察真实 `sip.STIP`，分别验证初态开/关时的处理计数和恢复时机。原 `irq-smoke` 仍只证明状态与嵌套恢复；编译器屏障的静态约束继续保留。实现和 QEMU 结果待两名成员审阅，不能据此推断 SMP 或真板能力。
