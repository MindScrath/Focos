# MM-003 参考：页表、启动映射与验收

本页定义 [MM-003](../../specs/MM-003.md) 的页表接口、启动映射和验收条件，实现与验证结果见 [PR #43](https://github.com/MindScrath/Focos/pull/43)，待人工审阅。

## 接口与资源

公共内存模块管理页表页和映射生命周期，arch 借用页面完成页表操作及 CPU 控制，platform 只交付布局。接口经统一入口使用，不把 RISC-V 页表项或板名暴露给公共调用者。遵循[资源交接表](delivery-details.md#平台接口怎样交接)，不再建立第二个物理页分配器。

以下为最小调用契约。公共类型从内存入口导出，架构能力从统一 HAL 入口导出；具体目录和内部存储随实现落地。开发机替身与真实实现使用相同签名。接口及失败路径经两名成员复审后，才进入实现。

| 类型 | 含义 |
| --- | --- |
| `PhysAddr`、`VirtAddr` | 分别包装 `usize`，可复制但不能隐式互换；均提供 `const fn from_raw(value: usize) -> Self` 和 `const fn as_usize(self) -> usize`，构造不校验、不截断数值 |
| `MapPerm { read, write, execute, user }` | 四个布尔权限；本阶段仅接受内核 R、RW、RX，其余返回 `UnsupportedPermissions` |
| `PageMapping { phys, perm }` | 页首物理地址及权限；撤销返回此类型，不转移页面所有权 |
| `Translation { phys, perm }` | 查询得到的实际物理字节地址及权限，保留查询地址的页内偏移 |
| `KernelLayout` | 可直接构造的布局描述；字段见下文，地址空间创建时校验 |
| `VmMemory`、`VmPage` | 页资源服务与独占页句柄；复用 MM-001 分配器，不另建物理页池 |
| `KernelAddressSpace<'a, M: VmMemory, A: VmArch>` | 独占借用资源服务与架构后端，持有所属页表页和状态；不支持复制或自动释放活动资源 |

### 页资源与受限访问

```rust
pub unsafe trait VmPage {
    fn phys_addr(&self) -> PhysAddr;
    fn with_bytes<R>(&self, f: impl FnOnce(&[u8; 4096]) -> R) -> R;
    fn with_bytes_mut<R>(&mut self, f: impl FnOnce(&mut [u8; 4096]) -> R) -> R;
    fn read_word(&self, slot: usize) -> u64;
    fn write_word(&mut self, slot: usize, value: u64);
}

pub unsafe trait VmMemory {
    type Page: VmPage;
    fn managed_range(&self) -> core::ops::Range<PhysAddr>;
    fn alloc_zeroed_page(&mut self) -> Result<Self::Page, PageAllocError>;
    unsafe fn release_page(
        &mut self, page: Self::Page,
    ) -> Result<(), (PageAllocError, Self::Page)>;
}
```

- `managed_range` 返回稳定的单区间描述，不是当前空闲区间；其中已分配页仍在范围内。适配层复用 MM-001 的地址与标记，保留其错误类型。
- 申请成功返回完整、按 4096 字节对齐且已清零的普通内存页。句柄不可伪造、复制或克隆，不借用 `&mut M`；页内容的有效期覆盖句柄持有期。页内容不属于对 `M` 元数据的独占借用。服务销毁不得回收仍被句柄持有的页；真实适配沿用 MM-002 的一次性初始化和单一分配器约束。真实实现提供稳定的恒等访问路径；开发机句柄持有有效的对齐缓冲区，物理地址只是供页表使用的数值，不能直接转指针。
- 访问闭包仅在调用期间借用整页，不得把指针或引用留作后续访问。句柄不自动归还页。映射建立前后都不得同时持有物理路径与虚拟别名的冲突引用。页表项仍由 arch 解释，资源服务只提供存储。
- `read_word` / `write_word` 是硬件共享存储的补充入口：每次以对齐、完整的 64 位 volatile 操作访问一个单元，越界索引在访存前拒绝；不构造指向页内容的 Rust 引用。页表项含义仍由 arch 解释。活动页表禁止整页借用；软件串行修改，A/D 预置使硬件只读，父项发布和地址转换同步仍按下文执行。
- 申请失败不留下新占用或修改旧页，耗尽为 `OutOfMemory`。归还成功消耗句柄；失败返回原句柄及原始错误，分配标记和内容不变。来源不符返回 `InvalidAddress`，已不属于有效分配的页返回 `NotAllocated`。合法持有的页出现这些错误属于内部所有权故障，启动流程须失败关机。
- `release_page` 的调用者保证持有有效句柄、所有访问已结束、临时映射已撤销并完成同步；服务先验证句柄来源与分配状态，再归还页。页表页还须已脱离全部可用页表，不能仍是活动根。基础恒等映射本身不表示页面仍被持有。`unsafe impl` 必须证明以上有效期、独占、来源与访问保证；清零不改变 MM-001 本身不访问页内容的约定。

自检的 P、Q 在第一次 `new` 前申请。地址空间存活期间只通过独立的页句柄访问它们，不再次借用 `M`。页数、所有权和分配事件由测试适配器观察；计数器仅供外部读取，归还时须验证页面是否由该适配器计数。统计方法不属于上述最小生产接口。读取统计时不得重借用已被地址空间独占的服务。

### 布局构造与创建校验

```rust
pub struct KernelLayout {
    pub code: core::ops::Range<PhysAddr>,
    pub rodata: core::ops::Range<PhysAddr>,
    pub writable: core::ops::Range<PhysAddr>,
    pub allocatable: core::ops::Range<PhysAddr>,
    pub test_window: core::ops::Range<VirtAddr>,
}
```

这些字段允许保存待校验数值。板级层用链接符号及 MM-002 的范围构造布局；`writable` 包含数据、BSS、栈、静态标记及末尾填充。开发机可直接构造合成布局。`new` 在申请任何页表页前依次检查：

1. 各区间为非空左闭右开区间，起点小于终点；两端均按 4096 字节对齐。
2. 长度、末页和恒等虚拟地址计算不溢出；每个被映射地址满足对应物理/虚拟地址规则，区间不能跨过非规范虚拟地址空洞。
3. `code.end == rodata.start`、`rodata.end == writable.start`、`writable.end == allocatable.start`，基础区间连续且不重叠；窗口不与基础区间相交。本阶段真实窗口固定为下文的四页范围；合成布局可换窗口位置测试索引边界。
4. `allocatable` 完整位于 `M::managed_range()` 内。本阶段还要求两者起止相等，保证服务分出的任一页表页都有基础恒等映射；不从范围外取页。板级构造方另保证这些段覆盖实际装载内容，验收用 ELF 段和符号核对。

以上任一失败返回 `InvalidLayout`，不分配、不修改标记或页面。这里的“无完整可分配页”仅指布局区间为空或不足一页。布局合法但空闲页不足以建立页表时，返回 `Allocator(OutOfMemory)` 并回收本次申请；不得误报布局错误。已由 P、Q 等持有的页不影响布局合法性。

### 地址空间入口

以下为外部签名；操作均串行执行，类型经统一内存/HAL 入口导出。

| 操作 | 签名 | 成功与失败状态 |
| --- | --- | --- |
| 创建 | `new(memory: &'a mut M, arch: &'a mut A, layout: KernelLayout) -> Result<Self, VmError>` | 建好基础映射但不启用；失败归还本次全部页表页，不修改数据页。`A` 可替换为开发机后端 |
| 建立映射 | `unsafe fn map_page(&mut self, va: VirtAddr, pa: PhysAddr, perm: MapPerm) -> Result<(), VmError>` | 在自检窗口内建立一页映射；已有映射报错，不隐式替换 |
| 查询 | `query(&self, va: VirtAddr) -> Result<Option<Translation>, VmError>` | 可查询基础映射或窗口；未映射返回 `Ok(None)`，非法地址报错 |
| 撤销 | `unsafe fn unmap_page(&mut self, va: VirtAddr) -> Result<PageMapping, VmError>` | 只撤销自检窗口映射；活动时同步后再归还空的中间页表页，不释放数据页 |
| 启用 | `unsafe fn activate(&mut self) -> Result<(), VmError>` | 仅从未启用或已停用状态进入；安装并验证当前根页表，返回前完成本核同步 |
| 停用 | `unsafe fn deactivate(&mut self) -> Result<(), VmError>` | 仅从活动状态回到无分页；恢复恒等地址执行并完成同步后返回 |
| 销毁 | `destroy(&mut self) -> Result<(), VmError>` | 仅销毁未启用或已停用的地址空间，归还其全部页表页；成功后所有操作报 `Destroyed` |

创建不调用 CPU 启用操作。资源服务和架构后端的借用持续到地址空间对象结束；`destroy(&mut self)` 只改变状态并归还页表页，不单独结束这两个借用。

状态顺序为“未启用 → 活动 → 已停用 → 活动”；未启用或已停用可销毁。重复启用、重复停用或从未启用时停用返回 `InvalidState`，销毁活动页表返回 `ActiveAddressSpace`。启用前须确认 CPU 处于无分页，停用前须确认当前根表属于本对象；不匹配返回 `InvalidState`，不覆盖其他根表。不支持 Sv39 或启用读回不匹配时返回 `ActivationFailed`，恢复无分页和未启用/已停用状态。停用也须读回确认无分页后才能改变对象状态和允许销毁。切换后无法恢复或确认状态、出现意外异常时失败关机，不返回成功。

### 错误与安全前提

`VmError` 包含 `Destroyed`、`InvalidState`、`InvalidLayout`、`InvalidAddress`、`UnalignedAddress`、`ProtectedMapping`、`UnsupportedPermissions`、`AlreadyMapped`、`NotMapped`、`Allocator(PageAllocError)`、`ActiveAddressSpace`、`ActivationFailed`、`InvalidTable`。`InvalidTable` 表示页表引用了不属于该地址空间的表页，或发现本阶段不允许的表项；报告失败，不继续沿错误地址访问。耗尽保留 `Allocator(OutOfMemory)`，不吞掉底层错误。

地址包装的 `from_raw` 不做合法性或对齐校验。以下 VA、PA 分别指虚拟地址和物理地址。`map_page` 依次检查：对象状态 → VA、PA 的地址表示 → VA、PA 的页对齐 → 自检窗口 → 权限 → 已有映射 → 申请页表页。两地址中任一非法均先返回 `InvalidAddress`，不会被另一地址的不对齐掩盖。`unmap_page` 使用同一顺序中的 VA 检查，再检查映射是否存在。`query` 先检查状态，再检查 VA 合法性，保留页内偏移，不要求对齐或位于窗口。窗口外修改返回 `ProtectedMapping`，窗口内缺失映射的撤销返回 `NotMapped`。每个单独错误及相邻优先级的组合须有测试。

- `map_page` 的调用者持有完整物理页，保证它可按请求权限使用，并在撤销完成前保持有效；本步只接 RAM，不接设备地址。不得给基础代码或只读数据增加可写别名，也不得把页表页作为自检数据页。
- `unmap_page` 前结束通过该虚拟页形成的全部引用和访问；返回后不再使用旧虚拟地址。数据页只能在所有临时映射失效、引用结束后归还原分配器。基础恒等访问路径不代表所有页面都已被分配，访问仍须持有页面。
- 启用/停用只能由唯一启动核执行，保持中断关闭；当前代码、栈、异常入口、返回位置和日志路径必须在转换前后均有效。切换时不得执行自检窗口中的代码或保留跨切换的窗口引用。
- 页表页的申请、回滚及归还统一由公共内存层完成；arch 不另持分配器。合法页归还若出现内部所有权错误，应报告并终止启动，不把未归还资源标为空闲或继续报告成功。
- 不依赖 `Drop` 隐式停用或释放。活动对象由启动流程持有至关机；销毁后的资源服务借用在对象结束时归还调用者。

单页建立最多需要两个新的中间页表页。先检查目标并取得所需资源，初始化未发布的子树，再连接原页表；资源不足时回收本次新页，原映射和数据不变。撤销先使叶映射无效，再断开空的中间表，完成必要同步后归还这些页。销毁同样只回收自身页表页；基础映射的 RAM 与外部持有的数据页都不随之释放。

### 统一页表后端

公共内存层保存页表页句柄，向 arch 临时提供 `TableAccess`。它只接受本地址空间登记的页表页地址；查不到返回 `InvalidTable`。数据页句柄不进入此集合，arch 得不到申请或归还页的方法。`TableAccess` 的借用及其返回的页引用均不得逃出本次页表操作。

```rust
pub trait TableAccess {
    type Page: VmPage;
    fn table(&self, pa: PhysAddr) -> Result<&Self::Page, VmError>;
    fn table_mut(&mut self, pa: PhysAddr) -> Result<&mut Self::Page, VmError>;
}

pub struct RemovedMapping {
    pub mapping: PageMapping,
    pub retired: [Option<PhysAddr>; 2],
}

pub unsafe trait VmArch {
    fn valid_phys(&self, pa: PhysAddr) -> bool;
    fn valid_virt(&self, va: VirtAddr) -> bool;
    fn needed_tables<T: TableAccess>(
        &self, tables: &T, root: PhysAddr, va: VirtAddr,
    ) -> Result<usize, VmError>;
    unsafe fn map_page<T: TableAccess>(
        &mut self, tables: &mut T, root: PhysAddr, va: VirtAddr,
        mapping: PageMapping, spare: &[PhysAddr],
    ) -> Result<(), VmError>;
    fn query<T: TableAccess>(
        &self, tables: &T, root: PhysAddr, va: VirtAddr,
    ) -> Result<Option<Translation>, VmError>;
    unsafe fn unmap_page<T: TableAccess>(
        &mut self, tables: &mut T, root: PhysAddr, va: VirtAddr,
    ) -> Result<RemovedMapping, VmError>;
    fn current_root(&self) -> Result<Option<PhysAddr>, VmError>;
    unsafe fn activate(&mut self, root: PhysAddr) -> Result<(), VmError>;
    unsafe fn deactivate(&mut self, root: PhysAddr) -> Result<(), VmError>;
    unsafe fn sync_translations(&mut self);
}
```

| 方法 | 最小语义 |
| --- | --- |
| `valid_phys` / `valid_virt` | 只判断地址表示，不检查页对齐、窗口或实际页所有权。Sv39 的物理页号与虚拟地址符号扩展规则由该后端实现 |
| `needed_tables` | 只读检查目标，返回 0–2 个所需中间页表页；重复映射返回 `AlreadyMapped`，不申请页、不改表 |
| `map_page` | 公共层先取得全部新页、加入所属表页集合，再传入 `spare`。其长度须等于前一步结果，各页须已清零、互异且未连接；不满足返回 `InvalidState`。后端先完成地址、权限和表项检查，再发布映射；错误保留旧树，公共层撤回并归还本次新页。串行调用期间不能插入其他修改 |
| `query` | 只读遍历，返回包含页内偏移的翻译或 `None`；非法地址报 `InvalidAddress`，未知表页或非法表项报 `InvalidTable`，不直接解引用未经 `TableAccess` 确认的地址 |
| `unmap_page` | 成功返回旧映射及已从树中断开的空表地址，最多两页，不重复且不含根。返回前不释放页；`NotMapped` 等错误不改变旧树 |
| `current_root` | 无分页返回 `None`；本阶段支持的模式且地址空间编号为 0 时返回当前根；其他模式或编号返回 `InvalidState`，不修改 CPU |
| `activate` / `deactivate` | 分别要求无分页或当前根等于参数，否则 `InvalidState`。执行下节的切换、读回和同步；不能启用时恢复无分页并返回 `ActivationFailed`。无法确认恢复状态则失败关机 |
| `sync_translations` | 完成当前核的全量地址转换缓存同步及编译器访存约束后返回；无可忽略的失败分支 |

公共入口负责上文的错误优先级。页表后端也不能用非法参数或表项进行越界访问。活动映射修改后，由公共层调用同步，再返回或归还 `retired` 页；非活动修改在启用前同步。新子树发布后不再执行可能返回普通错误的步骤；意外硬件异常走失败关机。

后端的 `unsafe` 调用要求页表页仍被独占持有、内容合法、所有访问路径有效，且只有启动核串行修改；修改页表不能同时保留其内容引用。切换还要求异常入口就绪，以及当前代码、栈、返回点和日志路径在前后均可用。`unsafe impl VmArch` 须保证实际表项访问宽度、访存顺序、寄存器与同步语义，不得私自分配或释放页。

真实后端经 HAL 的 `unsafe fn init_vm_arch() -> Result<NativeVmArch, VmError>` 取得。它只建立唯一的本核访问句柄，不启用分页；重复初始化或不满足单核、关中断、无分页前提返回 `InvalidState`。开发机直接提供实现 `VmArch` 的替身，或运行真实页表编解码并替换 CPU 控制部分。统计与故障注入留在测试适配器中。

## 页表与生效

采用三级 Sv39、4 KiB 页，每张页表 512 项，每项 8 字节；仅最底层建立叶映射。虚拟地址须满足 Sv39 符号扩展规则，物理地址须可由其物理页号表示。映射和撤销输入均为页首；查询保留低 12 位偏移。页大小复用 MM-001 的常量，另用规格断言验证其值为 4096。

叶项使用内核权限（U=0），本阶段不建立全局映射（G=0），访问/脏标记 A、D 预置为 1；中间项只设置下级表地址和有效位，U/A/D/R/W/X 均为 0。未使用的软件位、扩展位及保留位清零。拒绝仅写、可写可执行、用户页及本阶段不支持的权限，不自动放宽权限。

`satp` 是控制分页模式和根页表的 CPU 寄存器。本阶段地址空间编号固定为 0。启用时先完成页表写入的编译器约束与 `sfence.vma x0, x0`，写入 Sv39 和根页号，再执行一次全量本核同步，并读回模式、编号及根页号；确认前不报告启用成功。停用写回无分页模式后也完成同步。异常入口必须在第一次切换前就绪。

活动页表的映射、撤销和回滚在返回前执行全量 `sfence.vma x0, x0`；新页发布先于父项连接，空表归还晚于同步。页表项使用对齐的完整 64 位访问，不能拆成较窄写入。架构汇编与编译器访存约束共同保证顺序；仅加 `nomem` 而无其他约束不能通过静态审阅。此约定只覆盖本核，不承诺跨核生效。

## 启动映射与装配

保持 MM-002 的启动条件及原有实际页自检。布局来自现有[板级链接脚本](../../kernel/src/platform/qemu_riscv64/linker.ld)与[资源校验](../../kernel/src/platform/qemu_riscv64/memory.rs)；不能按本次构建的数值硬编码内核末尾。

实现时在板级链接脚本导出代码、只读数据、可写内核区的页对齐边界。各区间连同填充页互不重叠，覆盖全部装载段、启动栈、BSS 和静态元数据；末端仍由 `__kernel_end` 覆盖。沿用已有链接脚本依赖追踪。

| 区间 | 虚拟地址到物理地址 | 权限与用途 |
| --- | --- | --- |
| 内核代码及该段填充 | 恒等映射 | RX，含启动、异常、探针和切换返回代码 |
| 只读数据及该段填充 | 恒等映射 | R，含日志字符串及只读表 |
| 可写内核区至 `align_up(__kernel_end, 4096)` | 恒等映射 | RW，含 `.data`、BSS、栈、页状态标记；不可执行 |
| MM-002 的可分配区 | 恒等映射 | RW，覆盖后续页表页和已申请数据页的访问路径；映射不改变分配状态 |
| 自检窗口 `[0xffff_ffc0_0000_0000, 0xffff_ffc0_0000_4000)` | 初始不映射；按测试绑定已持有的页 | 四页窗口，与恒等区不相交；撤销全部窗口映射后才能归还测试页 |

每页均为 U=0；S 模式保持 SUM=0、MXR=0，避免允许访问用户页或把仅执行权限当读取权限。低端固件、设备树保留区、RAM 外设备地址不加入本阶段基础映射。SBI 输出和关机沿用原入口，不为此添加 UART/MMIO 驱动。

顺序为：MM-002 初始化和真实页自检 → 输出 `FOCOS_MM_OK` → 校验映射布局并构造页表 → 安装同步异常入口 → 启用分页 → 自检与清理 → 输出 `FOCOS_VM_OK` → 输出 `FOCOS_BOOT_OK` → 正常关机。成功路径保持分页开启，基础页表存活至关机；停用与销毁另由专项用例验证。

任何前置步骤失败均走 `FOCOS_PANIC` 与失败关机，不输出尚未完成阶段的成功标记。已输出的 MM 标记不能掩盖后续分页失败。CPU 不支持所需模式、异常未按约定到达或超时均为失败，不回退为“仅启动通过”。

## 同步异常与探针

本阶段范围为内核分页自检所需的同步异常入口。最小接口由本页约定，并由 MM-003 与 [#5 / CORE-001](https://github.com/MindScrath/Focos/issues/5) 的负责人共同审阅。调度、时钟中断、用户上下文、系统调用及 #35 的临界区能力不在本阶段范围内。

```rust
pub enum FaultKind { InstructionPage, LoadPage, StorePage }
pub struct FaultInfo {
    pub kind: FaultKind,
    pub instruction: VirtAddr,
    pub address: VirtAddr,
}
pub enum ProbeOp {
    ReadByte { address: VirtAddr },
    WriteByte { address: VirtAddr, value: u8 },
    Execute { address: VirtAddr },
}
pub struct ProbeSite {
    pub instruction: VirtAddr,
    pub resume: VirtAddr,
}
pub struct ExpectedFault {
    pub kind: FaultKind,
    pub address: VirtAddr,
    pub site: ProbeSite,
}
pub enum ProbeError { AlreadyInstalled, InstallFailed, Busy, InvalidProbe, NotObserved }
pub unsafe trait FaultProbe {
    fn site(&self, op: &ProbeOp) -> ProbeSite;
    unsafe fn probe(
        &mut self, op: ProbeOp, expected: ExpectedFault,
    ) -> Result<FaultInfo, ProbeError>;
    unsafe fn sync_instructions(&mut self);
}
```

HAL 的统一入口为 `unsafe fn install_fault_probe() -> Result<NativeFaultProbe, ProbeError>`。调用者须处于唯一启动核、关中断、有有效内核栈的启动阶段。成功安装并读回同步异常入口后才允许启用分页；重复安装返回 `AlreadyInstalled`，入口保持不变。安装失败返回 `InstallFailed` 并终止启动。句柄不借用 `VmMemory` 或 `VmArch`，可与活动地址空间同时使用，持有至关机；开发机用 `FaultProbe` 替身验证调用方。

`site` 不执行操作，只返回对应汇编探针的故障指令标签与恢复标签。取指探针的故障位置为目标窗口地址，恢复标签仍位于内核代码。自检提供预期种类、地址与 `site` 返回值，再调用 `probe`；不能自行猜测指令长度或构造恢复地址。

`probe` 先检查有无未完成记录，再核对操作、预期种类、地址及标签的一致性。已有记录返回 `Busy` 且不覆盖它；参数不匹配返回 `InvalidProbe` 且不执行操作。随后把预期记录复制到架构层固定存储，执行恰好一次汇编操作，并在返回时清除本次记录。正常执行却未捕获异常时返回 `NotObserved`，自检必须判失败；捕获匹配异常才返回 `FaultInfo`。记录不能借用普通测试对象的可变引用，也不能跨探针保留。

`probe` 的调用者保证无冲突引用，目标为仍持有的数据页或刚撤销的测试窗口，且异常入口、栈和恢复点不依赖被测页。写探针仅使用专用测试数据，取指探针的意外成功路径必须能安全返回；不得拿日志或栈内容作破坏性测试。写入探针指令后、取指前调用 `sync_instructions`，其返回表示本核指令同步和编译器访存约束已完成。生产后端负责真实执行；替身只证明控制逻辑。

arch 读取异常原因、故障指令地址和故障地址，转换为上述 `FaultInfo`。底层匹配器也须接受事件注入测试，覆盖不匹配与重复异常；只有公共自检的替身测试不能证明真实入口正确。

只在预期记录有效且三项完全相符时，消费该记录并返回指定标签。没有异常、收到第二次异常、原因或地址错误、指令位置不符，以及其他中断/异常，都不能算预期失败测试通过。QEMU 专项还须记录原始 `scause`、`sepc`、`stval`；页异常的地址报告或固件转交不满足要求时判失败，不跳过检查。

异常入口保证栈有效、对齐及受影响寄存器恢复，不能依赖被测页。恢复直接设置已知恢复标签，不使用统一的 `sepc += 4`；目标支持压缩指令，故障指令长度不能由这种假设推出。探针用汇编标签限定单条操作；正常执行路径也要显式检查“本次确实捕获一次异常”。

## 验收与证据

以下为验收要求，实际运行结果与证据见实现 PR。开发机假内存只能证明逻辑，CPU 控制与真实访问必须在 QEMU 验证。

### 开发机与静态审阅

| 场景 | 判据 |
| --- | --- |
| 地址与查询 | 页首、末字节和页内偏移正确；底层索引及中层索引跨界时相邻页不互相覆盖；合法高地址可用，非规范地址被拒绝 |
| 基础映射与权限 | 对合成布局执行 `new`，逐页查询 code=R/!W/X、rodata=R/!W/!X、writable/allocatable=R/W/!X，全部 U=0；物理地址等于虚拟地址，覆盖各段首尾、交界及页内偏移，窗口初始无映射 |
| 权限与表项 | 独立检查 `new` 实际生成的叶项权限位和物理页号，并核对中间项、保留位及 A/D；与 R/RW/RX 编解码测试配合，不以同一个编码函数生成期望值。拒绝用户页、仅写、RWX |
| 布局与资源契约 | 字段可构造非法描述；按规定顺序拒绝空区间、不对齐、溢出、非法地址、间断/重叠、范围不覆盖，且零分配。合法布局下逐次注入耗尽，保留 `Allocator(OutOfMemory)`；页句柄访问有效，归还失败保留句柄及内容 |
| 失败优先级 | 地址表示、对齐、窗口、权限、重复/缺失及耗尽分别测试；组合错误按约定优先级返回，缓冲区和旧映射不变 |
| 申请与回滚 | 创建及映射的每个页表页申请点分别注入耗尽；比较前后剩余页数、既有查询结果和数据，不能只检查错误类型 |
| 所有权与状态 | 撤销仅归还空表；停用后销毁回收全部所属表页而保留数据页；活动销毁被拒绝；重复启停及销毁后使用明确失败 |
| 生效顺序 | 假架构记录“初始化子表 → 发布父项 → 同步 → 归还空表”；活动变更不漏同步，非活动页表在启用前统一同步 |
| 异常匹配 | 安装失败/重复安装、Busy、非法探针、未捕获异常分别有负例；底层原因、故障地址、指令地址、有效期及重复异常分别验证失败，不能报告保护生效 |
| 机器码与链接 | 核对完整页表项访问、访存顺序、寄存器保存及探针标签；用 ELF 段和符号核对 RX/R/RW 区间及覆盖范围，变更脚本须触发重新链接 |

基础页表检查使用真实页表编码/遍历实现与开发机页缓冲区，CPU 控制替换为假实现。只用假后端返回预设权限不能通过此项。原始叶项检查放在架构测试中，沿缓冲区中的表项独立核对，不向公共调用方暴露页表项。若故意把 rodata 改成 RW，或把 writable 改成可执行，这些检查必须失败。

跨页表边界测试针对未发布的映射逻辑使用合成布局，不修改真实平台的四页自检窗口。假内存逐页记录所有权，不以固定申请顺序替代行为断言。可另加 QEMU 基础 rodata 写入及 data 取指探针，使用专用哨兵和可返回指令，预期异常与意外成功都须检查；本阶段最低要求为上述开发机基础映射检查及下列窗口探针。

### QEMU 实际行为

使用既有 `virt -m 128M -smp 1 -bios default` 配置及启动脚本的其余参数，新增专项复用它们；记录实际 QEMU/OpenSBI 版本，不设置精确版本门禁。

1. 按原流程执行 MM-002，并确认两个物理页写读和回收仍通过。随后在第一次 `new` 前申请 P、Q 两个测试页句柄，填入不同图样；Q 的开头放固定的可返回探针指令，其他字节为已知图样。不能在地址空间独占借用资源服务期间再次向其申请数据页。
2. 构造页表，安装同步异常入口，再启用分页并核对 `satp` 状态；继续使用栈、读只读数据，经 SBI 输出分页启用记录。
3. 将窗口 V 映射到 P，用不会被编译器消除的逐字节操作写满并读回，从页句柄的恒等访问路径交叉核对；两路径的引用分段使用。取指测试前经 `FaultProbe::sync_instructions` 完成本核指令同步。
4. 将另一窗口页以只读权限映射到测试数据，预先读取，再尝试单字节写入；只接受预期的写页异常，核对整页未被修改。
5. 将 Q 映射为不可执行的窗口数据页，执行取指探针，只接受预期取指页异常；故障位置是目标窗口地址，恢复点仍为内核代码标签。若意外执行了探针并返回，明确判失败，不把正常返回当保护生效。
6. 对已读取过的 V 撤销映射，读取 V 必须产生预期读页异常；再把 V 映射到 Q，读到 Q 的新图样而不是 P 的旧内容。
7. 撤销全部窗口映射并完成同步，确认临时页表页被回收，基础页表不被误释放；P、Q 暂由自检持有，用于检查销毁是否误释放数据页。
8. 在同次启动中检查生命周期：活动销毁应失败；在恒等代码和栈上停用，再销毁旧地址空间。结束旧对象的作用域或在销毁成功后显式 `drop`，释放对资源服务及架构后端的借用。核对全部所属页表页已归还，P、Q 仍被占用且内容正确；先归还 P、Q，再创建并启用最终地址空间。确认仅基础页表继续占用资源后，输出 VM 成功标记并关机。最终地址空间持有至关机，不能为归还数据页而再次借用其资源服务。

内核分页保护不能阻止其受信任代码经另一可写别名修改同一数据页；只读用例检验的是被测映射。代码与只读基础区域不得存在可写别名。上述结果不等于用户/内核隔离已经完成。

### 检查入口与运行记录

`make vm-smoke` 默认复用 `scripts/smoke.py` 的配置并增加分页检查模式。标准启动与 MM-002 的既有入口保留；新模式要求 `FOCOS_MM_OK`、`FOCOS_VM_OK`、`FOCOS_BOOT_OK` 各出现一次、独占一行且顺序正确，同时要求正常退出、无 panic 或意外异常。预计异常以独立事件记录，不能使用 panic 作为成功探针。

检查器先对现有未分页内核失败，再对实现通过；自身用例覆盖标记缺失、重复、嵌入其他文字、乱序、意外异常、非零退出和超时（包括已输出成功标记后超时）。上述生命周期检查与映射探针同属 `make vm-smoke`，全部完成后才报告总成功。

实现 PR 运行 `make test`、`make check`、`make mm-smoke` 和 `make vm-smoke`；后两者各自保留日志。证据包含源码 SHA/未提交状态、ELF SHA-256、工具链、QEMU/OpenSBI 版本、完整命令、实际布局、根页表与模式、探针结果、退出状态及原始串口输出。专项实际运行稳定后，再单独提交 CI 门禁变更。

## 规范依据

- [RISC-V 特权规范 20240411：Supervisor](https://docs.riscv.org/reference/isa/v20240411/priv/supervisor.html)：依据 `sstatus`、`stvec`、`sepc/scause/stval`、`satp`、`SFENCE.VMA` 及 Sv39 章节约定页表格式与生效；固定版本用于设计，不是 QEMU 版本门禁。
- [RISC-V 20240411：Zifencei](https://docs.riscv.org/reference/isa/v20240411/unpriv/zifencei.html)：用于探针指令写入后的本核取指同步，不作为页表同步的替代。
- [OpenSBI v1.3.1 异常转交](https://github.com/riscv-software-src/opensbi/blob/v1.3.1/lib/sbi/sbi_hart.c)、[平台默认能力](https://github.com/riscv-software-src/opensbi/blob/v1.3.1/include/sbi/sbi_platform.h)、[通用平台](https://github.com/riscv-software-src/opensbi/blob/v1.3.1/platform/generic/platform.c)：核对已有环境的页异常转交路径；不同实际固件仍须通过运行验收。
- [MM-001](../../specs/MM-001.md)、[MM-002](../../specs/MM-002.md)、[PLAT-002](../../specs/PLAT-002.md)、[ARCH-001](../../specs/ARCH-001.md)：复用页分配、机器布局和分层边界。[PR #31](https://github.com/MindScrath/Focos/pull/31) 的行为验收与独立专项约定继续保留。

用户态隔离后续与 #5/#23 单独约定。
