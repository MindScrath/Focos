//! MM-002 的 QEMU virt 内存布局；仅适用于约定的 128 MiB、单 hart 启动。

use core::ops::Range;

use crate::mm::{PAGE_SIZE, PageAllocError, PageAllocator};

/// QEMU virt 的物理内存起址，低端包含 OpenSBI 保留区。
const RAM_START: usize = 0x8000_0000;
/// 128 MiB 配置下物理内存的结束地址，不属于可访问范围。
const RAM_END: usize = 0x8800_0000;
/// 当前链接脚本约定的内核装载地址。
const KERNEL_START: usize = 0x8020_0000;
/// 为整段 RAM 预留标记容量，实际管理范围由内核末尾和设备树限定。
const MAX_PAGE_COUNT: usize = (RAM_END - RAM_START) / PAGE_SIZE;

/// 启动内存布局或分配器初始化失败的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryInitError {
    /// 内核末尾、设备树地址或对齐后的可用区间无效。
    InvalidLayout,
    /// 页分配器拒绝初始化，保留 MM-001 的原始错误。
    Allocator(PageAllocError),
}

/// 计算内核末尾与设备树之间的完整页范围，不访问内存或修改标记。
///
/// `kernel_end` 必须包含全部内核装载段，`dtb_start` 必须按 8 字节对齐。
/// 两者须满足 MM-002 的 RAM 边界；越界、溢出或对齐后无完整页返回 `InvalidLayout`。
pub fn usable_range(kernel_end: usize, dtb_start: usize) -> Result<Range<usize>, MemoryInitError> {
    if kernel_end < KERNEL_START
        || kernel_end >= dtb_start
        || dtb_start >= RAM_END
        || !dtb_start.is_multiple_of(8)
    {
        return Err(MemoryInitError::InvalidLayout);
    }
    let start = kernel_end
        .checked_add(PAGE_SIZE - 1)
        .ok_or(MemoryInitError::InvalidLayout)?
        & !(PAGE_SIZE - 1);
    let end = dtb_start & !(PAGE_SIZE - 1);
    if start >= end {
        return Err(MemoryInitError::InvalidLayout);
    }
    Ok(start..end)
}

/// 校验平台布局后创建页分配器，返回实际管理范围和独占借用标记的分配器。
///
/// 先校验布局，再检查 `used` 容量；失败不修改标记，也不访问待分配页。
pub fn init_page_allocator(
    kernel_end: usize,
    dtb_start: usize,
    used: &mut [bool],
) -> Result<(Range<usize>, PageAllocator<'_>), MemoryInitError> {
    let range = usable_range(kernel_end, dtb_start)?;
    let allocator =
        PageAllocator::new(range.start, range.end, used).map_err(MemoryInitError::Allocator)?;
    Ok((range, allocator))
}

#[cfg(target_arch = "riscv64")]
static mut PAGE_FLAGS: [bool; MAX_PAGE_COUNT] = [false; MAX_PAGE_COUNT];

/// 从链接符号和 OpenSBI 传入的设备树地址初始化启动页分配器。
/// 标记位于内核静态区，由 `__kernel_end` 一并排除在可分配范围之外。
///
/// # Safety
/// 仅可从 MM-002 约定的 QEMU 入口调用一次，且只有启动 hart、关闭中断和分页。
/// `dtb_start` 必须是固件传入的设备树地址；不得另建管理同一内存区间的分配器。
#[cfg(target_arch = "riscv64")]
pub unsafe fn boot_page_allocator(
    dtb_start: usize,
) -> Result<(Range<usize>, PageAllocator<'static>), MemoryInitError> {
    unsafe extern "C" {
        static __kernel_end: u8;
    }
    let kernel_end = (&raw const __kernel_end).addr();
    let flags = &raw mut PAGE_FLAGS;
    // SAFETY: 调用约定限定单 hart、无中断且仅初始化一次；静态标记始终存活，
    // 这是唯一的可变借用，只交给返回的分配器，不与待分配物理页重叠。
    let used = unsafe { &mut *flags };
    init_page_allocator(kernel_end, dtb_start, used)
}

#[cfg(test)]
mod tests;
