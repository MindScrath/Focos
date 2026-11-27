//! 页表与同步异常的架构能力；只借用公共内存层登记的页。

use crate::mm::{PageMapping, PhysAddr, Translation, VirtAddr, VmError, VmPage};
use core::ops::Range;

/// 只允许访问当前地址空间登记的表页，不提供分配和释放能力。
pub trait TableAccess {
    /// 资源服务交付的独占表页类型。
    type Page: VmPage;
    /// 只读借用已登记的表页；未知地址返回 InvalidTable，不按数值直接解引用。
    fn table(&self, pa: PhysAddr) -> Result<&Self::Page, VmError>;
    /// 独占借用已登记的表页；未知地址返回 InvalidTable。
    fn table_mut(&mut self, pa: PhysAddr) -> Result<&mut Self::Page, VmError>;
}
/// 已撤销的映射及已断开、尚未归还的中间页表。
pub struct RemovedMapping {
    /// 被撤销的数据页及其权限，数据页仍由外部持有。
    pub mapping: PageMapping,
    /// 最多两个已断开空表，不重复且不含根；同步完成后才可归还。
    pub retired: [Option<PhysAddr>; 2],
}

/// 无分配器的页表后端，负责编码、遍历和本核 CPU 状态。
///
/// # Safety
/// 只能通过 TableAccess 借用已登记页，不保留引用；失败不改变旧树。
/// 发布前完成全部校验；切换须读回确认，无法恢复确定状态时终止运行。
pub unsafe trait VmArch {
    /// 判断物理地址能否表示，不检查页对齐、实际内存或所有权。
    fn valid_phys(&self, pa: PhysAddr) -> bool;
    /// 判断虚拟地址能否表示，不检查页对齐或自检窗口。
    fn valid_virt(&self, va: VirtAddr) -> bool;
    /// 用户区间必须整页位于用户地址范围内，与每段保留区间不共用顶层槽位。
    fn valid_user_range(&self, user_range: Range<VirtAddr>, reserved: &[Range<VirtAddr>]) -> bool;
    /// 将内核恒等区覆盖的顶层项复制到空目的根，不复制其他槽位或改动下级表。
    ///
    /// # Safety
    /// 两根及下级表均已登记、由调用方持有。目的根未发布且独占；源分支保持只读，
    /// 所有借用它们的根退役前不得释放。操作串行，与硬件读取共享源表兼容。
    unsafe fn copy_kernel_root<T: TableAccess>(
        &mut self,
        tables: &mut T,
        dst_root: PhysAddr,
        src_root: PhysAddr,
        kernel_range: Range<VirtAddr>,
    ) -> Result<(), VmError>;
    /// 只读检查空缺路径，返回所需表页数 0–2；已有映射返回 AlreadyMapped。
    /// 非法地址返回 InvalidAddress，未知表页或非法表项返回 InvalidTable。
    fn needed_tables<T: TableAccess>(
        &self,
        tables: &T,
        root: PhysAddr,
        va: VirtAddr,
    ) -> Result<usize, VmError>;
    /// 建立一页映射，失败不改变旧树。spare 须已登记，且数量正确、清零、互异、未连接；
    /// 未登记返回 InvalidTable，其他 spare 条件不符返回 InvalidState。
    /// 非法地址、不对齐、权限不支持或重复映射分别返回对应 VmError。
    ///
    /// # Safety
    /// 修改路径和 spare 独占，表页存储有效，修改串行且无冲突引用；其他槽位可共享只读内核分支。
    /// 活动映射的数据页须按请求权限有效，不能破坏当前执行路径；未活动树在启用前保证。
    /// 调用方持有全部页面，活动修改返回后同步，再允许访问新别名或归还旧页。
    unsafe fn map_page<T: TableAccess>(
        &mut self,
        tables: &mut T,
        root: PhysAddr,
        va: VirtAddr,
        mapping: PageMapping,
        spare: &[PhysAddr],
    ) -> Result<(), VmError>;
    /// 只读查询，保留页内偏移；未映射为 None，非法地址或表项返回对应错误。
    fn query<T: TableAccess>(
        &self,
        tables: &T,
        root: PhysAddr,
        va: VirtAddr,
    ) -> Result<Option<Translation>, VmError>;
    /// 撤销一页并断开空的中间表，不释放任何页；错误保留旧树。
    /// 非法地址、不对齐、缺失映射及非法表项分别返回对应错误。
    ///
    /// # Safety
    /// 修改和回收的路径独占；其他槽位可共享只读内核分支。没有并发修改或冲突引用。
    /// 已结束目标别名访问，不能撤销当前执行所需映射；返回后同步才能释放 retired。
    unsafe fn unmap_page<T: TableAccess>(
        &mut self,
        tables: &mut T,
        root: PhysAddr,
        va: VirtAddr,
    ) -> Result<RemovedMapping, VmError>;
    /// 读回当前 CPU：无分页为 None；不支持的模式或地址空间编号为 InvalidState。
    fn current_root(&self) -> Result<Option<PhysAddr>, VmError>;
    /// 从无分页启用指定根并同步、读回确认；前置错误不覆盖现有根。
    /// 启用失败须恢复无分页后返回 ActivationFailed；不能确认恢复时终止运行。
    ///
    /// # Safety
    /// 单核关中断，已安装异常入口。调用方持有 root 及全部可达下级表页；
    /// 各表页须对齐、完整初始化、符合后端格式，映射的存储及权限满足实际访问。
    /// 根页须持有至确认停用且完成本核地址转换同步。
    /// 分页开启期间，仍可从根到达的下级表页保持有效；活动修改遵守发布与同步约定。
    /// 空子表从父表断开并完成本核同步后，即可回收或复用。
    /// 代码、栈、返回路径、只读数据、异常入口及其记录、日志路径所需内存，
    /// 均须在切换前后按所需权限可访问，包括切换后的状态输出和失败处理。
    unsafe fn activate(&mut self, root: PhysAddr) -> Result<(), VmError>;
    /// 从预期的当前根切至目标根，切换前后同步并读回新根；写入失败时确认恢复原根。
    ///
    /// # Safety
    /// 唯一启动核、关中断；两个根及可达表页均有效并持续持有。代码、栈、异常、
    /// 只读数据、返回与日志路径在两棵树中的权限和地址均有效，没有跨切换用户引用。
    unsafe fn switch_root(
        &mut self,
        expected_current: PhysAddr,
        next: PhysAddr,
    ) -> Result<(), VmError>;
    /// 当前根属于调用方时恢复无分页并同步；前置状态不符返回 InvalidState。
    /// 切换后无法确认无分页时终止运行，不允许后续回收。
    ///
    /// # Safety
    /// 当前根属于调用方；单核关中断，没有窗口访问或引用。
    /// 根及全部可达表页保持有效，直到确认无分页且同步完成。
    /// 代码、栈、返回路径、只读数据、异常记录和日志路径在无分页时仍可访问；
    /// 已安装的异常入口在切换前后均有效，失败处理不依赖已撤销的映射。
    unsafe fn deactivate(&mut self, root: PhysAddr) -> Result<(), VmError>;
    /// 完成本核全量转换同步及编译器访存约束，无可忽略的错误返回。
    ///
    /// # Safety
    /// 唯一启动核串行修改页表；页表写入完成，资源仍被持有。
    unsafe fn sync_translations(&mut self);
}

/// 只恢复这三种同步页异常。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultKind {
    /// 取指被页表拒绝。
    InstructionPage,
    /// 读取被页表拒绝。
    LoadPage,
    /// 写入被页表拒绝。
    StorePage,
}
/// 已观察到的同步页异常，保留实际指令与访问地址。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FaultInfo {
    /// 异常原因。
    pub kind: FaultKind,
    /// 触发异常的指令位置；取指异常时为目标窗口地址。
    pub instruction: VirtAddr,
    /// 导致异常的虚拟访问地址。
    pub address: VirtAddr,
}
/// 一次真实操作；即使意外成功也必须能安全返回。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeOp {
    /// 读取目标地址的一个字节。
    ReadByte { address: VirtAddr },
    /// 向专用测试页写一个字节。
    WriteByte { address: VirtAddr, value: u8 },
    /// 调用目标地址，要求目标包含架构提供的可返回代码。
    Execute { address: VirtAddr },
}
/// 由后端提供的指令和恢复标签，调用者不得按固定指令长度推算。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbeSite {
    /// 唯一允许匹配的故障指令位置。
    pub instruction: VirtAddr,
    /// 匹配后返回的明确标签，必须位于基础代码映射中。
    pub resume: VirtAddr,
}
/// 单次允许恢复的预期；原因、地址、指令必须全部一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpectedFault {
    /// 预期页异常原因。
    pub kind: FaultKind,
    /// 预期访问地址。
    pub address: VirtAddr,
    /// 与实际操作对应的后端标签。
    pub site: ProbeSite,
}
/// 安装或执行探针未满足约定；意外硬件异常直接终止运行，不经此类型恢复。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeError {
    /// 已安装入口，拒绝覆盖。
    AlreadyInstalled,
    /// 安装条件或入口读回检查失败。
    InstallFailed,
    /// 上一个预期或结果尚未消费。
    Busy,
    /// 操作、预期或标签不一致，尚未执行操作。
    InvalidProbe,
    /// 操作正常返回，没有捕获异常；保护用例必须判失败。
    NotObserved,
}

/// 受控用户拷贝的前置错误或已确认的部分完成结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserCopyError {
    /// 空间已经显式销毁。
    Destroyed,
    /// 当前根、异常入口或 CPU 访问状态不满足约定。
    InvalidState,
    /// 字节区间溢出或跨越非法地址。
    InvalidAddress,
    /// 地址可表示，但不在当前用户区间。
    OutsideUserRange,
    /// 还有尚未消费的探针或拷贝记录。
    Busy,
    /// 精确匹配到用户访存异常；前 copied 个字节已经完成。
    Fault {
        /// 由唯一用户访存指令触发的实际异常。
        fault: FaultInfo,
        /// 首个故障字节之前已完成的长度。
        copied: usize,
    },
}

/// 单核受控用户访存，与普通探针共用异常入口和单次匹配记录。
///
/// # Safety
/// 只对单条用户指令开放访问。故障须匹配原因、PC 和目标区间，所有出口恢复内核访问状态。
pub unsafe trait UserAccess {
    /// 验证常态下的用户页隔离，精确匹配单次异常。
    ///
    /// # Safety
    /// 同 FaultProbe::probe：关中断、目标为授权的测试页，意外成功也能安全返回。
    unsafe fn probe_user_isolation(
        &mut self,
        op: ProbeOp,
        expected: ExpectedFault,
    ) -> Result<FaultInfo, ProbeError>;
    /// 部分读取失败时清零内核缓冲区的未读部分；前置错误不修改缓冲区。
    ///
    /// # Safety
    /// 单核关中断，整个源区间已由活动空间校验，内核目的有效、无物理别名或冲突借用。
    unsafe fn copy_from_user(
        &mut self,
        user_addr: VirtAddr,
        dst: &mut [u8],
    ) -> Result<(), UserCopyError>;
    /// 部分写入失败时只修改已经完成的前缀。
    ///
    /// # Safety
    /// 单核关中断，整个目的区间已由活动空间校验，内核源有效且与用户页不别名。
    unsafe fn copy_to_user(&mut self, user_addr: VirtAddr, src: &[u8])
    -> Result<(), UserCopyError>;
}

/// 单次预期异常探针，不借用内存服务或地址空间。
///
/// # Safety
/// 实际执行指定操作，只接受原因/指令/地址完全匹配的一次异常；其他异常终止运行。
pub unsafe trait FaultProbe {
    /// 取得本次操作的指令和恢复标签，不执行目标操作。
    fn site(&self, op: &ProbeOp) -> ProbeSite;
    /// 参数检查后执行一次操作；返回匹配异常或 NotObserved。
    /// Busy/InvalidProbe 不执行操作，也不覆盖未完成的记录。
    ///
    /// # Safety
    /// 唯一启动核、关中断；目标为有效测试数据或已撤销窗口，无冲突引用。
    /// 取指意外成功时目标须能安全返回；栈、入口和恢复点不能依赖目标页。
    unsafe fn probe(
        &mut self,
        op: ProbeOp,
        expected: ExpectedFault,
    ) -> Result<FaultInfo, ProbeError>;
    /// 完成本核指令缓存和编译器同步，供执行新写入的探针代码前调用。
    ///
    /// # Safety
    /// 写入的探针指令已完成，唯一启动核没有并发修改它。
    unsafe fn sync_instructions(&mut self);
}
