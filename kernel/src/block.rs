//! HAL-001：以 512 字节为单位的同步块设备接口。

mod memory;

pub use memory::MemoryDisk;
/// 块大小固定为 512 字节
pub const BLOCK_SIZE: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockError {
    /// 单次读写缓冲区不是一个完整块。
    InvalidBufferLength,
    /// 块编号超出设备容量。
    OutOfBounds,
    /// 内存盘存储长度不是块大小的整数倍。
    InvalidCapacity,
    /// 设备读写或同步失败。
    Io,
    /// 设备不提供所请求的能力。
    Unsupported,
}

/// 上层只依赖此接口，设备实现负责访问内存或硬件。
///
/// 每次传输一个完整块；长度错误优先于块编号错误。
/// `&mut self` 要求调用者串行访问，未来共享设备时需由外层同步。
/// 读写成功不等于已持久化。硬件 I/O 失败可能留下部分传输，
/// 调用者不能将失败操作当成已完成。
pub trait BlockDevice {
    /// 返回设备容量，单位为块，每块 [`BLOCK_SIZE`] 字节。
    /// 有效块编号为 `0..block_count()`；返回 0 表示没有可读写的块。
    fn block_count(&self) -> u64;

    /// 将指定块的数据读入调用者提供的缓冲区。
    ///
    /// # 参数
    /// - `block`：从 0 开始的块编号，不是字节偏移，必须小于总块数。
    /// - `buffer`：接收数据的可变缓冲区，长度必须恰好为 [`BLOCK_SIZE`] 字节。
    ///
    /// # 返回与错误
    /// 成功返回 `Ok(())`，并填满 `buffer`。长度错误返回
    /// [`BlockError::InvalidBufferLength`]，块编号越界返回
    /// [`BlockError::OutOfBounds`]，设备读取失败返回 [`BlockError::Io`]。
    /// 硬件读取失败时，缓冲区可能已被部分修改。
    fn read_block(&mut self, block: u64, buffer: &mut [u8]) -> Result<(), BlockError>;

    /// 将调用者提供的数据写入指定块。
    ///
    /// # 参数
    /// - `block`：从 0 开始的块编号，不是字节偏移，必须小于总块数。
    /// - `buffer`：提供写入数据的只读缓冲区，长度必须恰好为 [`BLOCK_SIZE`] 字节。
    ///
    /// # 返回与错误
    /// 成功返回 `Ok(())`，不修改输入缓冲区；持久化保证由设备的 `flush` 实现提供。
    /// 长度错误返回 [`BlockError::InvalidBufferLength`]，块编号越界返回
    /// [`BlockError::OutOfBounds`]，设备写入失败返回 [`BlockError::Io`]。
    /// 硬件写入失败时，目标块可能已被部分修改。
    fn write_block(&mut self, block: u64, buffer: &[u8]) -> Result<(), BlockError>;

    /// 请求同步当前设备此前的写入，无额外参数。
    ///
    /// 成功返回 `Ok(())`；同步失败返回 [`BlockError::Io`]。
    /// 默认返回 [`BlockError::Unsupported`]，设备实现需明确说明同步的保证范围。
    fn flush(&mut self) -> Result<(), BlockError> {
        Err(BlockError::Unsupported)
    }
}

#[cfg(test)]
mod tests;
