//! 公共地址、页权限、资源和生命周期接口；不解释硬件页表项。

use super::{PAGE_SIZE, PageAllocError};
use core::ops::Range;

mod space;
pub use space::KernelAddressSpace;
mod user;
pub use crate::hal::UserCopyError;
pub use user::{KernelBase, UserAddressSpace};
mod memory;
pub use memory::{IdentityMemory, IdentityPage};

/// 物理字节地址；构造保留原值，合法性由架构后端判断。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PhysAddr(usize);
impl PhysAddr {
    /// 保存未截断、未校验的地址。
    pub const fn from_raw(value: usize) -> Self {
        Self(value)
    }
    /// 读取原始字节地址。
    pub const fn as_usize(self) -> usize {
        self.0
    }
}

/// 虚拟字节地址，不与物理地址隐式互换。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct VirtAddr(usize);
impl VirtAddr {
    /// 保存未截断、未校验的地址。
    pub const fn from_raw(value: usize) -> Self {
        Self(value)
    }
    /// 读取原始字节地址。
    pub const fn as_usize(self) -> usize {
        self.0
    }
}

/// 页映射的权限；内核页和用户页分别接受 R、RW、RX。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapPerm {
    /// 允许读取；本阶段所有合法组合均为 true。
    pub read: bool,
    /// 允许写入；不能与 execute 同时为 true。
    pub write: bool,
    /// 允许取指；不能与 write 同时为 true。
    pub execute: bool,
    /// 叶项属于用户域；内核地址空间不接受此位。
    pub user: bool,
}
impl MapPerm {
    /// 内核只读、不可执行。
    pub const R: Self = Self {
        read: true,
        write: false,
        execute: false,
        user: false,
    };
    /// 内核读写、不可执行。
    pub const RW: Self = Self {
        write: true,
        ..Self::R
    };
    /// 内核读取和执行、不可写。
    pub const RX: Self = Self {
        execute: true,
        ..Self::R
    };
    /// 用户只读、不可执行。
    pub const USER_R: Self = Self {
        user: true,
        ..Self::R
    };
    /// 用户读写、不可执行。
    pub const USER_RW: Self = Self {
        user: true,
        ..Self::RW
    };
    /// 用户读取和执行、不可写。
    pub const USER_RX: Self = Self {
        user: true,
        ..Self::RX
    };
    /// 两个权限域都不支持仅写、仅执行或可写可执行的叶项。
    pub fn supported_leaf(self) -> bool {
        self.read && !(self.write && self.execute)
    }
    /// 内核映射入口只接受内核叶项。
    pub fn supported_kernel(self) -> bool {
        !self.user && self.supported_leaf()
    }
    /// 用户映射入口只接受用户叶项。
    pub fn supported_user(self) -> bool {
        self.user && self.supported_leaf()
    }
    /// 不接受用户页、仅写/仅执行或可写可执行页。
    pub fn supported(self) -> bool {
        self.supported_kernel()
    }
}

/// 撤销前的页首与权限；不转移数据页所有权。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageMapping {
    /// 原映射的物理页首，不代表页面所有权。
    pub phys: PhysAddr,
    /// 原映射权限。
    pub perm: MapPerm,
}
/// 查询得到的物理字节地址（包含页内偏移）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Translation {
    /// 对应查询位置的物理字节地址，包含页内偏移。
    pub phys: PhysAddr,
    /// 被查询页面的权限。
    pub perm: MapPerm,
}

/// 可构造的启动布局；创建地址空间时先校验，再申请页。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelLayout {
    /// RX 恒等区，覆盖代码与该段的页对齐填充。
    pub code: Range<PhysAddr>,
    /// R 恒等区，紧邻代码区。
    pub rodata: Range<PhysAddr>,
    /// RW 恒等区，含数据、BSS、启动栈及静态元数据。
    pub writable: Range<PhysAddr>,
    /// RW 恒等区；本阶段必须与 VmMemory 的完整管理范围相等。
    pub allocatable: Range<PhysAddr>,
    /// 可临时修改的虚拟窗口，与全部基础映射分离。
    pub test_window: Range<VirtAddr>,
}

/// 分页入口的可恢复错误；所有权损坏或无法确认 CPU 状态时直接终止启动。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmError {
    /// 对象已经显式销毁，所有入口均拒绝使用。
    Destroyed,
    /// 重复或次序错误的启停，或 CPU 状态与操作前提不符。
    InvalidState,
    /// 布局为空、不对齐、不连续、地址不合法或不匹配资源范围。
    InvalidLayout,
    /// 地址不能由当前架构表示；优先于对齐错误。
    InvalidAddress,
    /// 页级操作的地址未按 PAGE_SIZE 对齐。
    UnalignedAddress,
    /// 修改目标在自检窗口外。
    ProtectedMapping,
    /// 叶项权限组合或所属权限域不受此入口支持。
    UnsupportedPermissions,
    /// 目标虚拟页已存在映射，禁止覆盖。
    AlreadyMapped,
    /// 撤销目标没有映射。
    NotMapped,
    /// 保留底层分配错误；句柄容量耗尽也使用 OutOfMemory。
    Allocator(PageAllocError),
    /// 不能销毁仍被 CPU 使用的地址空间。
    ActiveAddressSpace,
    /// CPU 不支持请求或读回不匹配，已经确认恢复 Bare。
    ActivationFailed,
    /// 页表引用未登记页，或项格式不符合本阶段约定。
    InvalidTable,
}
impl From<PageAllocError> for VmError {
    fn from(error: PageAllocError) -> Self {
        Self::Allocator(error)
    }
}

/// 独占、无自动归还的页句柄。闭包访问不得把指针留作后续使用。
///
/// # Safety
/// 物理页首与闭包得到的缓冲区都须按 PAGE_SIZE 对齐，完整初始化且持有期内有效。
/// 句柄不可伪造、复制或克隆；内容不借用资源服务的元数据。
/// 物理地址稳定，整页借用必须与全部虚拟别名及硬件访问分时进行。
/// word 入口只访问一个对齐的 64 位单元，不得形成页内容的 Rust 引用；
/// 越界索引在访存前拒绝。硬件共享表页只能使用 word 入口，含义由 arch 解释。
/// 句柄销毁不隐式归还分配标记。
pub unsafe trait VmPage {
    /// 返回稳定的物理页首；开发机数值地址不等于缓冲区指针。
    fn phys_addr(&self) -> PhysAddr;
    /// 在闭包期间借用整页；引用不能逃逸，也不得留下供后续访问的裸指针。
    fn with_bytes<R>(&self, f: impl FnOnce(&[u8; PAGE_SIZE]) -> R) -> R;
    /// 独占借用整页；期间不能通过其他物理路径或虚拟别名访问页面。
    fn with_bytes_mut<R>(&mut self, f: impl FnOnce(&mut [u8; PAGE_SIZE]) -> R) -> R;
    /// 按完整 64 位 volatile 读取一个存储单元；slot 越界时 panic。
    fn read_word(&self, slot: usize) -> u64;
    /// 按完整 64 位 volatile 写入一个存储单元；slot 越界时 panic。
    /// 硬件共享页只允许硬件读取；调用方负责发布顺序和修改后的地址转换同步。
    fn write_word(&mut self, slot: usize, value: u64);
}

/// 复用物理页分配器的资源服务，分配的句柄不借用服务。
///
/// # Safety
/// 管理范围稳定，所有分出的页均完整位于范围内，持有页彼此独立。
/// 申请失败不占页、不修改旧页，成功返回已清零且不借用服务元数据的句柄。
/// 服务销毁不得使仍持有的页失效。归还前验证来源及分配状态，失败保留原句柄和内容。
pub unsafe trait VmMemory {
    /// 不复制、不克隆、持有期内有效的独占页句柄。
    type Page: VmPage;
    /// 返回完整管理范围，包含已占用页，不随空闲页数变化。
    fn managed_range(&self) -> Range<PhysAddr>;
    /// 申请并清零一页；耗尽返回 OutOfMemory，不留下本次占用。
    fn alloc_zeroed_page(&mut self) -> Result<Self::Page, PageAllocError>;
    /// 成功消耗句柄；来源错误为 InvalidAddress，分配状态错误为 NotAllocated。
    /// 任何失败均返回原句柄，分配标记和内容保持不变。
    ///
    /// # Safety
    /// 所有访问已结束；临时别名已撤销并同步；表页已脱离可用页表且不再是活动根。
    unsafe fn release_page(&mut self, page: Self::Page)
    -> Result<(), (PageAllocError, Self::Page)>;
}

#[cfg(test)]
pub(crate) mod tests;
