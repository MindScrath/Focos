# MM-004 参考：用户地址空间、权限隔离与安全拷贝

本页定义 [MM-004](../../specs/MM-004.md) 的多地址空间模型、页表切换、S 模式/用户页硬件隔离、`copy_from_user`/`copy_to_user` 契约及验收清单。所有新增接口、自检与 `make user-vm-smoke` 均待实现；经另外两名成员审阅确认后再进入测试与实现。

## 接口与资源

沿用 [MM-003 参考页](sv39-paging.md#接口与资源) 的分层边界与既有类型：公共内存模块持有全部页表页与映射生命周期，arch 只通过 `TableAccess` 借用表页并执行页表编解码、根切换和用户拷贝汇编，platform 提供用户虚拟地址范围。不建立第二个物理页分配器，不向公共层暴露 RISC-V 页表项位宽、索引位移或 CSR 细节。

MM-003 的 `KernelAddressSpace<'a, M, A>` 及其开发机/QEMU 验收接口保持向后兼容，不破坏既有 `make vm-smoke` 与 `scripts/test_vm_api.py`。为支持多个用户地址空间并存、在分页开启期间动态申请数据页并在不同页表间直接切换，本阶段新增非永久借用 `VmMemory`/`VmArch` 的 `KernelBase` 与共享借用它的 `UserAddressSpace`。

| 类型 | 含义 |
| --- | --- |
| `MapPerm`（扩展） | 在 MM-003 的内核 `R`、`RW`、`RX` 基础上新增 `USER_R`、`USER_RW`、`USER_RX`（`user = true`）；提供 `supported_kernel()`、`supported_user()` 与 `supported_leaf()` |
| `KernelBase<P: VmPage>` | 持有不可变内核基础映射（`code`、`rodata`、`writable`、`allocatable`）的根表与内核侧页表页，不持久借用 `M` 或 `A`，不含可变 `test_window` |
| `UserAddressSpace<'k, P: VmPage>` | 共享借用 `&'k KernelBase<P>`，独占持有自身根页表与用户侧中间页表页及 `user_range`；不支持 `Copy`/`Clone` 或隐式 `Drop` 释放 |
| `UserCopyError` | 受控用户拷贝的可恢复错误：`Destroyed`、`InvalidState`、`InvalidAddress`、`OutsideUserRange`、`Busy`、`Fault { fault, copied }` |
| `UserAccess` | HAL/arch 提供的受控用户内存读写契约；在 RISC-V 上由现有唯一异常入口句柄 `NativeFaultProbe` 实现 |

### 权限常量与域边界

```rust
impl MapPerm {
    pub const USER_R: Self = Self { read: true, write: false, execute: false, user: true };
    pub const USER_RW: Self = Self { write: true, ..Self::USER_R };
    pub const USER_RX: Self = Self { execute: true, ..Self::USER_R };

    pub fn supported_kernel(self) -> bool {
        self.read && !self.user && !(self.write && self.execute)
    }
    pub fn supported_user(self) -> bool {
        self.read && self.user && !(self.write && self.execute)
    }
    pub fn supported_leaf(self) -> bool {
        self.read && !(self.write && self.execute)
    }
}
```

- `MapPerm::supported()` 保持原有语义（等价于 `supported_kernel()`），保证 MM-003 既有调用方与负例行为不变。
- `KernelAddressSpace::map_page` 只接受 `supported_kernel()`，继续拒绝 `user: true`。
- `UserAddressSpace::map_page` 只接受 `supported_user()`，拒绝 `user: false`、仅写、仅执行、空权限与可写可执行（`W+X`）。
- 查询时同时校验权限域：`KernelAddressSpace::query` 遇到 `perm.user == true` 返回 `InvalidTable`；`UserAddressSpace::query` 在 `user_range` 内遇到 `perm.user == false` 或在 `user_range` 外遇到 `perm.user == true` 均返回 `InvalidTable`。

### `KernelBase` 与 `UserAddressSpace` 生命周期

`KernelBase<P>` 代表启动后保持不变的内核基础页表模板。它支持两种建立入口，两种入口都保证 `KernelBase` 不含任何可变自检窗口映射：

| 操作 | 签名 | 成功与失败状态 |
| --- | --- | --- |
| 从已启用的 `KernelAddressSpace` 转交 | `KernelAddressSpace::into_kernel_base(self) -> Result<KernelBase<M::Page>, (VmError, Self)>` | 仅当对象为 `Active`、CPU 当前根匹配且 `test_window` 全部未映射时消耗 `self`、结束对 `M`/`A` 的独占借用并返回活动态 `KernelBase`；否则返回 `(Destroyed / InvalidState, self)` |
| 独立创建 | `KernelBase::new<M: VmMemory<Page = P>, A: VmArch>(memory: &mut M, arch: &mut A, layout: KernelLayout) -> Result<Self, VmError>` | 沿用 MM-003 的完整 `KernelLayout` 校验，仅建立 `code`/`rodata`/`writable`/`allocatable` 基础恒等映射（不开放 `test_window` 修改），返回未启用态 `KernelBase`；失败回收全部表页 |
| 根与基础区间 | `root(&self) -> PhysAddr`、`base_range(&self) -> Range<VirtAddr>` | 只读查询根物理页首及不可变内核基础虚拟区间 `[code.start, allocatable.end)` |
| 查询 | `query<A: VmArch>(&self, arch: &A, va: VirtAddr) -> Result<Option<Translation>, VmError>` | 只读查询；已销毁返回 `Destroyed`，非法地址返回 `InvalidAddress`，发现 `user == true` 返回 `InvalidTable` |
| 启用 / 停用 | `unsafe fn activate<A: VmArch>(&mut self, arch: &mut A) -> Result<(), VmError>` / `unsafe fn deactivate<A: VmArch>(&mut self, arch: &mut A) -> Result<(), VmError>` | 用于 `KernelBase` 与 Bare 模式间转换；`deactivate` 要求自身为 `Active` 且 CPU 当前根等于 `self.root`（防止未停用的泄漏用户空间绕过停用）；因需 `&mut self`，任何 `UserAddressSpace<'k, P>` 存活时均由编译器禁止调用 |
| 销毁 | `destroy<M: VmMemory<Page = P>, A: VmArch>(&mut self, memory: &mut M, arch: &A) -> Result<(), VmError>` | 仅销毁 `Inactive` 对象并归还全部内核表页；自身仍为 `Active` 或 CPU 当前根非 `None` 时返回 `ActiveAddressSpace`。因需 `&mut self`，存在任何 `UserAddressSpace` 时由编译器静态禁止 |

`UserAddressSpace<'k, P>` 通过共享借用 `&'k KernelBase<P>` 绑定到唯一内核基础模板，多个 `UserAddressSpace` 可同时存在并共享同一套不可变内核下级页表，而各自独占自身根表与用户侧中间表页（无堆阶段每个用户空间持有的固定表页句柄容量不小于 32 页，足够覆盖根表、L1 与多个用户 L0 表；容量不足与物理页耗尽均报 `Allocator(OutOfMemory)`）：

| 操作 | 签名 | 成功与失败状态 |
| --- | --- | --- |
| 创建 | `UserAddressSpace::new<M: VmMemory<Page = P>, A: VmArch>(kernel: &'k KernelBase<P>, memory: &mut M, arch: &mut A, user_range: Range<VirtAddr>) -> Result<Self, VmError>` | 先校验 `kernel` 未销毁及 `user_range` 合法且与内核基础区间顶层槽位分离，再申请 1 张已清零根表并复制内核基础顶层项；失败归还根表，返回未活动对象 |
| 映射用户页 | `unsafe fn map_page<M: VmMemory<Page = P>, A: VmArch>(&mut self, memory: &mut M, arch: &mut A, va: VirtAddr, pa: PhysAddr, perm: MapPerm) -> Result<(), VmError>` | 仅在 `user_range` 内建立用户页映射，不接管数据页；失败保留旧树并回收本次新表页，活动时同步后返回 |
| 查询 | `query<A: VmArch>(&self, arch: &A, va: VirtAddr) -> Result<Option<Translation>, VmError>` | 可查询用户区或内核基础区，保留页内偏移并校验权限域；未映射返回 `Ok(None)` |
| 撤销用户页 | `unsafe fn unmap_page<M: VmMemory<Page = P>, A: VmArch>(&mut self, memory: &mut M, arch: &mut A, va: VirtAddr) -> Result<PageMapping, VmError>` | 仅撤销 `user_range` 内映射；活动时先完成本核同步，再归还空的用户侧中间表页，不释放根表、内核表页或数据页 |
| 从内核切入 | `unsafe fn activate<A: VmArch>(&mut self, arch: &mut A) -> Result<(), VmError>` | 仅当本空间为 `Inactive` 且 CPU 当前根等于 `kernel.root()` 时，调用 `switch_root` 切换至本空间根并置 `Active` |
| 切回内核 | `unsafe fn deactivate<A: VmArch>(&mut self, arch: &mut A) -> Result<(), VmError>` | 仅当本空间为 `Active` 且 CPU 当前根等于自身根时，调用 `switch_root` 切回 `kernel.root()` 并置 `Inactive` |
| 用户空间直切 | `unsafe fn switch_to<A: VmArch>(&mut self, next: &mut UserAddressSpace<'k, P>, arch: &mut A) -> Result<(), VmError>` | 要求二者绑定同一 `KernelBase`（指针相等）、`self` 为 `Active` 且 CPU 根为 `self.root`、`next` 为 `Inactive`；调用 `switch_root` 直接切换，避免退回 Bare |
| 安全读取用户内存 | `unsafe fn copy_from_user<A: VmArch, U: UserAccess>(&self, arch: &A, access: &mut U, user_addr: VirtAddr, dst: &mut [u8]) -> Result<(), UserCopyError>` | 要求本空间活动且 `[user_addr, user_addr + dst.len())` 完整位于 `user_range`；坏指针受控报错并保证 `SUM=0` |
| 安全写入用户内存 | `unsafe fn copy_to_user<A: VmArch, U: UserAccess>(&self, arch: &A, access: &mut U, user_addr: VirtAddr, src: &[u8]) -> Result<(), UserCopyError>` | 同上；写入只读或未映射用户页触发受控 `StorePage` 故障并返回 `Fault`，保证 `SUM=0` |
| 销毁 | `destroy<M: VmMemory<Page = P>, A: VmArch>(&mut self, memory: &mut M, arch: &A) -> Result<(), VmError>` | 仅销毁未活动空间，归还自身根表及全部用户侧表页，绝不触碰 `KernelBase` 表页或用户数据页；成功后所有入口报 `Destroyed` |

与 MM-003 一致，`KernelBase` 和 `UserAddressSpace` 均使用 `ManuallyDrop` 持有表页集合，其 `Drop` 不隐式操作 CPU 或归还页。调用者须显式停用并调用 `destroy`，再用创建时的同一 `VmMemory` 归还资源；归还时若底层分配器返回所有权错误，保留句柄并终止启动。

### 错误与安全前提

以下顺序为公共入口的强制错误优先级，组合错误按先到先返验证：

1. **`UserAddressSpace::new`**：`kernel` 已销毁 → `Destroyed`；`user_range` 不满足非空、页对齐、下界 `>= PAGE_SIZE`、不溢出、用户规范地址或顶层槽位与内核基础区间分离 → `InvalidLayout`（零分配）；申请根表失败 → `Allocator(OutOfMemory)`；复制内核顶层项失败 → 回收根表并返回后端错误。
2. **`UserAddressSpace::map_page`**：`Destroyed` → 活动态 CPU 根不匹配 `InvalidState` → VA/PA 地址表示非法 `InvalidAddress`（优先于对齐） → VA/PA 未对齐 `UnalignedAddress` → VA 不在 `user_range` `ProtectedMapping` → 权限不满足 `supported_user()` `UnsupportedPermissions` → 重复映射 `AlreadyMapped` → 申请表页或表容量耗尽 `Allocator(OutOfMemory)`。
3. **`UserAddressSpace::unmap_page`**：`Destroyed` → 活动态 CPU 根不匹配 `InvalidState` → VA 非法 `InvalidAddress` → VA 未对齐 `UnalignedAddress` → VA 不在 `user_range` `ProtectedMapping` → 未映射 `NotMapped`。
4. **`UserAddressSpace::copy_from_user` / `copy_to_user`**：`Destroyed` → 非活动态或 CPU 当前根不等于自身根 `InvalidState` → `user_addr` 非法、`user_addr + len` 溢出、末字节非法或区间跨过非规范地址空洞 `InvalidAddress` → 区间未完整落于 `user_range`（含空指针保护区及内核映射区）`OutsideUserRange` → 后端前置状态不符 `InvalidState` 或已有未消费异常记录 `Busy` → 硬件页异常 `Fault { fault, copied }`。
   - **零长度特例**：`len == 0` 仍执行上述前四步检查；若 `user_addr` 非法报 `InvalidAddress`，若不在 `user_range`（例如 `0x0`、内核地址或等于 `user_range.end`）报 `OutsideUserRange`；合法且位于 `user_range` 内时直接返回 `Ok(())`，**不访存且不置位 `SUM`**。

安全前提：
- `map_page` 调用者保证物理页在撤销并同步或非活动地址空间销毁前持续被持有，不给内核代码/只读段建立用户可写别名，不把页表页当作用户数据页映射。
- `UserAddressSpace` 的修改与切换均在唯一启动核、关中断下串行执行；`destroy` 传入的 `VmMemory` 必须是分配该空间表页的同一服务。
- `copy_from_user` / `copy_to_user` 的内核缓冲区 `dst`/`src` 必须位于有效内核映射中，且**不得与目标用户区间 `[user_addr, user_addr + len)` 别名指向同一物理内存**，同时不得与物理页句柄的并发可变借用重叠。
- 切换根页表期间，内核代码、只读数据、栈、异常入口与日志路径必须在所有用户地址空间中通过共享的内核基础顶层项持续有效。

### 统一页表后端扩展（`VmArch`）

保留 MM-003 的 `TableAccess`、`RemovedMapping` 与全部 `VmArch` 既有方法，仅补充以下三项架构无关能力：

```rust
pub unsafe trait VmArch {
    // ... 保留 MM-003 全部既有方法；map_page/query 现支持全部 supported_leaf() 权限 ...

    /// 校验用户虚拟区间：非空、页对齐、下界 >= PAGE_SIZE、不溢出、
    /// 全部地址为合法用户规范地址，且与 kernel_range 的顶层页表槽位完全分离。
    fn valid_user_range(
        &self,
        user_range: core::ops::Range<VirtAddr>,
        kernel_range: core::ops::Range<VirtAddr>,
    ) -> bool;

    /// 将 src_root 中覆盖 kernel_range 的顶层页表项复制到已清零的 dst_root。
    /// 仅读取 src_root 与其指向的下级表合法性，只写 dst_root，不修改 src_root。
    unsafe fn copy_kernel_root<T: TableAccess>(
        &mut self,
        tables: &mut T,
        dst_root: PhysAddr,
        src_root: PhysAddr,
        kernel_range: core::ops::Range<VirtAddr>,
    ) -> Result<(), VmError>;

    /// 在分页开启状态下直接将活动根从 expected_current 切换为 next，
    /// 执行准备检查、切换前后全量本核同步及读回确认。
    unsafe fn switch_root(
        &mut self,
        expected_current: PhysAddr,
        next: PhysAddr,
    ) -> Result<(), VmError>;
}
```

公共内存层在操作 `UserAddressSpace` 时构造只读叠加视图 `UserTableView` 实现 `TableAccess`：
- `table(pa)` 先查用户空间自身表页集合，未命中再只读查 `KernelBase` 表页集合（因此 `connected` 环路/复用检测和内核区 `query` 均有效）。
- `table_mut(pa)` **只查用户空间自身表页集合**；对任何内核表页返回 `Err(VmError::InvalidTable)`，在接口层杜绝用户操作改写内核页表页。

`copy_kernel_root` 必须先检查 `dst_root != src_root`、二者均合法对齐、`dst_root` 全零，并验证 `src_root` 中覆盖 `kernel_range` 的每个顶层槽位均为合法中间项且指向已登记表页，再以完整 64 位 `write_word` 复制；未覆盖槽位保持 0，不复制 MM-003 `test_window` 所在槽位。

`switch_root(expected_current, next)` 依次检查：
1. `valid_phys` 与页对齐（分别报 `InvalidAddress`、`UnalignedAddress`）。
2. `current_root()? == Some(expected_current)`（否则 `InvalidState`，不修改 CPU）。
3. `prepare()` 确认关中断、异常入口已安装且清除并读回确认 `SUM=0`、`MXR=0`（否则 `InvalidState`）。
4. 执行 `fence()` → 写入新 `satp`（模式 Sv39、ASID=0、PPN=`next >> 12`）→ `fence()` → 读回确认 `read_satp() == expected_value`。
5. 若读回不匹配，恢复 `expected_current` 并 `fence()`、读回确认后返回 `ActivationFailed`；若恢复也无法确认，直接 panic 关机。

## Sv39 编码与地址布局

在 MM-003 基础上扩展 Sv39 叶项编码，其余规则（三级页表、4 KiB 页、无大页、G=0、A=D=1、保留位与软件位清零、ASID=0、完整 64 位 volatile 访问与发布顺序）保持不变：

| 权限 | `user` | 低 10 位标志（`V|R|W|X|U|G|A|D|RSW`） | 允许使用的空间 |
| --- | --- | --- | --- |
| `MapPerm::R` | `false` | `0xc3` (`V|R|A|D`) | `KernelAddressSpace` / `KernelBase` |
| `MapPerm::RW` | `false` | `0xc7` (`V|R|W|A|D`) | `KernelAddressSpace` / `KernelBase` |
| `MapPerm::RX` | `false` | `0xcb` (`V|R|X|A|D`) | `KernelAddressSpace` / `KernelBase` |
| `MapPerm::USER_R` | `true` | `0xd3` (`V|R|U|A|D`) | `UserAddressSpace` |
| `MapPerm::USER_RW` | `true` | `0xd7` (`V|R|W|U|A|D`) | `UserAddressSpace` |
| `MapPerm::USER_RX` | `true` | `0xdb` (`V|R|X|U|A|D`) | `UserAddressSpace` |
| 中间页表项（L2/L1） | — | `0x01` (`V`；`U/G/A/D/R/W/X` 必须全为 0) | 全部空间 |

- **非叶项禁止置 `U=1`**：按 RISC-V 特权规范 Sv39，中间页表项必须保持 `U=0`、`A=0`、`D=0`；`decode` 遇非法非叶项返回 `InvalidTable`。
- **32 位开发机构建兼容**：`valid_user_range` 与 Sv39 规范地址判定不得在 32 位 `usize` 目标（如 `i686-unknown-linux-gnu`）上产生位移溢出编译警告或未定义行为，须显式处理位宽边界。

QEMU `virt` 平台在 `kernel/src/platform/qemu_riscv64/memory.rs` 导出 `pub fn user_address_range() -> Range<VirtAddr>`（经 `platform` 统一导出）：

| 虚拟区间 | Sv39 顶层槽位 `VPN[2]` | 用途与权限域 |
| --- | --- | --- |
| `[0x0000_0000_0000_0000, 0x0000_0000_0001_0000)` | 0（低 64 KiB） | 空指针保护区；禁止映射，拷贝入口直接返回 `OutsideUserRange` |
| `[0x0000_0000_0001_0000, 0x0000_0000_4000_0000)` | 0（64 KiB .. 1 GiB） | MM-004 用户虚拟地址范围（`U=1`）；每个用户空间独占 L1/L0 表 |
| `[0x0000_0000_8020_0000, 0x0000_0000_8800_0000)` 内的内核基础段 | 2（2 GiB .. 3 GiB） | 不可变内核基础恒等映射（`U=0`）；由各用户空间在 L2 槽位只读共享 |
| `[0xffff_ffc0_0000_0000, 0xffff_ffc0_0000_4000)` | 256（高半区） | MM-003 自检窗口；不复制到 `KernelBase` 或任何 `UserAddressSpace` |

该分区在 L2 槽位上同时与 QEMU `virt` RAM（`VPN[2] = 2`）及后续 VisionFive2 RAM 起址 `0x4000_0000`（`VPN[2] = 1`）互不相交；后续若迁移高半区内核，只需调整板级区间而无需修改公共层逻辑。

## 权限隔离与安全用户拷贝

### 硬件隔离边界

依据 RISC-V 特权规范（Supervisor 20240411）：
- **`SUM=0`（常态）**：S 模式对任何 `U=1` 页的加载/存储均触发读页异常（`scause = 13`）或写页异常（`scause = 15`）。
- **S 模式取指禁令**：无论 `SUM` 为 0 还是 1，S 模式**永远不能**从 `U=1` 页取指执行；尝试执行 `USER_RX` 页必须触发取指页异常（`scause = 12`）。
- **`SUM=1`（仅受控拷贝内）**：RISC-V 在 `SUM=1` 时仍允许 S 模式访问 `U=0` 内核页，因此**硬件 `SUM` 单独不足以阻止恶意内核指针**。公共层必须在调用 `UserAccess` 前严格完成 `user_range` 包含检查，对核外地址零访存拒绝。
- **U 模式执行侧范围声明**：MM-004 尚未实现进入 U 模式（`SPP=0` 的 `sret`、`sscratch` 内核栈切换与用户上下文保存）。因此“U 模式代码访问 `U=0` 内核页触发硬件页异常”留待 #5 上下文与异常入口合入后联调验收；MM-004 在开发机独立审计所有内核基础叶项确为 `U=0`，并在 QEMU 实测 S 模式侧全部硬件隔离行为。

### `UserAccess` 接口与故障恢复协议

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserCopyError {
    Destroyed,
    InvalidState,
    InvalidAddress,
    OutsideUserRange,
    Busy,
    Fault {
        fault: FaultInfo,
        copied: usize,
    },
}

pub unsafe trait UserAccess {
    unsafe fn copy_from_user(
        &mut self,
        user_addr: VirtAddr,
        dst: &mut [u8],
    ) -> Result<(), UserCopyError>;

    unsafe fn copy_to_user(
        &mut self,
        user_addr: VirtAddr,
        src: &[u8],
    ) -> Result<(), UserCopyError>;
}
```

在 RISC-V 架构层，现有唯一同步异常入口句柄 `NativeFaultProbe` 同时实现 `FaultProbe`（MM-003）与 `UserAccess`（MM-004），复用同一个 `stvec` 入口 `focos_vm_trap_entry` 与固定匹配记录 `RECORD`，不安装第二个异常入口：

1. **前置检查**：`copy_from_user` / `copy_to_user` 进入汇编前，检查 `user_addr + len` 不溢出（`InvalidAddress`）、`!local_irqs_enabled()`、`fault::installed()`、`sstatus` 中 `SUM == 0 && MXR == 0`（不满足返回 `InvalidState`，绝不改写 `sstatus`），以及 `RECORD` 空闲（占用返回 `Busy`，绝不覆盖未消费记录）。
2. **范围匹配器**：`RECORD` 扩展为支持单地址精确探针（MM-003）或区间用户拷贝守卫（MM-004：`expected_kind` 为 `LoadPage`/`StorePage`、`instruction` 为唯一拷贝指令标签、`range` 为 `[user_addr, user_addr + len)`、`resume` 为拷贝故障恢复标签）。仅当 `scause`、`sepc` 完全匹配且 `stval` 落在 `range` 内时才消费单次记录并恢复到 `resume`；若内核缓冲区 `dst`/`src` 触发异常（`stval` 不在 `range` 内）、原因/PC 不符或重复异常，一律输出 `FOCOS_VM_UNEXPECTED`、`FOCOS_PANIC` 并失败关机。
3. **异常入口清零 `SUM`**：`focos_vm_trap_entry` 保存 `sstatus` 后、调用 `focos_vm_trap` 前必须立即清除 `sstatus.SUM`（`1 << 18`），确保 Rust 异常处理与控制台日志输出始终在 `SUM=0` 下执行。
4. **逐字节拷贝汇编与出口清零**：
   - 使用固定单指令标签的逐字节循环（`lbu` 读用户 / `sb` 写内核，或 `lbu` 读内核 / `sb` 写用户），天然支持任意对齐和跨页边界，不触发非对齐异常，且能精确返回已拷贝字节数 `copied`。
   - 仅在进入循环前置位 `SUM=1`，在正常结束与故障恢复共享出口处无条件执行 `csrc sstatus, (1 << 18)` 清零 `SUM`。
   - 返回 Rust 后立即消费并清空 `RECORD`，并**读回 `sstatus` 断言 `SUM == 0 && MXR == 0`**；若读回不为 0 则立即 panic 关机，绝不带着 `SUM=1` 返回调用者。
5. **部分拷贝与错误输出语义**：
   - **前置错误**（`Destroyed`、`InvalidState`、`InvalidAddress`、`OutsideUserRange`、`Busy`）：零访存，`dst` 和用户页均原样不变，`SUM` 从未置位。
   - **`copy_from_user` 触发 `LoadPage`**：`fault.address == user_addr + copied`，`dst[..copied]` 保留异常前已读字节，**`dst[copied..]` 必须全部清零**（避免暴露内核栈/缓冲区旧内容；首字节即失效时 `copied == 0`，整块 `dst` 清零）。
   - **`copy_to_user` 触发 `StorePage`**：`fault.address == user_addr + copied`，`[user_addr, user_addr + copied)` 可能已被硬件写入，故障页及之后字节未被修改；首字节失效（如写只读用户页）时 `copied == 0`，目标用户页完全不变。
   - 捕获到预期用户拷贝页异常时记录 `FOCOS_USER_FAULT scause=... sepc=... stval=... copied=...`，不以 panic 作为预期故障成功标记。

## 验收与证据

以下为验收门槛，实际结果与证据留待实现 PR 补充。开发机假后端只验证逻辑与页表字节，硬件 `U` 位、`SUM` 与多空间 TLB 切换必须在 QEMU 验证。

### 开发机与静态审阅

| 场景 | 判据 |
| --- | --- |
| 用户范围与顶层槽位 | `valid_user_range` 拒绝空区间、不对齐、溢出、非规范地址、高半区地址、包含低 64 KiB / 空页地址、以及与内核基础区间共享顶层 `VPN[2]` 槽位的区间；`UserAddressSpace::new` 拒绝非法布局且零分配 |
| `U` 位与权限编码 | 独立读取真实页表字节，核对 `USER_R=0xd3`、`USER_RW=0xd7`、`USER_RX=0xdb`、非叶项 `0x01`（`U=A=D=0`）及内核基础叶项全部 `U=0`；负例将内核叶项置 `U=1` 或用户叶项清 `U=0`，`query` 均返回 `InvalidTable` |
| 权限与错误优先级 | `KernelAddressSpace` 拒绝 `user: true`，`UserAddressSpace` 拒绝 `user: false`、仅写、仅执行、空权限和 `W+X`；`map_page`/`unmap_page`/`copy_*` 组合错误严格按约定优先级返回 |
| 内核表不可变与回收 | `UserTableView::table_mut` 对内核表页返回 `InvalidTable`；`UserAddressSpace::new` 与 `map_page` 每个页表页申请点注入 `OutOfMemory`，前后页数与旧映射完全不变；撤销仅回收空的用户中间表，销毁仅回收该用户空间的根表和用户表页，不释放内核表页或数据页 |
| 多空间与状态机 | 两个并存 `UserAddressSpace` 映射同一 VA 到不同 PA 互不干扰；`KernelBase → A → B → KernelBase` 及重复激活、跨不同 `KernelBase` 切换、活动销毁、`switch_root` 读回不匹配恢复失败均按契约处理 |
| 拷贝状态机与故障注入 | 事件注入覆盖 `copy_from_user`/`copy_to_user` 的零长度（合法与核外/空指针）、越界/溢出、`Busy`、`SUM!=0` 初态拒绝、首字节故障、跨页偏移故障（验证 `copied` 与 `dst[copied..]` 清零）、`stval` 越出拷贝区间或重复异常触发意外失败 |
| API 与编译边界 | `scripts/test_vm_api.py` 扩展调用方编译测试：存在 `UserAddressSpace` 时调用 `kernel_base.destroy()`/`deactivate()` 编译失败（`E0502`）；漏标 `unsafe` 调用 `UserAddressSpace`/`UserAccess` 入口报 `E0133`；`x86_64` 与 `i686` 均通过 `cargo clippy -- -D warnings` |
| 汇编与静态审阅 | 核对 `fault.S` 入口清零 `SUM`、拷贝循环仅单条标签指令、出口无条件清零 `SUM`、读回检查及寄存器/栈对齐；核对 `kernel/src/mm/` 不含架构条件编译 |

### QEMU 实际行为

复用 `virt -m 128M -smp 1 -bios default`；在 MM-003 自检输出 `FOCOS_VM_OK` 后，不退回 Bare，调用 `into_kernel_base()` 接管活动内核页表并执行下列步骤：

1. 在 `KernelBase` 活动状态下向 `VmMemory` 申请 4 个独立数据页 `P`、`Q`、`R`、`C`，写入不同已知图样（`C` 开头写入 `hal::probe_return_code()`）。
2. 基于同一 `&kernel_base` 与 `platform::user_address_range()` 创建两个并存用户地址空间 `space_a`、`space_b`；确认 `into_kernel_base` 在 `test_window` 非空时会拒绝（单测验证）且 `UserAddressSpace` 不继承 `test_window`。
3. 在 `space_a` 映射 `0x1_0000 -> P (USER_RW)`、`0x1_1000 -> Q (USER_R)`、`0x1_2000 -> C (USER_RX)`、`0x1_4000 -> R (USER_RW)`，保留 `0x1_3000` 与 `0x1_5000` 未映射；在 `space_b` 映射同一虚拟页 `0x1_0000 -> Q (USER_RW)`。
4. 执行 `space_a.activate(&mut arch)`，确认 `satp` 读回为 `space_a.root` 且 `SUM=0`、`MXR=0`。
5. **S 模式硬件隔离探针（`SUM=0`）**：
   - 对 `0x1_0000` 执行 `ReadByte` 探针，仅接受 `LoadPage`（`scause=13`）；
   - 对 `0x1_0000` 执行 `WriteByte` 探针，仅接受 `StorePage`（`scause=15`），核对 `P` 未被修改；
   - `sync_instructions()` 后对 `0x1_2000`（`USER_RX`）执行 `Execute` 探针，仅接受 `InstructionPage`（`scause=12`）；若意外执行返回判失败。
6. **受控用户拷贝与坏指针测试（在 `space_a`）**：
   - `copy_to_user(0x1_0000, pattern_a)` 后以 `copy_from_user` 与 `P.with_bytes` 交叉核对数据一致，且 `SUM` 读回为 0；`copy_from_user(0x1_1000, buf)` 读回 `Q` 的图样；
   - 传入内核代码/可写地址与空指针区（`0x0`、`0x1000`）调用读写拷贝，均返回 `OutsideUserRange` 且缓冲区与内核页不变；
   - `copy_from_user(0x1_3000, buf)` 返回 `Fault { kind: LoadPage, address: 0x1_3000, copied: 0 }` 且 `buf` 全零；
   - `copy_to_user(0x1_1000, src)` 返回 `Fault { kind: StorePage, address: 0x1_1000, copied: 0 }` 且 `Q` 整页未改；
   - 跨页 `copy_from_user(0x1_4ff8, &mut buf16)` 返回 `Fault { kind: LoadPage, address: 0x1_5000, copied: 8 }`，`buf16[..8]` 匹配 `R[0xff8..]`、`buf16[8..]` 全零；跨页 `copy_to_user(0x1_4ff8, &src16)` 返回 `Fault { kind: StorePage, address: 0x1_5000, copied: 8 }`，`R[0xff8..]` 匹配 `src16[..8]`；每次失败后 `SUM` 均读回为 0。
7. **跨空间直切与隔离**：
   - `space_a.switch_to(&mut space_b, &mut arch)`，核对 `satp` 切至 `space_b.root`；在 `space_b` 读取 `0x1_0000` 得到 `Q` 而非 `P`；写入 `pattern_b` 后核对 `Q` 更新而 `P` 仍为 `pattern_a`；
   - 对活动态 `space_b` 调用 `destroy` 返回 `ActiveAddressSpace`；
   - `space_b.switch_to(&mut space_a, &mut arch)` 切回 `space_a`，确认 `0x1_0000` 仍读到 `pattern_a`。
8. **撤销、销毁与零泄漏回收**：
   - `space_a.deactivate(&mut arch)` 切回 `kernel_base.root()`；
   - 在 `space_a` 撤销一页并验证空用户表页回收；分别销毁 `space_a`、`space_b` 并结束其作用域，核对仅剩 `kernel_base` 表页与 4 个数据页；
   - 归还 `P`、`Q`、`R`、`C`，核对最终存活页数精确回到进入用户自检前 `kernel_base` 持有的基础表页数；输出 `FOCOS_USER_VM_OK`、`FOCOS_BOOT_OK` 并保持分页开启正常关机。

### 检查入口与运行记录

新增 `make user-vm-smoke`（对应 `scripts/smoke.py --user-vm`），要求 `FOCOS_MM_OK`、`FOCOS_VM_OK`、`FOCOS_USER_VM_OK`、`FOCOS_BOOT_OK` 各出现一次、独占一行且顺序正确，无 panic 或意外异常，退出码为 0。既有 `make check`、`make mm-smoke`、`make irq-smoke`、`make vm-smoke` 全部保留且不回退；CI 的 `bootstrap` 作业在 `vm-smoke` 后增加 `make user-vm-smoke` 并上传原始日志。

实现 PR 须提供源码 SHA、ELF SHA-256、工具链与 QEMU/OpenSBI 版本、完整命令、原始串口日志及失败回归记录（旧内核触发 `make user-vm-smoke` 失败，去掉 `SUM` 清零或破坏 `user_range` 检查的变异用例触发断言失败）。

## 规范依据

- [RISC-V 特权规范 20240411：Supervisor](https://docs.riscv.org/reference/isa/v20240411/priv/supervisor.html)：依据 `sstatus`（`SUM`/`MXR`/`SIE`）、`stvec`、`sepc`/`scause`/`stval`、`satp`、`SFENCE.VMA` 及 Sv39 页表项 `U` 位与 S 模式取指禁止规则。
- [RISC-V 20240411：Zifencei](https://docs.riscv.org/reference/isa/v20240411/unpriv/zifencei.html)：用于写入探针指令后执行 `USER_RX` 取指拒绝检查前的本核指令缓存同步。
- [Rust 内联汇编参考](https://doc.rust-lang.org/reference/inline-assembly.html)与[Rust 未定义行为参考](https://doc.rust-lang.org/reference/behavior-considered-undefined.html)：依据寄存器/访存约束、`SUM` 开关汇编不使用 `nomem`，以及跨用户/内核缓冲区无别名冲突的 `unsafe` 前提。
- [MM-003](../../specs/MM-003.md)、[MM-003 参考页](sv39-paging.md)、[ARCH-001](../../specs/ARCH-001.md)、[PLAT-002](../../specs/PLAT-002.md)：复用页表格式、同步异常契约、单核关中断边界与板级布局。

U 模式执行进入、`sscratch` 栈切换、ELF 加载与系统调用由 [#5](https://github.com/MindScrath/Focos/issues/5) 衔接；`mmap`/`munmap`/`brk` 与真板内存接入在 [#4](https://github.com/MindScrath/Focos/issues/4) 后续专项推进。
