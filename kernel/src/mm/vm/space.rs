//! 页表页由公共层持有；先取得资源再发布，先同步再回收。
use super::*;
use crate::hal::{TableAccess, VmArch};
use crate::mm::PAGE_SIZE;
use core::mem::ManuallyDrop;

/// 无堆启动阶段的表页句柄容量，足够覆盖当前 128 MiB 平台及窗口。
const TABLE_CAPACITY: usize = 128;

/// 只登记此地址空间拥有的表页；数据页句柄不会进入集合。
pub(crate) struct Tables<P: VmPage, const N: usize = TABLE_CAPACITY> {
    pages: [Option<P>; N],
}

/// 内核表和用户视图共用申请、发布、回滚顺序；新表始终登记在接收方的私有集合中。
pub(super) trait MappingTables<const N: usize>: TableAccess + Sized {
    fn owned(&mut self) -> &mut Tables<Self::Page, N>;

    /// # Safety
    /// 目标路径和新页由调用方独占。目标物理页须满足映射权限；活动时单核关中断，
    /// 修改不破坏执行、异常或返回路径。后端失败时保留旧树，新增页尚未发布。
    unsafe fn insert_page(
        &mut self,
        memory: &mut impl VmMemory<Page = Self::Page>,
        arch: &mut impl VmArch,
        root: PhysAddr,
        va: VirtAddr,
        mapping: PageMapping,
        active: bool,
    ) -> Result<(), VmError> {
        let count = arch.needed_tables(self, root, va)?;
        let mut spare = [PhysAddr(0); 2];
        let slots = spare.get_mut(..count).ok_or(VmError::InvalidTable)?;
        let mut acquired = 0;
        let result = (|| {
            for slot in &mut *slots {
                *slot = self.owned().alloc(memory)?;
                acquired += 1;
            }
            // SAFETY: 目标路径独占，所需的新表均已清零、互异并登记；共享分支只读。
            unsafe { arch.map_page(self, root, va, mapping, slots) }
        })();
        if active {
            // SAFETY: 修改已结束，全部页仍持有；本核关中断，回收前必须完成同步。
            unsafe { arch.sync_translations() };
        }
        if result.is_err() {
            for &pa in &spare[..acquired] {
                self.owned().release(memory, pa);
            }
        }
        result
    }
}

impl<P: VmPage, const N: usize> MappingTables<N> for Tables<P, N> {
    fn owned(&mut self) -> &mut Self {
        self
    }
}
impl<P: VmPage, const N: usize> TableAccess for Tables<P, N> {
    type Page = P;
    fn table(&self, pa: PhysAddr) -> Result<&P, VmError> {
        self.pages
            .iter()
            .flatten()
            .find(|p| p.phys_addr() == pa)
            .ok_or(VmError::InvalidTable)
    }
    fn table_mut(&mut self, pa: PhysAddr) -> Result<&mut P, VmError> {
        self.pages
            .iter_mut()
            .flatten()
            .find(|p| p.phys_addr() == pa)
            .ok_or(VmError::InvalidTable)
    }
}

impl<P: VmPage, const N: usize> Tables<P, N> {
    /// 空登记表本身不申请物理页。
    pub(super) fn empty() -> Self {
        Self {
            pages: core::array::from_fn(|_| None),
        }
    }
    /// 先取得句柄槽位，再从同一资源服务申请一张清零的页。
    pub(super) fn alloc(
        &mut self,
        memory: &mut impl VmMemory<Page = P>,
    ) -> Result<PhysAddr, VmError> {
        let slot = self
            .pages
            .iter_mut()
            .find(|p| p.is_none())
            .ok_or(PageAllocError::OutOfMemory)?;
        let page = memory.alloc_zeroed_page()?;
        let pa = page.phys_addr();
        *slot = Some(page);
        Ok(pa)
    }
    /// 仅归还未发布或断开并同步过的独占表页；归还失败时保留原句柄。
    pub(super) fn release(&mut self, memory: &mut impl VmMemory<Page = P>, pa: PhysAddr) {
        let slot = self
            .pages
            .iter_mut()
            .find(|p| p.as_ref().is_some_and(|p| p.phys_addr() == pa))
            .expect("owned table missing");
        let page = slot.take().expect("owned table missing");
        // SAFETY: 调用点已保证表页不再被地址翻译或别名访问。
        if let Err((error, page)) = unsafe { memory.release_page(page) } {
            *slot = Some(page);
            panic!("VM table ownership failure: {error:?}");
        }
    }
    /// 只用于构造回滚或已确认不活动的地址空间。
    pub(super) fn release_all(&mut self, memory: &mut impl VmMemory<Page = P>) {
        for index in 0..N {
            if let Some(page) = &self.pages[index] {
                self.release(memory, page.phys_addr());
            }
        }
    }
}

/// 公共层可复用叶项编解码，但查询内核地址空间时绝不接受用户权限。
pub(super) fn kernel_translation(
    entry: Option<Translation>,
) -> Result<Option<Translation>, VmError> {
    if entry.is_some_and(|t| t.perm.user) {
        Err(VmError::InvalidTable)
    } else {
        Ok(entry)
    }
}
/// 未启用和已停用共用 Inactive；重复停用都返回 InvalidState。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum State {
    Inactive,
    Active,
    Destroyed,
}

/// 独占资源服务和架构后端；显式 destroy 回收，Drop 不停用或释放表页。
pub struct KernelAddressSpace<'a, M: VmMemory, A: VmArch> {
    memory: &'a mut M,
    arch: &'a mut A,
    // 防止泛型页句柄的析构使仍被硬件使用的存储失效。
    pub(crate) tables: ManuallyDrop<Tables<M::Page>>,
    pub(crate) root: PhysAddr,
    base: Range<VirtAddr>,
    window: Range<VirtAddr>,
    state: State,
}

impl<M: VmMemory, A: VmArch> Drop for KernelAddressSpace<'_, M, A> {
    fn drop(&mut self) {
        // 不隐式操作 CPU 或归还页。ManuallyDrop 保留未显式销毁的表存储；
        // Drop 同时使服务借用持续到对象真正结束，destroy 不能单独结束借用。
    }
}

impl<'a, M: VmMemory, A: VmArch> KernelAddressSpace<'a, M, A> {
    /// 校验全部布局，建立基础恒等映射；失败归还本次取得的所有表页。
    /// InvalidLayout 必须先于任何申请；表页或句柄容量不足保留 OutOfMemory。
    pub fn new(memory: &'a mut M, arch: &'a mut A, layout: KernelLayout) -> Result<Self, VmError> {
        validate_layout(memory, arch, &layout)?;
        Self::from_validated(memory, arch, layout)
    }

    /// 共享基础映射的构造路径；调用点已对同一份 layout 完成全部布局校验。
    pub(super) fn from_validated(
        memory: &'a mut M,
        arch: &'a mut A,
        layout: KernelLayout,
    ) -> Result<Self, VmError> {
        let mut space = Self {
            memory,
            arch,
            tables: ManuallyDrop::new(Tables::empty()),
            root: PhysAddr(0),
            base: VirtAddr(layout.code.start.0)..VirtAddr(layout.allocatable.end.0),
            window: layout.test_window,
            state: State::Inactive,
        };
        let result = (|| {
            space.root = space.tables.alloc(space.memory)?;
            for (range, perm) in [
                (layout.code, MapPerm::RX),
                (layout.rodata, MapPerm::R),
                (layout.writable, MapPerm::RW),
                (layout.allocatable, MapPerm::RW),
            ] {
                for pa in (range.start.0..range.end.0).step_by(PAGE_SIZE) {
                    space.insert(VirtAddr(pa), PhysAddr(pa), perm)?;
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            space.tables.release_all(space.memory);
            return Err(error);
        }
        Ok(space)
    }

    /// 建立窗口映射，不接管数据页。
    /// 按状态、地址表示、对齐、窗口、权限、重复映射、资源申请的顺序报错。
    /// 失败保留旧映射；活动时修改及回滚都先同步，再归还本次新表页。
    ///
    /// # Safety
    /// 参数通过校验时，调用者须持有目标普通内存页至撤销并同步；无冲突引用，
    /// 不给代码/只读段添加可写别名，
    /// 不将表页作数据映射。唯一启动核串行执行，活动时保持关中断。
    pub unsafe fn map_page(
        &mut self,
        va: VirtAddr,
        pa: PhysAddr,
        perm: MapPerm,
    ) -> Result<(), VmError> {
        self.live()?;
        if !self.arch.valid_virt(va) || !self.arch.valid_phys(pa) {
            return Err(VmError::InvalidAddress);
        }
        if !va.0.is_multiple_of(PAGE_SIZE) || !pa.0.is_multiple_of(PAGE_SIZE) {
            return Err(VmError::UnalignedAddress);
        }
        self.in_window(va)?;
        if !perm.supported() {
            return Err(VmError::UnsupportedPermissions);
        }
        self.insert(va, pa, perm)
    }

    /// 查询字节地址，允许页内偏移和窗口外查询。
    /// 未映射返回 None；非法地址、未知表页和已销毁对象分别返回对应错误。
    pub fn query(&self, va: VirtAddr) -> Result<Option<Translation>, VmError> {
        self.live()?;
        if !self.arch.valid_virt(va) {
            return Err(VmError::InvalidAddress);
        }
        kernel_translation(self.arch.query(&*self.tables, self.root, va)?)
    }

    /// 撤销窗口映射；同步后归还断开的空表，不释放数据页。
    /// 窗口外返回 ProtectedMapping，窗口内缺失映射返回 NotMapped。
    ///
    /// # Safety
    /// 目标别名的全部访问和引用已结束；唯一启动核串行执行，活动时关中断。
    pub unsafe fn unmap_page(&mut self, va: VirtAddr) -> Result<PageMapping, VmError> {
        self.live()?;
        if !self.arch.valid_virt(va) {
            return Err(VmError::InvalidAddress);
        }
        if !va.0.is_multiple_of(PAGE_SIZE) {
            return Err(VmError::UnalignedAddress);
        }
        self.in_window(va)?;
        // SAFETY: 所属表仍有效，调用者结束了目标访问，后端只断开、不释放。
        let removed = unsafe { self.arch.unmap_page(&mut *self.tables, self.root, va)? };
        self.sync_if_active();
        for pa in removed.retired.into_iter().flatten() {
            self.tables.release(self.memory, pa);
        }
        Ok(removed.mapping)
    }

    /// 启用后仍保留对服务的独占借用。
    ///
    /// # Safety
    /// 唯一启动核、关中断且已装异常入口；代码、栈、表页及返回路径跨切换有效。
    /// 只读数据、异常入口及记录、日志和失败处理所需内存均按所需权限可访问。
    pub unsafe fn activate(&mut self) -> Result<(), VmError> {
        self.live()?;
        if self.state == State::Active || self.arch.current_root()?.is_some() {
            return Err(VmError::InvalidState);
        }
        // SAFETY: 基础布局已完整建立，本对象持有根及全部可达下级表页。
        // 根持有至确认停用并完成本核同步；空子表断开父项并完成本核同步后才回收。
        // 调用者保证执行、异常和日志路径在切换前后有效。
        unsafe {
            self.arch.activate(self.root)?;
        }
        match self.arch.current_root() {
            Ok(Some(root)) if root == self.root => {}
            observed => panic!("VM backend did not confirm active root: {observed:?}"),
        }
        self.state = State::Active;
        Ok(())
    }

    /// 停用并读回确认后才能销毁。
    ///
    /// # Safety
    /// 唯一启动核、关中断，无窗口引用或执行；代码、栈、返回路径、只读数据、
    /// 已安装的异常入口及记录、日志和失败处理所需内存在无分页时仍有效。
    pub unsafe fn deactivate(&mut self) -> Result<(), VmError> {
        self.live()?;
        if self.state != State::Active || self.arch.current_root()? != Some(self.root) {
            return Err(VmError::InvalidState);
        }
        // SAFETY: 已核对当前根，调用者保证跨切换路径有效。
        unsafe {
            self.arch.deactivate(self.root)?;
        }
        match self.arch.current_root() {
            Ok(None) => {}
            observed => panic!("VM backend did not confirm Bare mode: {observed:?}"),
        }
        self.state = State::Inactive;
        Ok(())
    }

    /// 只销毁未活动的表页；对象仍保持服务借用，直到离开作用域。
    pub fn destroy(&mut self) -> Result<(), VmError> {
        self.live()?;
        if self.state == State::Active || self.arch.current_root()? == Some(self.root) {
            return Err(VmError::ActiveAddressSpace);
        }
        self.tables.release_all(self.memory);
        self.state = State::Destroyed;
        Ok(())
    }

    /// 公共层先申请全部页，再交架构发布；失败时仅撤回本次资源。
    fn insert(&mut self, va: VirtAddr, pa: PhysAddr, perm: MapPerm) -> Result<(), VmError> {
        // SAFETY: 构造只建立未启用的基础映射；窗口调用方保证页面有效、活动时关中断。
        unsafe {
            self.tables.insert_page(
                self.memory,
                self.arch,
                self.root,
                va,
                PageMapping { phys: pa, perm },
                self.state == State::Active,
            )
        }
    }
    /// 所有公共操作先拒绝已销毁对象，不再访问表页或 CPU。
    fn live(&self) -> Result<(), VmError> {
        if self.state == State::Destroyed {
            Err(VmError::Destroyed)
        } else {
            Ok(())
        }
    }
    /// 基础恒等区不能经普通映射入口修改。
    fn in_window(&self, va: VirtAddr) -> Result<(), VmError> {
        if self.window.contains(&va) {
            Ok(())
        } else {
            Err(VmError::ProtectedMapping)
        }
    }
    /// 非活动修改由 activate 统一同步，活动修改在回收前完成本核同步。
    fn sync_if_active(&mut self) {
        if self.state == State::Active {
            // SAFETY: 修改已完成，表页仍持有，调用者保证单核关中断。
            unsafe {
                self.arch.sync_translations();
            }
        }
    }
    /// 转交已启用的基础页表。失败交回原对象，调用者仍可撤销窗口或修正区间。
    #[allow(clippy::result_large_err)] // 无堆；错误需完整交回原表页与服务借用。
    pub fn into_kernel_base(
        self,
        allowed: Range<VirtAddr>,
    ) -> Result<KernelBase<M::Page>, (VmError, Self)> {
        let check = (|| {
            self.live()?;
            if self.state != State::Active || self.arch.current_root() != Ok(Some(self.root)) {
                return Err(VmError::InvalidState);
            }
            for addr in (self.window.start.0..self.window.end.0).step_by(PAGE_SIZE) {
                if self.query(VirtAddr(addr)) != Ok(None) {
                    return Err(VmError::InvalidState);
                }
            }
            if !self
                .arch
                .valid_user_range(allowed.clone(), &[self.base.clone(), self.window.clone()])
            {
                return Err(VmError::InvalidLayout);
            }
            Ok(())
        })();
        if let Err(error) = check {
            return Err((error, self));
        }
        let base = self.base.clone();
        let (root, tables) = self.take_tables();
        Ok(KernelBase::from_tables(
            root,
            tables,
            base,
            allowed,
            State::Active,
        ))
    }

    /// 转移已验证的全部页句柄，原容器留下空表。
    pub(super) fn take_tables(mut self) -> (PhysAddr, ManuallyDrop<Tables<M::Page>>) {
        (
            self.root,
            core::mem::replace(&mut self.tables, ManuallyDrop::new(Tables::empty())),
        )
    }
}

/// 先检查每段和连续性，再检查资源范围；不触碰任何页面。
pub(super) fn validate_layout<M: VmMemory, A: VmArch>(
    memory: &M,
    arch: &A,
    layout: &KernelLayout,
) -> Result<(), VmError> {
    let ranges = [
        &layout.code,
        &layout.rodata,
        &layout.writable,
        &layout.allocatable,
    ];
    for range in ranges {
        valid_range(range.start.0, range.end.0, |value| {
            arch.valid_phys(PhysAddr(value)) && arch.valid_virt(VirtAddr(value))
        })?;
    }
    valid_range(
        layout.test_window.start.0,
        layout.test_window.end.0,
        |value| arch.valid_virt(VirtAddr(value)),
    )?;
    if layout.code.end != layout.rodata.start
        || layout.rodata.end != layout.writable.start
        || layout.writable.end != layout.allocatable.start
        || layout.allocatable != memory.managed_range()
        || (layout.test_window.start.0 < layout.allocatable.end.0
            && layout.test_window.end.0 > layout.code.start.0)
    {
        return Err(VmError::InvalidLayout);
    }
    Ok(())
}
/// 检查端点及穿越的二进制边界，拒绝跨规范地址空洞的区间，不逐页扫描巨大非法范围。
fn valid_range(start: usize, end: usize, valid: impl Fn(usize) -> bool) -> Result<(), VmError> {
    if start >= end
        || !start.is_multiple_of(PAGE_SIZE)
        || !end.is_multiple_of(PAGE_SIZE)
        || !valid(start)
        || !valid(end - 1)
        || crosses_invalid_boundary(start, end, valid)
    {
        return Err(VmError::InvalidLayout);
    }
    Ok(())
}

/// 即使两端可表示，两个规范地址区之间也可能存在不能访问的二进制边界。
pub(super) fn crosses_invalid_boundary(
    start: usize,
    end: usize,
    valid: impl Fn(usize) -> bool,
) -> bool {
    (0..usize::BITS)
        .map(|bit| 1usize << bit)
        .any(|boundary| start < boundary && boundary < end && !valid(boundary))
}
