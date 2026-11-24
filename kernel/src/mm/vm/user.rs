//! 用户根只读共享内核分支，用户表页随各自的空间切换和退役。

use super::space::{
    MappingTables, State, Tables, crosses_invalid_boundary, kernel_translation, validate_layout,
};
use super::*;
use crate::hal::{TableAccess, UserAccess, VmArch};
use core::mem::ManuallyDrop;

/// 不可变的内核基础映射和已校验的板级用户窗口。须显式销毁。
pub struct KernelBase<P: VmPage> {
    root: PhysAddr,
    tables: ManuallyDrop<Tables<P>>,
    base: Range<VirtAddr>,
    allowed: Range<VirtAddr>,
    state: State,
}

impl<P: VmPage> KernelBase<P> {
    /// 在申请页之前检查内核布局及板级传入的用户窗口；仅建立基础恒等映射。
    pub fn new<M: VmMemory<Page = P>, A: VmArch>(
        memory: &mut M,
        arch: &mut A,
        layout: KernelLayout,
        allowed: Range<VirtAddr>,
    ) -> Result<Self, VmError> {
        validate_layout(memory, arch, &layout)?;
        let base = VirtAddr(layout.code.start.0)..VirtAddr(layout.allocatable.end.0);
        if !arch.valid_user_range(allowed.clone(), &[base.clone(), layout.test_window.clone()]) {
            return Err(VmError::InvalidLayout);
        }
        let (root, tables) =
            KernelAddressSpace::from_validated(memory, arch, layout)?.take_tables();
        Ok(Self::from_tables(
            root,
            tables,
            base,
            allowed,
            State::Inactive,
        ))
    }
    pub(super) fn from_tables(
        root: PhysAddr,
        tables: ManuallyDrop<Tables<P>>,
        base: Range<VirtAddr>,
        allowed: Range<VirtAddr>,
        state: State,
    ) -> Self {
        Self {
            root,
            tables,
            base,
            allowed,
            state,
        }
    }
    /// 从无分页启用基础页表。
    ///
    /// # Safety
    /// 单核关中断，异常入口已安装；执行、栈、日志及返回路径跨切换有效。
    pub unsafe fn activate(&mut self, arch: &mut impl VmArch) -> Result<(), VmError> {
        if self.state == State::Destroyed {
            return Err(VmError::Destroyed);
        }
        if self.state == State::Active || arch.current_root()?.is_some() {
            return Err(VmError::InvalidState);
        }
        // SAFETY: 本对象持有完整基础树；调用者保证切换两侧所需内存有效。
        unsafe { arch.activate(self.root)? };
        self.state = State::Active;
        Ok(())
    }
    /// 从基础根返回无分页状态；存活的用户空间借用会阻止此操作。
    ///
    /// # Safety
    /// 单核关中断，执行、栈、异常、日志及返回路径在无分页时仍可达。
    pub unsafe fn deactivate(&mut self, arch: &mut impl VmArch) -> Result<(), VmError> {
        if self.state == State::Destroyed {
            return Err(VmError::Destroyed);
        }
        if self.state != State::Active || arch.current_root()? != Some(self.root) {
            return Err(VmError::InvalidState);
        }
        // SAFETY: 已核对唯一活动根，基础树仍持有至后端完成同步与读回。
        unsafe { arch.deactivate(self.root)? };
        self.state = State::Inactive;
        Ok(())
    }
    /// 基础页表根的物理页首。
    pub fn root(&self) -> PhysAddr {
        self.root
    }
    /// 内核恒等映射的完整虚拟区间。
    pub fn base_range(&self) -> Range<VirtAddr> {
        self.base.clone()
    }
    /// 板级允许创建用户映射的最大虚拟区间。
    pub fn allowed_user_range(&self) -> Range<VirtAddr> {
        self.allowed.clone()
    }
    /// 仅供开发机向未共享的基础表注入损坏项，验证查询和创建失败的回滚。
    #[cfg(all(test, target_pointer_width = "64"))]
    pub(crate) fn test_tree(&mut self) -> (PhysAddr, &mut impl TableAccess<Page = P>) {
        (self.root, &mut *self.tables)
    }
    /// 读取页内翻译，不允许内核基础叶项带用户权限。
    pub fn query(&self, arch: &impl VmArch, va: VirtAddr) -> Result<Option<Translation>, VmError> {
        if self.state == State::Destroyed {
            return Err(VmError::Destroyed);
        }
        kernel_translation(arch.query(&*self.tables, self.root, va)?)
    }
    /// 用原分配服务归还基础表页；销毁要求没有用户空间借用且 CPU 已关闭分页。
    pub fn destroy(
        &mut self,
        memory: &mut impl VmMemory<Page = P>,
        arch: &impl VmArch,
    ) -> Result<(), VmError> {
        if self.state == State::Destroyed {
            return Err(VmError::Destroyed);
        }
        // 即使活动的是另一个根，它也可能正在读共享的内核分支。
        if self.state == State::Active || arch.current_root()?.is_some() {
            return Err(VmError::ActiveAddressSpace);
        }
        self.tables.release_all(memory);
        self.state = State::Destroyed;
        Ok(())
    }
}

/// 一张根、一张中级表、最多三十张末级表；数据页不计入该集合。
const USER_TABLE_CAPACITY: usize = 32;
type UserTables<P> = Tables<P, USER_TABLE_CAPACITY>;

/// 从用户根可以读内核项，写操作只能取得用户空间自己持有的表页。
struct UserTableView<'k, P: VmPage> {
    owned: ManuallyDrop<UserTables<P>>,
    kernel: &'k KernelBase<P>,
}
impl<P: VmPage> TableAccess for UserTableView<'_, P> {
    type Page = P;
    fn table(&self, pa: PhysAddr) -> Result<&P, VmError> {
        self.owned
            .table(pa)
            .or_else(|_| self.kernel.tables.table(pa))
    }
    fn table_mut(&mut self, pa: PhysAddr) -> Result<&mut P, VmError> {
        self.owned.table_mut(pa)
    }
}
impl<P: VmPage> MappingTables<USER_TABLE_CAPACITY> for UserTableView<'_, P> {
    fn owned(&mut self) -> &mut UserTables<P> {
        &mut self.owned
    }
}

/// 共享借用内核基础映射，仅持有本空间的根和用户表页。不隐式释放页面。
pub struct UserAddressSpace<'k, P: VmPage> {
    user_range: Range<VirtAddr>,
    root: PhysAddr,
    tables: UserTableView<'k, P>,
    state: State,
}

impl<P: VmPage> Drop for UserAddressSpace<'_, P> {
    fn drop(&mut self) {
        // 保持对内核表的借用直至用户对象离开作用域；只允许 destroy 显式归还私有表页。
    }
}

impl<'k, P: VmPage> UserAddressSpace<'k, P> {
    /// 创建未启用的用户根；不合法的请求在调用分配器前返回 InvalidLayout。
    pub fn new<M: VmMemory<Page = P>, A: VmArch>(
        kernel: &'k KernelBase<P>,
        memory: &mut M,
        arch: &mut A,
        user_range: Range<VirtAddr>,
    ) -> Result<Self, VmError> {
        if kernel.state == State::Destroyed {
            return Err(VmError::Destroyed);
        }
        if user_range.start < kernel.allowed.start
            || user_range.end > kernel.allowed.end
            || !arch.valid_user_range(user_range.clone(), core::slice::from_ref(&kernel.base))
        {
            return Err(VmError::InvalidLayout);
        }
        let mut tables = UserTableView {
            owned: ManuallyDrop::new(UserTables::empty()),
            kernel,
        };
        let root = tables.owned.alloc(memory)?;
        // SAFETY: root 是未发布的新页；对内核表只读，kernel 的借用持续到本空间退役。
        if let Err(e) =
            unsafe { arch.copy_kernel_root(&mut tables, root, kernel.root, kernel.base_range()) }
        {
            tables.owned.release_all(memory);
            return Err(e);
        }
        Ok(Self {
            user_range,
            root,
            tables,
            state: State::Inactive,
        })
    }

    /// 查询已登记的用户与共享内核表页；叶项权限必须符合地址所属的域。
    pub fn query(&self, arch: &impl VmArch, va: VirtAddr) -> Result<Option<Translation>, VmError> {
        if self.state == State::Destroyed {
            return Err(VmError::Destroyed);
        }
        let entry = arch.query(&self.tables, self.root, va)?;
        if entry.is_some_and(|t| t.perm.user != self.user_range.contains(&va)) {
            return Err(VmError::InvalidTable);
        }
        Ok(entry)
    }

    /// 仅供开发机在未启用的独占测试树上独立读取表项、注入权限位错误。
    #[cfg(all(test, target_pointer_width = "64"))]
    pub(crate) fn test_tree(&mut self) -> (PhysAddr, &mut impl TableAccess<Page = P>) {
        (self.root, &mut self.tables)
    }

    /// 映射一张已授权的数据页，活动时同步转换；不接管数据页所有权。
    ///
    /// # Safety
    /// 调用者有权按指定权限共享该普通内存页。不得传入内核、表页、MMIO 或含其他用户数据的页；
    /// 页面须持有至映射退役，不得与存活引用冲突。使用原分配服务，单核关中断串行修改。
    pub unsafe fn map_page<M: VmMemory<Page = P>, A: VmArch>(
        &mut self,
        memory: &mut M,
        arch: &mut A,
        va: VirtAddr,
        pa: PhysAddr,
        perm: MapPerm,
    ) -> Result<(), VmError> {
        self.check_root(arch)?;
        if !arch.valid_virt(va) || !arch.valid_phys(pa) {
            return Err(VmError::InvalidAddress);
        }
        if !va.0.is_multiple_of(PAGE_SIZE) || !pa.0.is_multiple_of(PAGE_SIZE) {
            return Err(VmError::UnalignedAddress);
        }
        if !self.user_range.contains(&va) {
            return Err(VmError::ProtectedMapping);
        }
        if !perm.supported_user() {
            return Err(VmError::UnsupportedPermissions);
        }
        // SAFETY: 目标路径与内核槽位分离、由本空间独占；调用者持有并授权数据页。
        unsafe {
            self.tables.insert_page(
                memory,
                arch,
                self.root,
                va,
                PageMapping { phys: pa, perm },
                self.state == State::Active,
            )
        }
    }

    /// 本空间私有根的物理页首。
    pub fn root(&self) -> PhysAddr {
        self.root
    }

    fn check_root(&self, arch: &impl VmArch) -> Result<(), VmError> {
        if self.state == State::Destroyed {
            return Err(VmError::Destroyed);
        }
        if (self.state == State::Active) != (arch.current_root()? == Some(self.root)) {
            return Err(VmError::InvalidState);
        }
        Ok(())
    }
    /// 撤销一张用户页。活动树先同步，再归还本次断开的空表。
    ///
    /// # Safety
    /// 单核关中断；使用原分配服务，目标别名的访问与借用均已结束。
    pub unsafe fn unmap_page(
        &mut self,
        memory: &mut impl VmMemory<Page = P>,
        arch: &mut impl VmArch,
        va: VirtAddr,
    ) -> Result<PageMapping, VmError> {
        self.check_root(arch)?;
        if !arch.valid_virt(va) {
            return Err(VmError::InvalidAddress);
        }
        if !va.0.is_multiple_of(PAGE_SIZE) {
            return Err(VmError::UnalignedAddress);
        }
        if !self.user_range.contains(&va) {
            return Err(VmError::ProtectedMapping);
        }
        // SAFETY: 用户范围与内核分支隔离，目标路径私有；数据别名已停止使用。
        let removed = unsafe { arch.unmap_page(&mut self.tables, self.root, va)? };
        if self.state == State::Active {
            // SAFETY: 断开的表页尚未释放，唯一启动核完成旧转换失效。
            unsafe { arch.sync_translations() };
        }
        for pa in removed.retired.into_iter().flatten() {
            self.tables.owned.release(memory, pa);
        }
        Ok(removed.mapping)
    }
    /// 从共享内核根切入此用户根。
    ///
    /// # Safety
    /// 单核关中断，异常入口已安装；执行、栈、日志及返回路径均在共享基础映射内。
    pub unsafe fn activate(&mut self, arch: &mut impl VmArch) -> Result<(), VmError> {
        if self.state == State::Destroyed {
            return Err(VmError::Destroyed);
        }
        if self.state == State::Active || self.tables.kernel.state != State::Active {
            return Err(VmError::InvalidState);
        }
        // SAFETY: 两根均受本对象持有或借用；后端先核对当前根，再切换并同步。
        unsafe { arch.switch_root(self.tables.kernel.root, self.root)? };
        self.state = State::Active;
        Ok(())
    }
    /// 从当前用户根切回其内核根。
    ///
    /// # Safety
    /// 单核关中断，目标用户别名不再使用；执行、异常、日志及栈位于共享映射。
    pub unsafe fn deactivate(&mut self, arch: &mut impl VmArch) -> Result<(), VmError> {
        if self.state == State::Destroyed {
            return Err(VmError::Destroyed);
        }
        if self.state != State::Active {
            return Err(VmError::InvalidState);
        }
        // SAFETY: 内核借用仍有效，两棵树持有至切换读回完成。
        unsafe { arch.switch_root(self.root, self.tables.kernel.root)? };
        self.state = State::Inactive;
        Ok(())
    }
    /// 同一内核模板下，从当前空间直接切换到另一个空间。
    ///
    /// # Safety
    /// 单核关中断，无仍在访问的旧用户别名；执行、栈、异常和日志路径由两根共享。
    pub unsafe fn switch_to(
        &mut self,
        next: &mut Self,
        arch: &mut impl VmArch,
    ) -> Result<(), VmError> {
        if self.state == State::Destroyed || next.state == State::Destroyed {
            return Err(VmError::Destroyed);
        }
        if self.state != State::Active
            || next.state == State::Active
            || !core::ptr::eq(self.tables.kernel, next.tables.kernel)
        {
            return Err(VmError::InvalidState);
        }
        // SAFETY: 已核实两根共享基础映射，两套私有表持续持有且串行切换。
        unsafe { arch.switch_root(self.root, next.root)? };
        self.state = State::Inactive;
        next.state = State::Active;
        Ok(())
    }

    pub(super) fn validate_copy(
        &self,
        arch: &impl VmArch,
        addr: VirtAddr,
        len: usize,
    ) -> Result<(), UserCopyError> {
        if self.state == State::Destroyed {
            return Err(UserCopyError::Destroyed);
        }
        if self.state != State::Active || arch.current_root() != Ok(Some(self.root)) {
            return Err(UserCopyError::InvalidState);
        }
        let end = addr
            .0
            .checked_add(len)
            .ok_or(UserCopyError::InvalidAddress)?;
        if !arch.valid_virt(addr) || (len != 0 && !arch.valid_virt(VirtAddr(end - 1))) {
            return Err(UserCopyError::InvalidAddress);
        }
        if !self.user_range.contains(&addr) || end > self.user_range.end.0 {
            // 两端都可能合法但跨过规范地址空洞。此路径只在范围检查失败时执行。
            if crosses_invalid_boundary(addr.0, end, |v| arch.valid_virt(VirtAddr(v))) {
                return Err(UserCopyError::InvalidAddress);
            }
            return Err(UserCopyError::OutsideUserRange);
        }
        Ok(())
    }
    /// 从活动用户空间读取，部分失败时保留已读前缀并清零剩余缓冲区。
    ///
    /// # Safety
    /// 单核关中断，用户映射稳定；内核缓冲区有效，与目标物理页及其他活动借用无别名。
    pub unsafe fn copy_from_user(
        &self,
        arch: &impl VmArch,
        access: &mut impl UserAccess,
        addr: VirtAddr,
        dst: &mut [u8],
    ) -> Result<(), UserCopyError> {
        self.validate_copy(arch, addr, dst.len())?;
        if dst.is_empty() {
            return Ok(());
        }
        // SAFETY: 整个区间已校验，调用者保证缓冲区独占且执行上下文有效。
        unsafe { access.copy_from_user(addr, dst) }
    }
    /// 向活动用户空间写入，返回故障前已经写入的字节数。
    ///
    /// # Safety
    /// 单核关中断，用户映射稳定；内核源缓冲区有效且不与任何目标字节别名。
    pub unsafe fn copy_to_user(
        &self,
        arch: &impl VmArch,
        access: &mut impl UserAccess,
        addr: VirtAddr,
        src: &[u8],
    ) -> Result<(), UserCopyError> {
        self.validate_copy(arch, addr, src.len())?;
        if src.is_empty() {
            return Ok(());
        }
        // SAFETY: 完整用户范围已校验，源与目标彼此独立。
        unsafe { access.copy_to_user(addr, src) }
    }

    /// 用原分配服务归还未活动空间的私有表页，不触碰共享内核表页和数据页。
    pub fn destroy(
        &mut self,
        memory: &mut impl VmMemory<Page = P>,
        arch: &impl VmArch,
    ) -> Result<(), VmError> {
        if self.state == State::Destroyed {
            return Err(VmError::Destroyed);
        }
        if self.state == State::Active || arch.current_root()? == Some(self.root) {
            return Err(VmError::ActiveAddressSpace);
        }
        self.tables.owned.release_all(memory);
        self.state = State::Destroyed;
        Ok(())
    }
}
