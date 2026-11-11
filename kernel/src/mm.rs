//! MM-001：物理页分配与回收。
//!
//! 页分配器只管理页地址与占用状态，不访问物理内存；`self_test` 提供独立读写自检。

pub mod self_test;

/// 每个物理页的大小，单位为字节。
pub(crate) const PAGE_SIZE: usize = 4096;

/// 页分配器初始化、分配或回收失败的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageAllocError {
    /// 地址范围反向，或任一边界未按 4096 字节对齐。
    InvalidRange,
    /// 调用者提供的标记数少于管理范围内的页数。
    MetadataTooSmall,
    /// 当前没有空闲页。
    OutOfMemory,
    /// 释放地址不对齐，或不在管理范围内。
    InvalidAddress,
    /// 待释放的页当前未被分配。
    NotAllocated,
}

/// 管理 `[start, end)` 内的 4096 字节物理页，支持分配、回收和复用。
///
/// 返回空闲页的页首地址，检查耗尽和非法释放；不访问或清零页内容。
pub struct PageAllocator<'a> {
    /// 管理范围的起始地址，按页对齐。
    start: usize,
    /// 管理范围的结束地址，按页对齐且不包含在范围内。
    end: usize,
    /// 每页一个占用标记（`true` 为已分配），仅覆盖管理范围。
    used: &'a mut [bool],
}

impl<'a> PageAllocator<'a> {
    /// 创建页分配器，将 `[start, end)` 内的页标记为空闲。
    ///
    /// # 参数
    /// - `start`、`end`：均须按页对齐，且 `start <= end`；允许空范围。
    /// - `used`：调用者提供的页状态存储，每页一个标记，长度至少为总页数。
    ///
    /// # 返回与错误
    /// 成功时只将管理范围需要的标记设为空闲，多余标记不变。
    /// 先检查范围，再检查容量，分别返回 [`PageAllocError::InvalidRange`]
    /// 或 [`PageAllocError::MetadataTooSmall`]；失败不修改标记。
    pub fn new(start: usize, end: usize, used: &'a mut [bool]) -> Result<Self, PageAllocError> {
        if start > end || !start.is_multiple_of(PAGE_SIZE) || !end.is_multiple_of(PAGE_SIZE) {
            return Err(PageAllocError::InvalidRange);
        }

        // 先排除反向范围，再相减，避免无符号整数下溢。
        let page_count = (end - start) / PAGE_SIZE;
        if used.len() < page_count {
            return Err(PageAllocError::MetadataTooSmall);
        }

        // 全部校验通过后才修改标记，保证错误返回时调用者的数据不变。
        let used = &mut used[..page_count];
        used.fill(false);
        Ok(Self { start, end, used })
    }

    /// 返回一个范围内、按页对齐且当前未占用的页首地址，并标记为已分配。
    ///
    /// 不约定分配顺序；耗尽返回 [`PageAllocError::OutOfMemory`]，状态不变。
    pub fn alloc(&mut self) -> Result<usize, PageAllocError> {
        let index = self
            .used
            .iter()
            .position(|&used| !used)
            .ok_or(PageAllocError::OutOfMemory)?;

        // new 已将标记裁剪到总页数，地址计算始终落在 [start, end) 内，不会溢出。
        let addr = self.start + index * PAGE_SIZE;
        self.used[index] = true;
        Ok(addr)
    }

    /// 归还本分配器中当前已分配的页，调用者须确保该页已不再使用。
    ///
    /// `addr` 必须为页首地址。先检查地址，再检查分配状态，分别返回
    /// [`PageAllocError::InvalidAddress`] 或 [`PageAllocError::NotAllocated`]。
    /// 失败不修改状态；成功后允许再次分配该页。
    pub fn dealloc(&mut self, addr: usize) -> Result<(), PageAllocError> {
        if !addr.is_multiple_of(PAGE_SIZE) || addr < self.start || addr >= self.end {
            return Err(PageAllocError::InvalidAddress);
        }

        // 先检查地址，再相减和索引，避免下溢或访问管理范围外的标记。
        let index = (addr - self.start) / PAGE_SIZE;
        if !self.used[index] {
            return Err(PageAllocError::NotAllocated);
        }

        self.used[index] = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
