use core::ops::Range;

use super::{BLOCK_SIZE, BlockDevice, BlockError};

/// 独占借用调用者的存储；释放内存盘后，存储仍归调用者所有。
/// 生命周期 `'a` 将内存盘与底层缓冲区的有效期关联，内存盘不能比缓冲区活得更久。
pub struct MemoryDisk<'a> {
    /// 底层连续字节存储，长度始终为块大小的整数倍。
    storage: &'a mut [u8],
}

impl<'a> MemoryDisk<'a> {
    /// 使用调用者提供的内存创建内存盘，保留其中已有的数据。
    ///
    /// # 参数
    /// - `storage`：作为磁盘存储的可变字节切片，长度必须是 [`BLOCK_SIZE`] 的整数倍。
    ///   例如 2048 字节对应 4 个块。允许空切片，此时总块数为 0。
    ///   内存盘独占借用该切片，不分配或释放底层内存；借用结束后调用者可重新访问它。
    ///
    /// # 返回与错误
    /// 成功返回借用该存储的内存盘；容量不是完整块数时返回
    /// [`BlockError::InvalidCapacity`]。构造过程不修改存储内容。
    pub fn new(storage: &'a mut [u8]) -> Result<Self, BlockError> {
        if !storage.len().is_multiple_of(BLOCK_SIZE) {
            return Err(BlockError::InvalidCapacity);
        }
        Ok(Self { storage })
    }

    /// 在访问数据前完成所有校验，使参数错误不会产生部分读写。
    ///
    /// # 参数
    /// - `block`：从 0 开始的块编号，必须小于当前内存盘的总块数。
    /// - `buffer_len`：本次读写缓冲区的字节数，必须等于 [`BLOCK_SIZE`]。
    ///
    /// # 返回与错误
    /// 返回底层存储中的字节区间，左闭右开；例如第 2 块对应 `1024..1536`。
    /// 优先报告 [`BlockError::InvalidBufferLength`]；块编号越界、整数转换失败
    /// 或偏移计算溢出时返回 [`BlockError::OutOfBounds`]。此函数不修改任何数据。
    fn block_range(&self, block: u64, buffer_len: usize) -> Result<Range<usize>, BlockError> {
        if buffer_len != BLOCK_SIZE {
            return Err(BlockError::InvalidBufferLength);
        }
        if block >= self.block_count() {
            return Err(BlockError::OutOfBounds);
        }
        let index = usize::try_from(block).map_err(|_| BlockError::OutOfBounds)?;
        let start = index
            .checked_mul(BLOCK_SIZE)
            .ok_or(BlockError::OutOfBounds)?;
        let end = start
            .checked_add(BLOCK_SIZE)
            .ok_or(BlockError::OutOfBounds)?;
        Ok(start..end)
    }
}

impl BlockDevice for MemoryDisk<'_> {
    /// 返回底层存储可容纳的完整块数，即字节长度除以 [`BLOCK_SIZE`]。
    fn block_count(&self) -> u64 {
        (self.storage.len() / BLOCK_SIZE) as u64
    }

    /// 将从 0 开始编号的 `block` 块复制到输出缓冲区 `buffer`。
    /// `buffer` 必须恰好为 [`BLOCK_SIZE`] 字节，成功后全部覆盖为该块内容。
    /// 长度错误或块编号越界时返回对应错误，内存盘与输出缓冲区均保持不变。
    fn read_block(&mut self, block: u64, buffer: &mut [u8]) -> Result<(), BlockError> {
        let range = self.block_range(block, buffer.len())?;
        buffer.copy_from_slice(&self.storage[range]);
        Ok(())
    }

    /// 将输入缓冲区 `buffer` 复制到从 0 开始编号的 `block` 块。
    /// `buffer` 必须恰好为 [`BLOCK_SIZE`] 字节，成功后覆盖目标块，其他块保持不变。
    /// 长度错误或块编号越界时返回对应错误，盘内数据不变；输入缓冲区始终不变。
    fn write_block(&mut self, block: u64, buffer: &[u8]) -> Result<(), BlockError> {
        let range = self.block_range(block, buffer.len())?;
        self.storage[range].copy_from_slice(buffer);
        Ok(())
    }

    /// 同步当前内存盘，无额外参数，始终返回 `Ok(())`。
    /// 写入已直接修改底层内存，因此无需额外操作；不提供断电持久化保证。
    fn flush(&mut self) -> Result<(), BlockError> {
        // 写入已直接落在借用的内存中；没有物理设备或持久化承诺。
        Ok(())
    }
}

#[cfg(test)]
mod tests;
