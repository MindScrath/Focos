# ARCH-002 参考：接口、入口迁移与分阶段验收

本页补充 [ARCH-002](../../specs/ARCH-002.md)，规格已由 [PR #61](https://github.com/MindScrath/Focos/pull/61) 合入；本分支已接入设备树频率、单次时钟、共用异常入口和真实屏蔽专项，仍待两名成员人工审阅。

术语：SIE 指 `sstatus.SIE` 全局中断允许位；`sie` 是各中断源的允许寄存器，`sie=0` 表示全部中断源关闭。STIE 指 `sie.STIE` 定时源允许位；pending（已挂起、等待处理）在本项中指 `sip.STIP` 硬件状态位。

## 分三阶段交付

1. **约定**：审阅本 SPEC，确定唯一异常入口、时间单位、所有权、失败状态和真实 pending 判据。此阶段只改文档。
2. **异常与单次时钟**：交付板级最小设备树频率读取器及其边界负例；先写分发/换算/错误与检查器失败测试，再扩展现有入口、接 SBI TIME 和显式切换中断的 HAL 接口，完成 `timer-smoke`、真实上下文保存检查及分页回归。
3. **临界区联调**：先让真实 pending 验收在旧实现上失败，再完成 `irq-timer-smoke` 的初态开/关与嵌套场景；稳定后将两项专项纳入 CI。提交完整证据供人工验收 #35。

第一阶段规格已独立合入；按负责人要求，第二、三阶段合为一个实现 PR 审阅，保留各阶段的独立验收。上下文切换另提 SPEC，不在本任务内顺带实现。

## 最小接口

类型与入口通过 HAL 导出，不向公共模块暴露 CSR 编号、SBI 编号或板名。

```rust
pub struct MonotonicNs(pub u64);
pub struct TimerSnapshot {
    pub completed: u64,
    pub pending: bool,
}
pub enum TimerError {
    InvalidState, Unsupported, Overflow,
    DeadlineElapsed, Busy, Firmware(isize),
}
// NativeTimer 字段私有，不实现 Clone/Copy/Send/Sync；唯一句柄持有至关机。
pub unsafe fn init_boot_timer(frequency: core::num::NonZeroU32)
    -> Result<NativeTimer, TimerError>;
impl NativeTimer {
    pub fn now(&self) -> Result<MonotonicNs, TimerError>;
    pub fn snapshot(&self) -> TimerSnapshot;
    pub unsafe fn arm_at(&mut self, deadline: MonotonicNs) -> Result<(), TimerError>;
    pub unsafe fn cancel(&mut self) -> Result<(), TimerError>;
    pub unsafe fn set_boot_irqs_enabled(&self, enabled: bool) -> Result<(), TimerError>;
}
```

- 初始化要求唯一启动 hart、有效内核栈和基础映射、SIE=0、`sie=0`、现有异常入口已安装且读回一致；句柄不拥有或替换分页探针。重复初始化返回 `InvalidState`，不覆盖已有服务。初始化失败保持中断关闭，不交付句柄。
- `arm_at` 和 `cancel` 要求 SIE=0、没有并发调用且仍使用本入口。预约成功只开放 STIE，不修改全局 SIE；调用者按原临界区状态恢复 SIE。`snapshot` 以短本核临界区一致读取完成计数及硬件 pending 状态，不能仅用软件“已到时间”替代 pending。
- `now` 读取启动后的硬件时间换算值，允许连续读数相等。频率由 QEMU 板级层核验设备树 `timebase-frequency` 后交付；属性缺失、格式错误或为零由板级读取器返回具体的 `TimebaseError`，见下节。不把 CPU 主频当时间基频，也不复用未经核对的真板数值。当前 PLAT-002 只检查设备树起址、不解引用；最小读取器及范围/长度检查由本项实现，契约见下节。启动时在分页前取得频率值，分页后传给 `init_boot_timer`，不保留设备树借用。
- `set_boot_irqs_enabled` 是新增的启动期 HAL 能力，通过 `NativeTimer` 方法导出，不放宽现有私有 `set_local_irqs_enabled` 的契约。调用方须独占启动 hart、持有有效内核栈和基础映射、不在临界区闭包或异常处理器内、不持有会与处理器冲突的引用。开启还要求调用时 SIE=0；开启前检查服务有效、入口读回一致，且 `sie` 只可能包含 STIE；若 STIE 已开，必须有本服务的有效预约，允许 STIP 已挂起。检查失败保持 SIE=0 并返回 `InvalidState`；成功只设置 SIE，事件可在返回前处理。关闭操作先清 SIE，并保留 STIE/预约供后续观察或撤销；失败服务仍允许关闭。两方向均须有编译器访存约束。普通调用须编译失败，显式 `unsafe` 调用方须编译通过；host 覆盖非法服务/入口/中断源，QEMU 覆盖 pending 时开启。
- 纳秒与 tick 换算使用足够宽的中间值及受检运算。预约对应 tick 必须严格晚于当前 tick，且不能等于保留作撤销的 `u64::MAX`；已过期报 `DeadlineElapsed`，不可表示报 `Overflow`。参数检查在改变预约前完成；已有预约报 `Busy`，保留原预约。
- 原始计数与纳秒结果不允许回绕；检测倒退报 `InvalidState`。本阶段不支持硬件计数回绕后的继续运行。三次定时验收是三次显式预约，不声称已有周期时钟或调度器。
- 固件调用前探测 SBI TIME 支持；不支持报 `Unsupported`，不静默退回 legacy。固件错误保留错误码，关闭 STIE、将服务置为不可再预约的失败状态并交启动调用方失败关机；不承诺恢复旧硬件期限。`cancel` 成功关闭 STIE、把固件期限设置为最大值，清除软件预约；无预约时仍可调用且不增加完成计数。
- 定时入口只接受已预约事件，保持 SIE=0，先关闭 STIE 并调用固件撤销；撤销成功后以原子状态发布完成、清除预约。固件失败或计数溢出直接失败关机。处理中不输出普通成功日志，返回后由启动自检打印观察结果。计数存储独立于句柄，处理器不访问被 `&mut self` 借用的对象。

## 本项新增的板级最小设备树读取器

本项遵循 [PLAT-001](../../specs/PLAT-001.md) 的资源校验方向，但不依赖尚未实现的通用启动描述校验器。新增能力限于固定 QEMU 配置下读取 `/cpus/timebase-frequency`，不改变 MM-002 的内存分配或保留区规则，也不交付通用设备枚举。

板级入口为 `unsafe fn boot_timebase_frequency(dtb_start: usize) -> Result<NonZeroU32, TimebaseError>`，通过 `platform` 导出；纯解析入口为 `fn parse_timebase_frequency(bytes: &[u8]) -> Result<NonZeroU32, TimebaseError>`。`TimebaseError` 区分 `InvalidRange`、`MalformedDtb`、`UnsupportedVersion`、`MissingFrequency` 和 `InvalidFrequency`。启动调用方保留具体错误并失败关机，不以默认频率替代。缺失目标属性返回 `MissingFrequency`；设备树结构或编码错误返回 `MalformedDtb`；目标属性宽度不是 4 字节或值为零返回 `InvalidFrequency`。`init_boot_timer` 只接收已校验的 `NonZeroU32`，因此 `TimerError` 不设置 `InvalidFrequency` 变体，板级错误不映射为 HAL 错误。

- 裸机入口仅在唯一启动 hart、SIE=0、分页关闭时调用；地址必须来自当前 QEMU 固件，固件已交付可读且不再修改的设备树内存。实现先复用 MM-002 的起址/对齐及内核边界检查，再以受检加法确认完整头部位于 `[dtb_start, 0x88000000)` 后才读取头部。读取总长度后，再核对完整数据仍在该保留区内，才形成有界字节视图。地址数值合法不能替代固件已初始化这段内存的调用前提：完整头部及总长通过范围检查后声明的全部字节（含填充）须属于同一连续、已初始化的固件存储，调用期间无写者。不可信的有界字节缓冲区使用安全解析入口，不能仅凭起址有效调用裸机入口。
- 解析器检查魔数、支持的版本、总长度、结构块/字符串块的偏移与长度、对齐和不重叠；每次推进或取值前检查范围及算术溢出。节点名和属性名须在对应块内终止，节点嵌套及结束标记必须完整；读到频率后仍核对剩余结构，拒绝重复目标节点/属性。只接受目标路径下长度为 4 字节的大端非零频率值。
- host 用有界字节数组覆盖正常读取、截断头部/属性、错误魔数、不支持版本、总长度越界、块范围溢出/重叠、名称未终止、节点未闭合、缺失/重复属性、错误宽度及零频率；地址范围计算另测 RAM 末尾与加法溢出。任何错误不访问范围外字节、不启用中断、不修改分配器。
- 在启用 MM-003 分页之前读取并仅保留频率值。当前基础页表不覆盖设备树保留区，不能在分页后直接沿用物理指针。QEMU 验收记录实际频率，host 替身通过不能代替该启动路径验收。

## 如何保留现有异常入口

当前 `fault/native.rs` 的 `installed()` 同时核对安装状态和 `stvec`；`vm_cpu.rs` 依赖它才允许启用分页。本实现扩展同一汇编入口和 Rust 分发，保留该检查，不再次写入 `stvec`。保留 `install_fault_probe` 的单次安装、unsafe 条件和 `FaultProbe` 三项匹配语义。

异步中断可能发生在任意指令处，不能只验证 Rust 调用约定中的被调用者保存寄存器。汇编专项在定时事件前填入整数寄存器与可用浮点状态，事件后逐项核对，覆盖浮点关闭与启用两条路径；向量扩展不在当前目标支持范围。入口先保存状态再调用 Rust，返回依靠保存的 SPIE/SPP 等状态执行 `sret`，不能固定开中断或随意增加 `sepc`。同步异常仍只允许显式探针标签恢复。

处理器内的异常不应递归进入普通诊断路径。实现须有重入检测和不依赖格式化分配的最小失败出口；同时审阅有效栈及映射前提，不能宣称能从损坏的栈恢复。外部中断与软件中断保持禁用，意外收到时失败，不当成 timer 消费。

## 真实 pending 与证据

临界区场景在外层 SIE 已关闭后才预约短期限，避免预约与进入临界区之间提前处理。内层轮询实际 pending 位；确认 pending 后分别在内层、退出内层后和退出外层前记录计数。初态关场景还须在外层退出后再次核对 pending 与计数，再通过 `set_boot_irqs_enabled(true)` 显式开启 SIE。每次只允许计数从 N 到 N+1，并在有界观察期确认没有重复事件；最后通过 `set_boot_irqs_enabled(false)` 关闭 SIE 并撤销时钟。

自检不得在临界区闭包中自行改 SIE；打开初态与最终显式开启均通过新接口在闭包外进行。每个等待同时设置有限轮询上限与硬件时间截止值，时间停滞也必须失败。QEMU 外部超时覆盖内核无法返回的情况；日志中的成功标记不能抵消退出失败或超时。

实现 PR 记录 base/head、脏工作区状态、ELF SHA-256、Rust/QEMU/OpenSBI 版本、完整命令、实际时间基频、三次预约/完成计数、pending 与 SIE 观察、退出状态和原始串口日志。host 注入测试不算真实硬件事件；QEMU SBI 时钟不算 #2 的真板自有时钟驱动验收。

硬件 pending 的可观察性须在实际 QEMU/OpenSBI 组合上验证：SIE=0 时仍需固件转交 STIP。等不到该位必须超时失败，不能改用期限已过来替代。

## 本次实现与复现

本次将设备树读取、单次时钟与临界区联调合成一个实现。`platform::boot_timebase_frequency` 在分页前只交付 `NonZeroU32`；基础页表和分页探针完成后，`init_boot_timer` 探测 SBI TIME 并清理定时源。CPU 编码、状态机和固件调用留在 `arch/riscv64`，公共 `time` 模块只定义时间、错误与轮询预算。

`TimerCore` 记录未初始化、就绪、已预约、失败四种状态。原生后端在 SIE=0 时复制固定记录到局部变量，操作结束后写回；打开 SIE 前局部借用已结束。处理器只接触固定记录，不访问被调用方借用的 `NativeTimer`；句柄不可复制、跨核传递或共享。显式中断开关的汇编不使用 `nomem`，保留两侧编译器访存约束，原临界区的编译器屏障也未移除。

共用入口保存全部整数状态及 FS 启用时的浮点状态，再进入 Rust。保存整数寄存器后设置重入守卫；二次异常直接走汇编固定字符串与 SBI 失败关机，不再进入 Rust 格式化。该出口依赖有效内核栈及固件 SBI，不承诺恢复栈损坏或固件自身不可用的情况。定时事件返回原 `sepc`；同步页异常仍只按一次性三项匹配恢复。

`unsafe fn check_boot_timer(&mut NativeTimer) -> Result<(), TimerCheckError>` 仅供启动期独占自检，入口要求 SIE=0、无活动预约，不能放在临界区或处理器内。先完成三次独立预约，再验证 FS 关闭/开启两种寄存器恢复，最后验证初态开/关的嵌套临界区。每个等待同时有 100 ms 硬件时间预算与一百万次轮询上限；计数静默观察期为 1 ms。三处短期限预约都用最新时间加 2 ms，遇到 `DeadlineElapsed` 才重新取时预约，最多尝试三次。其他错误立即返回，尝试耗尽也由启动调用方失败关机。

寄存器专项在 STIP 已挂起后填入整数/浮点哨兵值，在固定 256 条 NOP 的汇编窗口内开启 SIE，随后关闭。专用标志使异常入口在返回前主动破坏待恢复的寄存器；因此删掉恢复指令不能靠“处理器没碰这个寄存器”蒙混通过。核对全部整数寄存器、32 个浮点寄存器、`fcsr` 及返回状态；按 `sret` 语义排除 SPP/SPIE 的固定变化。实际被打断 PC 必须位于 ELF 的窗口符号范围内。当前链接使用 `--no-relax`，无 TLS，测试中的 gp/tp 哨兵不构成通用用户态 ABI 支持。

两组临界区日志逐点记录真实 STIP、SIE 和处理次数。最终再次从已撤销窗口触发读取页异常，核对同步探针在定时事件后仍有效。原分页专项的三条异常记录仍核对，新增第四条为该回归。
读取器接受与格式版本 17 兼容的 DTB；校验头部、保留表终止、结构/字符串块范围及顺序、名称终止、嵌套与完整结束，重复目标节点/属性报错。对齐填充只校验可访问范围，不要求内容为零；实际 QEMU 固件可能保留非零填充。它只提取时间基频，不据保留表重新划分内存，也不提供通用设备树枚举。

裸机读取与 host 测试共用先验证范围、再读取的顺序；host 通过读回调核对非法头部地址不读取，非法整树长度只读头部。内部截断测试会重建合法的外层长度，确保到达结构和字符串块内部校验。分别移除两次范围检查、忽略字符串块长度、找到频率就提前返回的四个变异均须被运行中的测试拒绝。

运行 `make check`、`make timebase-smoke`、`make timer-smoke`、`make irq-timer-smoke`、`make mm-smoke`、`make irq-smoke` 和 `make vm-smoke`。基频专项对照 OpenSBI 与内核读数；时钟专项核对期限、实际时间、上下文计数/PC；屏蔽专项核对每个观察点，均检查标记顺序、退出结果及超时。日志在 `target/evidence/`，带源码、ELF 哈希、QEMU/OpenSBI 信息与命令。CI 已配置运行这些专项，远端结果以 PR 为准。

`make trap-negative-smoke` 顺序构建 `trap-unknown-test` 和 `trap-reentry-test` 两个专用失败镜像。前者在普通代码中执行未登记的断点，后者在 Rust 异常处理器内再次触发断点；要求进入对应失败出口且没有启动成功标记，超时或信号退出不能算预期失败。固件请求关机可使 QEMU 返回 0 或 1，负例检查同时核对诊断，不能单看进程退出码。脚本最后恢复无 feature 的普通构建；这些 feature 不用于正常运行。

本次本地结果只覆盖 QEMU 单核 S 模式；没有用户态、调度、周期重装、外部 IRQ 或真板时钟。#35 的验收仍需人工审阅本次证据，#5/#2 的其余交付保持开放。

## 固定版本依据

- [RISC-V 特权规范 20240411：Supervisor](https://docs.riscv.org/reference/isa/v20240411/priv/supervisor.html)：入口、异常原因、SIE/SPIE、STIE/STIP 与异常返回。
- [SBI v2.0 TIME](https://github.com/riscv-non-isa/riscv-sbi-doc/blob/v2.0/src/ext-time.adoc)：绝对 tick 期限、清除 pending，以及最大期限/屏蔽 STIE 的撤销方式。纳秒为本项目 HAL 的单位，arch 负责转换。
- [MM-003 异常契约](sv39-paging.md#同步异常与探针)、[本核临界区](local-irq-critical-section.md)：沿用现有安全条件与编译器屏障约定。
- [Devicetree Specification v0.4：DTB 格式](https://github.com/devicetree-org/devicetree-specification/blob/v0.4/source/chapter5-flattened-format.rst)：最小读取器的头部、块边界、版本与结构编码依据。
