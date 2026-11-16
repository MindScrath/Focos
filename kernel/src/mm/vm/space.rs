//! 页表页由公共层持有；先取得资源再发布，先同步再回收。
use super::*;
use crate::hal::{TableAccess, VmArch};
use crate::mm::PAGE_SIZE;
use core::mem::ManuallyDrop;

/// 无堆启动阶段的表页句柄容量，足够覆盖当前 128 MiB 平台及窗口。
const TABLE_CAPACITY: usize = 128;

/// 只登记此地址空间拥有的表页；数据页句柄不会进入集合。
pub(crate) struct Tables<P: VmPage> {
    pages: [Option<P>; TABLE_CAPACITY],
}
impl<P: VmPage> TableAccess for Tables<P> {
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
/// 未启用和已停用共用 Inactive；重复停用都返回 InvalidState。
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
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
        let mut space = Self {
            memory,
            arch,
            tables: ManuallyDrop::new(Tables {
                pages: core::array::from_fn(|_| None),
            }),
            root: PhysAddr(0),
            window: layout.test_window,
            state: State::Inactive,
        };
        let result = (|| {
            space.root = space.alloc_table()?;
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
            space.release_all();
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
        self.arch.query(&*self.tables, self.root, va)
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
            self.release_table(pa);
        }
        Ok(removed.mapping)
    }

    /// 启用后仍保留对服务的独占借用。
    ///
    /// # Safety
    /// 唯一启动核、关中断且已装异常入口；代码、栈、表页及返回路径跨切换有效。
    pub unsafe fn activate(&mut self) -> Result<(), VmError> {
        self.live()?;
        if self.state == State::Active || self.arch.current_root()?.is_some() {
            return Err(VmError::InvalidState);
        }
        // SAFETY: 调用者满足切换前提，基础布局已经完整建立。
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
    /// 同 activate；无窗口引用或执行，恒等代码和栈在无分页时有效。
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
        self.release_all();
        self.state = State::Destroyed;
        Ok(())
    }

    /// 公共层先申请全部页，再交架构发布；失败时仅撤回本次资源。
    fn insert(&mut self, va: VirtAddr, pa: PhysAddr, perm: MapPerm) -> Result<(), VmError> {
        let count = self.arch.needed_tables(&*self.tables, self.root, va)?;
        if count > 2 {
            return Err(VmError::InvalidTable);
        }
        let mut spare = [PhysAddr(0); 2];
        let mut acquired = 0;
        let result = (|| {
            for slot in spare.iter_mut().take(count) {
                *slot = self.alloc_table()?;
                acquired += 1;
            }
            // SAFETY: 全部新页已清零并登记，地址不重复，操作期间独占表页。
            unsafe {
                self.arch.map_page(
                    &mut *self.tables,
                    self.root,
                    va,
                    PageMapping { phys: pa, perm },
                    &spare[..count],
                )
            }
        })();
        self.sync_if_active();
        if result.is_err() {
            for pa in spare[..acquired].iter().copied() {
                self.release_table(pa);
            }
        }
        result
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
    /// 先找空的句柄槽位，再从同一资源服务取得已清零表页。
    fn alloc_table(&mut self) -> Result<PhysAddr, VmError> {
        let slot = self
            .tables
            .pages
            .iter_mut()
            .find(|p| p.is_none())
            .ok_or(PageAllocError::OutOfMemory)?;
        let page = self.memory.alloc_zeroed_page()?;
        let pa = page.phys_addr();
        *slot = Some(page);
        Ok(pa)
    }
    /// 调用点已保证表页不再可用；归还失败保留句柄并终止，不丢弃所有权故障。
    fn release_table(&mut self, pa: PhysAddr) {
        let slot = self
            .tables
            .pages
            .iter_mut()
            .find(|p| p.as_ref().is_some_and(|p| p.phys_addr() == pa))
            .expect("owned table missing");
        let page = slot.take().expect("owned table missing");
        // SAFETY: 未发布、已断开并同步，或整个地址空间已确认停用。
        if let Err((error, page)) = unsafe { self.memory.release_page(page) } {
            *slot = Some(page);
            panic!("VM table ownership failure: {error:?}");
        }
    }
    /// 仅用于构造回滚或已确认不活动的空间，归还全部登记表页。
    fn release_all(&mut self) {
        for index in 0..TABLE_CAPACITY {
            if let Some(page) = &self.tables.pages[index] {
                self.release_table(page.phys_addr());
            }
        }
    }
}

/// 先检查每段和连续性，再检查资源范围；不触碰任何页面。
fn validate_layout<M: VmMemory, A: VmArch>(
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
    {
        return Err(VmError::InvalidLayout);
    }
    for bit in 0..usize::BITS {
        let boundary = 1usize << bit;
        if boundary > start && boundary < end && !valid(boundary) {
            return Err(VmError::InvalidLayout);
        }
    }
    Ok(())
}
