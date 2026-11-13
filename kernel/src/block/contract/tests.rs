//! 契约检查自身的负例：每个非合规设备对应一类真实后端可能的缺陷，
//! 验证检查能在对应场景下拒绝它。

use super::{
    ContractFailure, FailureKind, check_buffer_length_validation, check_empty_device,
    check_first_last_write_preserves_neighbors, check_flush_ok, check_known_data,
    check_out_of_bounds_rejected, check_read_write_roundtrip, check_write_isolation,
};
use crate::block::{BLOCK_SIZE, BlockDevice, BlockError, MemoryDisk};

/// 非合规设备：读路径先查块号再查长度，违反长度错误优先约定。
struct BlockFirstReadDisk<'a> {
    inner: MemoryDisk<'a>,
}

impl BlockDevice for BlockFirstReadDisk<'_> {
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
    fn read_block(&mut self, block: u64, buffer: &mut [u8]) -> Result<(), BlockError> {
        if block >= self.block_count() {
            return Err(BlockError::OutOfBounds);
        }
        self.inner.read_block(block, buffer)
    }
    fn write_block(&mut self, block: u64, buffer: &[u8]) -> Result<(), BlockError> {
        self.inner.write_block(block, buffer)
    }
}

#[test]
fn length_priority_covers_read_path() {
    let mut storage = [0x33; BLOCK_SIZE];
    let mut disk = BlockFirstReadDisk {
        inner: MemoryDisk::new(&mut storage).unwrap(),
    };
    assert!(check_buffer_length_validation(&mut disk).is_err());
}

/// 非合规设备：写入某块时把相同数据串写到下一块。
struct SpillingWriteDisk<'a> {
    inner: MemoryDisk<'a>,
}

impl BlockDevice for SpillingWriteDisk<'_> {
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
    fn read_block(&mut self, block: u64, buffer: &mut [u8]) -> Result<(), BlockError> {
        self.inner.read_block(block, buffer)
    }
    fn write_block(&mut self, block: u64, buffer: &[u8]) -> Result<(), BlockError> {
        self.inner.write_block(block, buffer)?;
        let next = block + 1;
        if next < self.block_count() {
            self.inner.write_block(next, buffer)?;
        }
        Ok(())
    }
}

#[test]
fn neighbor_check_covers_adjacent_blocks() {
    let mut storage = [0x33; BLOCK_SIZE * 4];
    let mut disk = SpillingWriteDisk {
        inner: MemoryDisk::new(&mut storage).unwrap(),
    };
    assert!(check_first_last_write_preserves_neighbors(&mut disk).is_err());
}

/// 非合规设备：读只回放会话内最后写入的缓存，未写块返回伪造内容，
/// 从不读取介质原有数据。
struct WriteOnlyCacheDisk<'a> {
    inner: MemoryDisk<'a>,
    cached: Option<(u64, [u8; BLOCK_SIZE])>,
}

impl BlockDevice for WriteOnlyCacheDisk<'_> {
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
    fn read_block(&mut self, block: u64, buffer: &mut [u8]) -> Result<(), BlockError> {
        if block >= self.block_count() {
            return Err(BlockError::OutOfBounds);
        }
        if buffer.len() != BLOCK_SIZE {
            return Err(BlockError::InvalidBufferLength);
        }
        match self.cached {
            Some((cached_block, data)) if cached_block == block => {
                buffer.copy_from_slice(&data);
            }
            _ => {
                buffer.fill(0xAA);
            }
        }
        Ok(())
    }
    fn write_block(&mut self, block: u64, buffer: &[u8]) -> Result<(), BlockError> {
        if block >= self.block_count() {
            return Err(BlockError::OutOfBounds);
        }
        if buffer.len() != BLOCK_SIZE {
            return Err(BlockError::InvalidBufferLength);
        }
        let data: [u8; BLOCK_SIZE] = buffer.try_into().unwrap();
        self.cached = Some((block, data));
        Ok(())
    }
    fn flush(&mut self) -> Result<(), BlockError> {
        Ok(())
    }
}

#[test]
fn known_data_rejects_cache_only_reads() {
    // 两个全新实例分离证明：只回放会话内写入的假后端能通过写后读，
    // 但在读取介质原有内容的已知数据检查处失败——顺序不可互换。
    let original: [u8; BLOCK_SIZE] = core::array::from_fn(|i| (i ^ (i >> 8)) as u8);
    let mut fresh = original;
    let mut fresh_disk = WriteOnlyCacheDisk {
        inner: MemoryDisk::new(&mut fresh).unwrap(),
        cached: None,
    };
    assert!(check_known_data(&mut fresh_disk, 0, &original).is_err());

    let mut written = original;
    let mut written_disk = WriteOnlyCacheDisk {
        inner: MemoryDisk::new(&mut written).unwrap(),
        cached: None,
    };
    check_read_write_roundtrip(&mut written_disk).unwrap();
}

/// 非合规设备：写首块时把垃圾数据串写到末块，模拟地址回绕式跨块破坏。
struct FirstWriteClobbersLastDisk<'a> {
    inner: MemoryDisk<'a>,
}

impl BlockDevice for FirstWriteClobbersLastDisk<'_> {
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
    fn read_block(&mut self, block: u64, buffer: &mut [u8]) -> Result<(), BlockError> {
        self.inner.read_block(block, buffer)
    }
    fn write_block(&mut self, block: u64, buffer: &[u8]) -> Result<(), BlockError> {
        self.inner.write_block(block, buffer)?;
        if block == 0 && self.block_count() > 1 {
            let last = self.block_count() - 1;
            self.inner.write_block(last, &[0xEE; BLOCK_SIZE])?;
        }
        Ok(())
    }
}

#[test]
fn neighbor_check_catches_clobber_before_healing() {
    // 写末块会治愈首块造成的破坏，检查必须在写末块之前发现它。
    let mut storage = [0x33; BLOCK_SIZE * 4];
    let mut disk = FirstWriteClobbersLastDisk {
        inner: MemoryDisk::new(&mut storage).unwrap(),
    };
    assert!(check_first_last_write_preserves_neighbors(&mut disk).is_err());
}

/// 非合规设备：被拒的非法长度写入先把垃圾写进第 0 块，再返回长度错误。
struct DestructiveRejectDisk<'a> {
    inner: MemoryDisk<'a>,
}

impl BlockDevice for DestructiveRejectDisk<'_> {
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
    fn read_block(&mut self, block: u64, buffer: &mut [u8]) -> Result<(), BlockError> {
        self.inner.read_block(block, buffer)
    }
    fn write_block(&mut self, block: u64, buffer: &[u8]) -> Result<(), BlockError> {
        if buffer.len() != BLOCK_SIZE {
            self.inner.write_block(0, &[0xEE; BLOCK_SIZE])?;
            return Err(BlockError::InvalidBufferLength);
        }
        self.inner.write_block(block, buffer)
    }
}

#[test]
fn length_validation_rejects_destructive_errors() {
    // 只查调用者缓冲区发现不了"先破坏再报错"的后端，必须重读目标块。
    let mut storage = [0x33; BLOCK_SIZE];
    let mut disk = DestructiveRejectDisk {
        inner: MemoryDisk::new(&mut storage).unwrap(),
    };
    assert!(check_buffer_length_validation(&mut disk).is_err());
}

/// 非合规设备：对空切片特判返回长度错误，对非空长度却先查块号。
struct EmptySpecialCaseDisk<'a> {
    inner: MemoryDisk<'a>,
}

impl BlockDevice for EmptySpecialCaseDisk<'_> {
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
    fn read_block(&mut self, block: u64, buffer: &mut [u8]) -> Result<(), BlockError> {
        if buffer.is_empty() {
            return Err(BlockError::InvalidBufferLength);
        }
        if block >= self.block_count() {
            return Err(BlockError::OutOfBounds);
        }
        self.inner.read_block(block, buffer)
    }
    fn write_block(&mut self, block: u64, buffer: &[u8]) -> Result<(), BlockError> {
        if buffer.is_empty() {
            return Err(BlockError::InvalidBufferLength);
        }
        if block >= self.block_count() {
            return Err(BlockError::OutOfBounds);
        }
        self.inner.write_block(block, buffer)
    }
}

#[test]
fn length_priority_covers_all_illegal_lengths() {
    // 非空非法长度也必须与非法块号组合验证长度优先，
    // 否则"只特判空切片"的后端能绕过优先级约定。
    let mut storage = [0x33; BLOCK_SIZE];
    let mut disk = EmptySpecialCaseDisk {
        inner: MemoryDisk::new(&mut storage).unwrap(),
    };
    assert!(check_buffer_length_validation(&mut disk).is_err());
}

/// 非合规设备：越界访问被拒时误写第 1 块（模拟越界偏移换算错误）。
struct OobSpillDisk<'a> {
    inner: MemoryDisk<'a>,
}

impl BlockDevice for OobSpillDisk<'_> {
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
    fn read_block(&mut self, block: u64, buffer: &mut [u8]) -> Result<(), BlockError> {
        if block >= self.block_count() {
            let _ = self.inner.write_block(1, &[0xEE; BLOCK_SIZE]);
            return Err(BlockError::OutOfBounds);
        }
        self.inner.read_block(block, buffer)
    }
    fn write_block(&mut self, block: u64, buffer: &[u8]) -> Result<(), BlockError> {
        if block >= self.block_count() {
            let _ = self.inner.write_block(1, &[0xEE; BLOCK_SIZE]);
            return Err(BlockError::OutOfBounds);
        }
        self.inner.write_block(block, buffer)
    }
}

#[test]
fn out_of_bounds_check_covers_every_block() {
    // 4 块盘上采样探针会漏掉第 1 块，越界被拒时写坏它必须被检出。
    let mut storage = [0x33; BLOCK_SIZE * 4];
    let mut disk = OobSpillDisk {
        inner: MemoryDisk::new(&mut storage).unwrap(),
    };
    assert!(check_out_of_bounds_rejected(&mut disk).is_err());
}

/// 非合规设备：写首块或末块时误写中间的第 2 块（模拟跨块地址串扰）。
struct MidSpillDisk<'a> {
    inner: MemoryDisk<'a>,
}

impl BlockDevice for MidSpillDisk<'_> {
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
    fn read_block(&mut self, block: u64, buffer: &mut [u8]) -> Result<(), BlockError> {
        self.inner.read_block(block, buffer)
    }
    fn write_block(&mut self, block: u64, buffer: &[u8]) -> Result<(), BlockError> {
        self.inner.write_block(block, buffer)?;
        let count = self.block_count();
        if block == 0 || block == count - 1 {
            self.inner.write_block(2, &[0xEE; BLOCK_SIZE])?;
        }
        Ok(())
    }
}

#[test]
fn neighbor_check_covers_every_block() {
    // 6 块盘上采样探针会漏掉第 2 块，写首末块时写坏它必须被检出。
    let mut storage = [0x33; BLOCK_SIZE * 6];
    let mut disk = MidSpillDisk {
        inner: MemoryDisk::new(&mut storage).unwrap(),
    };
    assert!(check_first_last_write_preserves_neighbors(&mut disk).is_err());
}

#[test]
fn probe_patterns_distinguish_all_block_numbers() {
    // 头部编码完整块号：相差 256 整数倍的块图样必须不同（旧实现把块号
    // 截断为 u8 时块 1 与块 257 同型，读混后端能骗过全盘校验）。
    assert_ne!(super::probe_pattern(1), super::probe_pattern(257));
    assert_ne!(super::probe_pattern(0), super::probe_pattern(256));
    assert_ne!(super::probe_pattern(2), super::probe_pattern(258));
    assert_ne!(super::probe_pattern(5), super::probe_pattern(6));
    assert_ne!(super::probe_pattern(4096), super::probe_pattern(4352));
}

/// 非合规设备：容量为 0，参数校验先查块号再查长度。
struct EmptyBlockFirstDisk;

impl BlockDevice for EmptyBlockFirstDisk {
    fn block_count(&self) -> u64 {
        0
    }
    fn read_block(&mut self, _block: u64, _buffer: &mut [u8]) -> Result<(), BlockError> {
        Err(BlockError::OutOfBounds)
    }
    fn write_block(&mut self, _block: u64, _buffer: &[u8]) -> Result<(), BlockError> {
        Err(BlockError::OutOfBounds)
    }
}

#[test]
fn empty_device_check_covers_length_priority() {
    // 空盘上长度错误仍须优先于块号错误，"先查块号"的实现在此必须被拒收。
    let mut disk = EmptyBlockFirstDisk;
    assert!(check_empty_device(&mut disk).is_err());
}

/// 非合规设备：容量为 0，块 0 被拒但错误接受其他块号的读写。
struct EmptyAcceptsMaxDisk;

impl BlockDevice for EmptyAcceptsMaxDisk {
    fn block_count(&self) -> u64 {
        0
    }
    fn read_block(&mut self, block: u64, buffer: &mut [u8]) -> Result<(), BlockError> {
        if buffer.len() != BLOCK_SIZE {
            return Err(BlockError::InvalidBufferLength);
        }
        if block == 0 {
            return Err(BlockError::OutOfBounds);
        }
        buffer.fill(0xEE);
        Ok(())
    }
    fn write_block(&mut self, block: u64, buffer: &[u8]) -> Result<(), BlockError> {
        if buffer.len() != BLOCK_SIZE {
            return Err(BlockError::InvalidBufferLength);
        }
        if block == 0 {
            return Err(BlockError::OutOfBounds);
        }
        Ok(())
    }
}

#[test]
fn empty_device_check_rejects_nonzero_blocks() {
    // 空盘上任何块编号都越界：只拒块 0 却接受 u64::MAX 的实现必须被检出。
    let mut disk = EmptyAcceptsMaxDisk;
    assert!(check_empty_device(&mut disk).is_err());
}

/// 非合规设备：读某块时先正确填充缓冲区，随后破坏编号低一块的介质内容。
struct ReadCorruptsPreviousDisk<'a> {
    inner: MemoryDisk<'a>,
}

impl BlockDevice for ReadCorruptsPreviousDisk<'_> {
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
    fn read_block(&mut self, block: u64, buffer: &mut [u8]) -> Result<(), BlockError> {
        self.inner.read_block(block, buffer)?;
        if block >= 1 {
            let previous = block - 1;
            self.inner.write_block(previous, &[0xEE; BLOCK_SIZE])?;
        }
        Ok(())
    }
    fn write_block(&mut self, block: u64, buffer: &[u8]) -> Result<(), BlockError> {
        self.inner.write_block(block, buffer)
    }
}

#[test]
fn baseline_verify_rechecks_after_full_pass() {
    // 单次升序遍历会在副作用发生前比对低块，"读高块破坏低块"的后端
    // 必须被第二遍遍历检出。
    let mut storage = [0x33; BLOCK_SIZE * 4];
    let mut disk = ReadCorruptsPreviousDisk {
        inner: MemoryDisk::new(&mut storage).unwrap(),
    };
    assert!(check_out_of_bounds_rejected(&mut disk).is_err());
}

/// 非合规设备：容量为 0，仅对块 0 先校验长度，其他块号先返回越界。
struct EmptyBlockFirstOtherDisk;

impl BlockDevice for EmptyBlockFirstOtherDisk {
    fn block_count(&self) -> u64 {
        0
    }
    fn read_block(&mut self, block: u64, buffer: &mut [u8]) -> Result<(), BlockError> {
        if buffer.len() != BLOCK_SIZE && block == 0 {
            return Err(BlockError::InvalidBufferLength);
        }
        Err(BlockError::OutOfBounds)
    }
    fn write_block(&mut self, block: u64, buffer: &[u8]) -> Result<(), BlockError> {
        if buffer.len() != BLOCK_SIZE && block == 0 {
            return Err(BlockError::InvalidBufferLength);
        }
        Err(BlockError::OutOfBounds)
    }
}

#[test]
fn empty_device_length_priority_covers_all_blocks() {
    // 长度优先约定适用于全部块号：非法长度与非零、巨大块号也须组合验证。
    let mut disk = EmptyBlockFirstOtherDisk;
    assert!(check_empty_device(&mut disk).is_err());
}

/// 非合规设备：读块 0 以外的块时先正确填充缓冲区，随后破坏块 0。
struct FlushReadClobbersZeroDisk<'a> {
    inner: MemoryDisk<'a>,
}

impl BlockDevice for FlushReadClobbersZeroDisk<'_> {
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
    fn read_block(&mut self, block: u64, buffer: &mut [u8]) -> Result<(), BlockError> {
        self.inner.read_block(block, buffer)?;
        if block >= 1 {
            self.inner.write_block(0, &[0xEE; BLOCK_SIZE])?;
        }
        Ok(())
    }
    fn write_block(&mut self, block: u64, buffer: &[u8]) -> Result<(), BlockError> {
        self.inner.write_block(block, buffer)
    }
    fn flush(&mut self) -> Result<(), BlockError> {
        self.inner.flush()
    }
}

#[test]
fn flush_check_rechecks_target_after_baseline_verify() {
    // 终态复核必须包含写入目标块（0x55）的当前期望值：读取路径在
    // 目标块比对之后破坏它时，排除目标块的复核发现不了。
    let mut storage = [0x33; BLOCK_SIZE * 3];
    let mut disk = FlushReadClobbersZeroDisk {
        inner: MemoryDisk::new(&mut storage).unwrap(),
    };
    assert!(check_flush_ok(&mut disk).is_err());
}

/// 非合规设备：写块 1 时把垃圾串写到块 2（后续基线写入会治愈它）。
struct WriteClobbersLaterDisk<'a> {
    inner: MemoryDisk<'a>,
}

impl BlockDevice for WriteClobbersLaterDisk<'_> {
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
    fn read_block(&mut self, block: u64, buffer: &mut [u8]) -> Result<(), BlockError> {
        self.inner.read_block(block, buffer)
    }
    fn write_block(&mut self, block: u64, buffer: &[u8]) -> Result<(), BlockError> {
        self.inner.write_block(block, buffer)?;
        if block == 1 {
            self.inner.write_block(2, &[0xEE; BLOCK_SIZE])?;
        }
        Ok(())
    }
}

#[test]
fn write_isolation_rewrites_each_block_with_immediate_verify() {
    // 顺序基线写入会治愈先前写入的串扰：逐块重写并即时验证其余块，
    // 使写侧跨块破坏无法被后续写入掩盖。
    let mut storage = [0x33; BLOCK_SIZE * 3];
    let mut disk = WriteClobbersLaterDisk {
        inner: MemoryDisk::new(&mut storage).unwrap(),
    };
    assert!(check_write_isolation(&mut disk).is_err());
}

/// 非合规设备：静默丢弃对块 1 的写入但返回成功。
struct MirrorDropMiddleDisk<'a> {
    inner: MemoryDisk<'a>,
}

impl BlockDevice for MirrorDropMiddleDisk<'_> {
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
    fn read_block(&mut self, block: u64, buffer: &mut [u8]) -> Result<(), BlockError> {
        self.inner.read_block(block, buffer)
    }
    fn write_block(&mut self, block: u64, buffer: &[u8]) -> Result<(), BlockError> {
        if block == 1 {
            return Ok(());
        }
        self.inner.write_block(block, buffer)
    }
}

#[test]
fn write_isolation_rewrites_with_distinct_pattern_and_verifies_target() {
    // 测试盘预置成契约基线的镜像：静默丢弃块 1 写入的后端，靠"重写图样
    // 与基线不同 + 目标块纳入即时复核"才能被检出。
    let mut storage = [0u8; BLOCK_SIZE * 4];
    for (index, chunk) in storage.chunks_exact_mut(BLOCK_SIZE).enumerate() {
        chunk.copy_from_slice(&super::probe_pattern(index as u64));
    }
    let mut disk = MirrorDropMiddleDisk {
        inner: MemoryDisk::new(&mut storage).unwrap(),
    };
    assert!(check_write_isolation(&mut disk).is_err());
}

/// 非合规设备：静默丢弃启动后的第一次写入，后续写入正常。
struct DropFirstWriteDisk<'a> {
    inner: MemoryDisk<'a>,
    first_write_dropped: bool,
}

impl BlockDevice for DropFirstWriteDisk<'_> {
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
    fn read_block(&mut self, block: u64, buffer: &mut [u8]) -> Result<(), BlockError> {
        self.inner.read_block(block, buffer)
    }
    fn write_block(&mut self, block: u64, buffer: &[u8]) -> Result<(), BlockError> {
        if self.first_write_dropped {
            self.first_write_dropped = false;
            return Ok(());
        }
        self.inner.write_block(block, buffer)
    }
}

#[test]
fn write_isolation_probes_first_write_before_baseline() {
    // 只丢第一次写入的后端：基线写块 0 被丢且未验证，随后的隔离重写
    // 会抹去证据；首写可观测探针（读现值、写取反、立即读回）必须先拒收它。
    let mut storage = [0x33; BLOCK_SIZE * 3];
    let mut disk = DropFirstWriteDisk {
        inner: MemoryDisk::new(&mut storage).unwrap(),
        first_write_dropped: true,
    };
    assert!(check_write_isolation(&mut disk).is_err());
}

/// 非合规设备：只静默丢弃第二次写入，其余写入正常。
struct DropSecondWriteDisk<'a> {
    inner: MemoryDisk<'a>,
    writes_done: u32,
}

impl BlockDevice for DropSecondWriteDisk<'_> {
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
    fn read_block(&mut self, block: u64, buffer: &mut [u8]) -> Result<(), BlockError> {
        self.inner.read_block(block, buffer)
    }
    fn write_block(&mut self, block: u64, buffer: &[u8]) -> Result<(), BlockError> {
        self.writes_done += 1;
        if self.writes_done == 2 {
            return Ok(());
        }
        self.inner.write_block(block, buffer)
    }
}

#[test]
fn write_isolation_verifies_baseline_before_rewrite() {
    // 基线写入被静默丢弃时，重写循环会抹掉证据；进入重写前必须整盘
    // 复核已建立的基线。
    let mut storage = [0x33; BLOCK_SIZE * 3];
    let mut disk = DropSecondWriteDisk {
        inner: MemoryDisk::new(&mut storage).unwrap(),
        writes_done: 0,
    };
    assert!(check_write_isolation(&mut disk).is_err());
}

/// 非合规设备：首次成功写入时正确落盘目标块，同时误写块 2。
struct FirstWriteClobbersTwoDisk<'a> {
    inner: MemoryDisk<'a>,
    first_write_done: bool,
}

impl BlockDevice for FirstWriteClobbersTwoDisk<'_> {
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
    fn read_block(&mut self, block: u64, buffer: &mut [u8]) -> Result<(), BlockError> {
        self.inner.read_block(block, buffer)
    }
    fn write_block(&mut self, block: u64, buffer: &[u8]) -> Result<(), BlockError> {
        self.inner.write_block(block, buffer)?;
        if !self.first_write_done {
            self.first_write_done = true;
            self.inner.write_block(2, &[0xEE; BLOCK_SIZE])?;
        }
        Ok(())
    }
}

#[test]
fn write_isolation_probes_other_blocks_around_first_write() {
    // 首次写入的跨块串扰会被随后的基线写入治愈：首写前后取其余块
    // 状态摘要并比对，串扰必须在基线建立前暴露。
    let mut storage = [0x33; BLOCK_SIZE * 3];
    let mut disk = FirstWriteClobbersTwoDisk {
        inner: MemoryDisk::new(&mut storage).unwrap(),
        first_write_done: false,
    };
    assert!(check_write_isolation(&mut disk).is_err());
}

/// 非合规设备：首次成功写入时交换块 1 和块 2 的介质内容。
struct FirstWriteSwapsOneTwoDisk<'a> {
    inner: MemoryDisk<'a>,
    first_write_done: bool,
}

impl BlockDevice for FirstWriteSwapsOneTwoDisk<'_> {
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
    fn read_block(&mut self, block: u64, buffer: &mut [u8]) -> Result<(), BlockError> {
        self.inner.read_block(block, buffer)
    }
    fn write_block(&mut self, block: u64, buffer: &[u8]) -> Result<(), BlockError> {
        self.inner.write_block(block, buffer)?;
        if !self.first_write_done {
            self.first_write_done = true;
            let mut first = [0u8; BLOCK_SIZE];
            let mut second = [0u8; BLOCK_SIZE];
            self.inner.read_block(1, &mut first)?;
            self.inner.read_block(2, &mut second)?;
            self.inner.write_block(1, &second)?;
            self.inner.write_block(2, &first)?;
        }
        Ok(())
    }
}

#[test]
fn write_isolation_digest_is_order_sensitive() {
    // 块 1、2 初始内容不同：首次写入交换两块时，纯异或与求和的摘要
    // 保持不变；混入块号的摘要必须能检出整块置换。
    let mut storage = [0u8; BLOCK_SIZE * 3];
    storage[BLOCK_SIZE..BLOCK_SIZE * 2].fill(0x44);
    storage[BLOCK_SIZE * 2..].fill(0x55);
    let mut disk = FirstWriteSwapsOneTwoDisk {
        inner: MemoryDisk::new(&mut storage).unwrap(),
        first_write_done: false,
    };
    assert!(check_write_isolation(&mut disk).is_err());
}

/// 非合规设备：首次成功写入时把块 2 的偏移 0 改为 0x34、偏移 8 改为
/// 0x32（同通道等权字节相抵消），其余字节不变。
struct FirstWriteOffsettingClobberDisk<'a> {
    inner: MemoryDisk<'a>,
    first_write_done: bool,
}

impl BlockDevice for FirstWriteOffsettingClobberDisk<'_> {
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
    fn read_block(&mut self, block: u64, buffer: &mut [u8]) -> Result<(), BlockError> {
        self.inner.read_block(block, buffer)
    }
    fn write_block(&mut self, block: u64, buffer: &[u8]) -> Result<(), BlockError> {
        self.inner.write_block(block, buffer)?;
        if !self.first_write_done {
            self.first_write_done = true;
            let mut junk = [0x33u8; BLOCK_SIZE];
            junk[0] = 0x34;
            junk[8] = 0x32;
            self.inner.write_block(2, &junk)?;
        }
        Ok(())
    }
}

#[test]
fn write_isolation_probes_with_lossless_snapshot() {
    // 加法摘要存在同通道等权字节抵消盲区：首写隔离必须整块快照逐块
    // 无损比对，偏移 0 与 8 的一加一减也要被检出。
    let mut storage = [0x33; BLOCK_SIZE * 3];
    let mut disk = FirstWriteOffsettingClobberDisk {
        inner: MemoryDisk::new(&mut storage).unwrap(),
        first_write_done: false,
    };
    assert!(check_write_isolation(&mut disk).is_err());
}

/// 非合规设备：读取返回成功但不填充调用者缓冲区。
struct LeavesBufferUntouchedDisk<'a> {
    inner: MemoryDisk<'a>,
}

impl BlockDevice for LeavesBufferUntouchedDisk<'_> {
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
    fn read_block(&mut self, block: u64, buffer: &mut [u8]) -> Result<(), BlockError> {
        if block >= self.block_count() {
            return Err(BlockError::OutOfBounds);
        }
        if buffer.len() != BLOCK_SIZE {
            return Err(BlockError::InvalidBufferLength);
        }
        Ok(())
    }
    fn write_block(&mut self, block: u64, buffer: &[u8]) -> Result<(), BlockError> {
        self.inner.write_block(block, buffer)
    }
}

#[test]
fn known_data_prefills_buffer_with_complement() {
    // 全零扇区 + 返回成功却不填缓冲区的后端，靠零初值恰好相等蒙混；
    // 缓冲区按期望值取反预填后必须被检出。
    let mut storage = [0u8; BLOCK_SIZE];
    let mut disk = LeavesBufferUntouchedDisk {
        inner: MemoryDisk::new(&mut storage).unwrap(),
    };
    let expected = [0u8; BLOCK_SIZE];
    assert!(check_known_data(&mut disk, 0, &expected).is_err());
}

/// 非合规设备：首次写块 0 时破坏块 1，第二次写块 0 时恢复块 1
/// （按写入次序触发的两阶段缓存缺陷）。
struct TwoPhaseCacheDisk<'a> {
    inner: MemoryDisk<'a>,
    saved: Option<[u8; BLOCK_SIZE]>,
}

impl BlockDevice for TwoPhaseCacheDisk<'_> {
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
    fn read_block(&mut self, block: u64, buffer: &mut [u8]) -> Result<(), BlockError> {
        self.inner.read_block(block, buffer)
    }
    fn write_block(&mut self, block: u64, buffer: &[u8]) -> Result<(), BlockError> {
        self.inner.write_block(block, buffer)?;
        if block == 0 {
            match self.saved {
                None => {
                    let mut current = [0u8; BLOCK_SIZE];
                    self.inner.read_block(1, &mut current)?;
                    self.saved = Some(current);
                    self.inner.write_block(1, &[0xEE; BLOCK_SIZE])?;
                }
                Some(saved) => {
                    self.inner.write_block(1, &saved)?;
                    self.saved = None;
                }
            }
        }
        Ok(())
    }
}

#[test]
fn roundtrip_verifies_full_state_after_each_write() {
    // 两阶段缓存缺陷：写 first 破坏块 1、写 second 恢复，两次目标读回
    // 都正确；每次写入后的全盘复核必须在恢复发生前暴露破坏。
    let mut storage = [0x33; BLOCK_SIZE * 3];
    let mut disk = TwoPhaseCacheDisk {
        inner: MemoryDisk::new(&mut storage).unwrap(),
        saved: None,
    };
    assert!(check_read_write_roundtrip(&mut disk).is_err());
}

#[test]
fn write_isolation_reports_fixture_limit_beyond_capacity() {
    // 超出快照上限是夹具/资源限制，不是设备容量错误，须用专属变体报告。
    let mut storage = [0u8; BLOCK_SIZE * 40];
    let mut disk = MemoryDisk::new(&mut storage).unwrap();
    assert_eq!(
        check_write_isolation(&mut disk),
        Err(ContractFailure {
            scenario: "write_isolation",
            kind: FailureKind::FixtureLimitExceeded {
                limit: 33,
                actual: 40
            }
        })
    );
}

/// 非合规设备：识别契约探针图样（头部为块号小端编码）的写入，
/// 并把下一块串写为垃圾。
struct ProbeSpillToNextDisk<'a> {
    inner: MemoryDisk<'a>,
}

impl BlockDevice for ProbeSpillToNextDisk<'_> {
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
    fn read_block(&mut self, block: u64, buffer: &mut [u8]) -> Result<(), BlockError> {
        self.inner.read_block(block, buffer)
    }
    fn write_block(&mut self, block: u64, buffer: &[u8]) -> Result<(), BlockError> {
        self.inner.write_block(block, buffer)?;
        let header_matches =
            buffer.len() == BLOCK_SIZE && buffer[..8] == (block as u64).to_le_bytes();
        if header_matches && block + 1 < self.block_count() {
            self.inner.write_block(block + 1, &[0xEE; BLOCK_SIZE])?;
        }
        Ok(())
    }
}

#[test]
fn baseline_rewrites_verify_immediately_against_healing() {
    // 顺序基线写入会治愈"探针写入串写下一块"的破坏：基线第二遍逐块
    // 重写并在每次写入后整盘复核，破坏在治愈前暴露。
    let mut storage = [0x33; BLOCK_SIZE * 3];
    let mut disk = ProbeSpillToNextDisk {
        inner: MemoryDisk::new(&mut storage).unwrap(),
    };
    assert!(check_out_of_bounds_rejected(&mut disk).is_err());
}

/// 非合规设备：非法长度读取时保存块 1 并写入垃圾，非法长度写入时
/// 把块 1 恢复（破坏/恢复跨操作配对）。
struct IllegalReadSaveRestoreDisk<'a> {
    inner: MemoryDisk<'a>,
    armed: bool,
    saved: Option<[u8; BLOCK_SIZE]>,
}

impl BlockDevice for IllegalReadSaveRestoreDisk<'_> {
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
    fn read_block(&mut self, block: u64, buffer: &mut [u8]) -> Result<(), BlockError> {
        if buffer.len() != BLOCK_SIZE {
            if self.armed {
                self.armed = false;
                let mut current = [0u8; BLOCK_SIZE];
                self.inner.read_block(1, &mut current)?;
                self.saved = Some(current);
                self.inner.write_block(1, &[0xEE; BLOCK_SIZE])?;
            }
            return Err(BlockError::InvalidBufferLength);
        }
        self.inner.read_block(block, buffer)
    }
    fn write_block(&mut self, block: u64, buffer: &[u8]) -> Result<(), BlockError> {
        if buffer.len() != BLOCK_SIZE {
            if let Some(saved) = self.saved.take() {
                self.inner.write_block(1, &saved)?;
            }
            return Err(BlockError::InvalidBufferLength);
        }
        self.inner.write_block(block, buffer)
    }
}

#[test]
fn length_check_verifies_media_after_each_rejected_op() {
    // 非法读取破坏块 1、非法写入恢复的两阶段缺陷：每个被拒操作返回后
    // 立即整盘复核，恢复发生前就暴露破坏。
    let mut storage = [0x33; BLOCK_SIZE * 2];
    let mut disk = IllegalReadSaveRestoreDisk {
        inner: MemoryDisk::new(&mut storage).unwrap(),
        armed: true,
        saved: None,
    };
    assert!(check_buffer_length_validation(&mut disk).is_err());
}

/// 非合规设备：仅对块 0 与巨大块号先校验长度，其他块号先返回越界。
struct BlockCountFirstDisk<'a> {
    inner: MemoryDisk<'a>,
}

impl BlockDevice for BlockCountFirstDisk<'_> {
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
    fn read_block(&mut self, block: u64, buffer: &mut [u8]) -> Result<(), BlockError> {
        if buffer.len() != BLOCK_SIZE && (block == 0 || block == u64::MAX) {
            return Err(BlockError::InvalidBufferLength);
        }
        if block >= self.block_count() {
            return Err(BlockError::OutOfBounds);
        }
        self.inner.read_block(block, buffer)
    }
    fn write_block(&mut self, block: u64, buffer: &[u8]) -> Result<(), BlockError> {
        if buffer.len() != BLOCK_SIZE && (block == 0 || block == u64::MAX) {
            return Err(BlockError::InvalidBufferLength);
        }
        if block >= self.block_count() {
            return Err(BlockError::OutOfBounds);
        }
        self.inner.write_block(block, buffer)
    }
}

#[test]
fn length_priority_covers_first_out_of_bounds_block() {
    // 长度优先探针必须包含首个越界块 count：只对块 0 与巨大块号
    // 先查长度的实现不能绕过优先级约定。
    let mut storage = [0x33; BLOCK_SIZE * 2];
    let mut disk = BlockCountFirstDisk {
        inner: MemoryDisk::new(&mut storage).unwrap(),
    };
    assert!(check_buffer_length_validation(&mut disk).is_err());
}

/// 空盘设备：首次非空非法长度读取向缓冲区写垃圾，巨大块号的非法
/// 读取恢复哨兵值，其余不触碰——恢复恰好发生在循环末次读取。
struct EmptyRestoreAtMaxDisk {
    junked: bool,
}

impl BlockDevice for EmptyRestoreAtMaxDisk {
    fn block_count(&self) -> u64 {
        0
    }
    fn read_block(&mut self, block: u64, buffer: &mut [u8]) -> Result<(), BlockError> {
        if buffer.len() != BLOCK_SIZE {
            if !buffer.is_empty() {
                if block == u64::MAX && self.junked {
                    buffer.fill(0x77);
                    self.junked = false;
                } else if !self.junked {
                    buffer.fill(0x11);
                    self.junked = true;
                }
            }
            return Err(BlockError::InvalidBufferLength);
        }
        Err(BlockError::OutOfBounds)
    }
    fn write_block(&mut self, _block: u64, buffer: &[u8]) -> Result<(), BlockError> {
        if buffer.len() != BLOCK_SIZE {
            return Err(BlockError::InvalidBufferLength);
        }
        Err(BlockError::OutOfBounds)
    }
}

#[test]
fn empty_device_checks_buffer_after_each_rejected_read() {
    // 探针缓冲区的哨兵比对必须在每次非法读取返回后立即进行：
    // 块 0 写垃圾、块 1 恢复的翻转后端不能等到循环结束才被发现。
    let mut disk = EmptyRestoreAtMaxDisk { junked: false };
    assert!(check_empty_device(&mut disk).is_err());
}

/// 非合规设备：首次成功写入时正确落盘目标块，同时把尚未初始化的
/// 块 2 改坏（随后块 2 的基线写入会治愈它）。
struct FirstWriteClobbersSuffixDisk<'a> {
    inner: MemoryDisk<'a>,
    armed: bool,
}

impl BlockDevice for FirstWriteClobbersSuffixDisk<'_> {
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
    fn read_block(&mut self, block: u64, buffer: &mut [u8]) -> Result<(), BlockError> {
        self.inner.read_block(block, buffer)
    }
    fn write_block(&mut self, block: u64, buffer: &[u8]) -> Result<(), BlockError> {
        self.inner.write_block(block, buffer)?;
        if self.armed {
            self.armed = false;
            self.inner.write_block(2, &[0xEE; BLOCK_SIZE])?;
        }
        Ok(())
    }
}

#[test]
fn baseline_suffix_snapshot_catches_first_write_clobber() {
    // 前缀复核排除了未写后缀：首写对未初始化块的破坏必须靠基线前的
    // 整盘快照在治愈前暴露。
    let mut storage = [0x33; BLOCK_SIZE * 3];
    let mut disk = FirstWriteClobbersSuffixDisk {
        inner: MemoryDisk::new(&mut storage).unwrap(),
        armed: true,
    };
    assert!(check_out_of_bounds_rejected(&mut disk).is_err());
}

/// 非合规设备：首次写块 0 正确落盘并破坏块 1，随后的 flush 修复块 1
/// （同步治愈一次性串扰）。
struct FlushRepairsClobberDisk<'a> {
    inner: MemoryDisk<'a>,
    armed: bool,
    saved: Option<[u8; BLOCK_SIZE]>,
}

impl BlockDevice for FlushRepairsClobberDisk<'_> {
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
    fn read_block(&mut self, block: u64, buffer: &mut [u8]) -> Result<(), BlockError> {
        self.inner.read_block(block, buffer)
    }
    fn write_block(&mut self, block: u64, buffer: &[u8]) -> Result<(), BlockError> {
        self.inner.write_block(block, buffer)?;
        if block == 0 && self.armed {
            self.armed = false;
            let mut current = [0u8; BLOCK_SIZE];
            self.inner.read_block(1, &mut current)?;
            self.saved = Some(current);
            self.inner.write_block(1, &[0xEE; BLOCK_SIZE])?;
        }
        Ok(())
    }
    fn flush(&mut self) -> Result<(), BlockError> {
        self.inner.flush()?;
        if let Some(saved) = self.saved.take() {
            self.inner.write_block(1, &saved)?;
        }
        Ok(())
    }
}

#[test]
fn flush_check_verifies_state_before_sync() {
    // 写入与同步之间必须整盘复核：被 flush 治愈的一次性串扰要在
    // 同步发生前暴露。
    let mut storage = [0x33; BLOCK_SIZE * 2];
    let mut disk = FlushRepairsClobberDisk {
        inner: MemoryDisk::new(&mut storage).unwrap(),
        armed: true,
        saved: None,
    };
    assert!(check_flush_ok(&mut disk).is_err());
}

/// 非合规设备：读取块 2 时先正确填充缓冲区，随后一次性破坏块 1。
struct BaselineReadTwoClobbersOneDisk<'a> {
    inner: MemoryDisk<'a>,
    reads_of_two: u32,
}

impl BlockDevice for BaselineReadTwoClobbersOneDisk<'_> {
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
    fn read_block(&mut self, block: u64, buffer: &mut [u8]) -> Result<(), BlockError> {
        self.inner.read_block(block, buffer)?;
        if block == 2 {
            self.reads_of_two += 1;
            if self.reads_of_two == 2 {
                self.inner.write_block(1, &[0xEE; BLOCK_SIZE])?;
            }
        }
        Ok(())
    }
    fn write_block(&mut self, block: u64, buffer: &[u8]) -> Result<(), BlockError> {
        self.inner.write_block(block, buffer)
    }
}

#[test]
fn baseline_suffix_recheck_reads_in_reverse() {
    // 后缀快照复核升序进行时，"读块 2 破坏块 1（仅一次）"发生在块 1
    // 比对之后，下一基线写入将其治愈；逆序复核使最低块的比对成为
    // 最后一次读取，破坏即被检出。
    let mut storage = [0x33; BLOCK_SIZE * 3];
    let mut disk = BaselineReadTwoClobbersOneDisk {
        inner: MemoryDisk::new(&mut storage).unwrap(),
        reads_of_two: 0,
    };
    assert!(check_out_of_bounds_rejected(&mut disk).is_err());
}

/// 非合规设备：首次写入后，读取块 1 时先正确填充缓冲区、随后破坏块 2。
struct ReadOneClobbersTwoAfterFirstWriteDisk<'a> {
    inner: MemoryDisk<'a>,
    first_write_done: bool,
    clobbered: bool,
}

impl BlockDevice for ReadOneClobbersTwoAfterFirstWriteDisk<'_> {
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
    fn read_block(&mut self, block: u64, buffer: &mut [u8]) -> Result<(), BlockError> {
        self.inner.read_block(block, buffer)?;
        if block == 1 && self.first_write_done && !self.clobbered {
            self.clobbered = true;
            self.inner.write_block(2, &[0xEE; BLOCK_SIZE])?;
        }
        Ok(())
    }
    fn write_block(&mut self, block: u64, buffer: &[u8]) -> Result<(), BlockError> {
        self.inner.write_block(block, buffer)?;
        if block == 0 {
            self.first_write_done = true;
        }
        Ok(())
    }
}

#[test]
fn write_isolation_rechecks_others_in_both_directions() {
    // 纯降序复核后，"读最低块破坏较高块"会落在最高块比对之后逃过；
    // 正反两遍复核任一方向的读取破坏都在反向遍历中暴露。
    let mut storage = [0x33; BLOCK_SIZE * 3];
    let mut disk = ReadOneClobbersTwoAfterFirstWriteDisk {
        inner: MemoryDisk::new(&mut storage).unwrap(),
        first_write_done: false,
        clobbered: false,
    };
    assert!(check_write_isolation(&mut disk).is_err());
}

/// 非合规设备：非法长度仅对块 0 与越界块优先校验，有效非零块号
/// 却返回设备错误。
struct ValidBlockIoDisk<'a> {
    inner: MemoryDisk<'a>,
}

impl BlockDevice for ValidBlockIoDisk<'_> {
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
    fn read_block(&mut self, block: u64, buffer: &mut [u8]) -> Result<(), BlockError> {
        if buffer.len() != BLOCK_SIZE {
            if block == 0 || block >= self.block_count() {
                return Err(BlockError::InvalidBufferLength);
            }
            return Err(BlockError::Io);
        }
        self.inner.read_block(block, buffer)
    }
    fn write_block(&mut self, block: u64, buffer: &[u8]) -> Result<(), BlockError> {
        if buffer.len() != BLOCK_SIZE {
            if block == 0 || block >= self.block_count() {
                return Err(BlockError::InvalidBufferLength);
            }
            return Err(BlockError::Io);
        }
        self.inner.write_block(block, buffer)
    }
}

#[test]
fn length_priority_covers_valid_nonzero_block() {
    // 长度优先探针必须包含有效非零块：非法长度命中有效块时返回
    // 设备错误（而非长度错误）的实现不能绕过优先级约定。
    let mut storage = [0x33; BLOCK_SIZE * 2];
    let mut disk = ValidBlockIoDisk {
        inner: MemoryDisk::new(&mut storage).unwrap(),
    };
    assert!(check_buffer_length_validation(&mut disk).is_err());
}

/// 非合规设备：读取只把传入缓冲区逐字节取反，不访问介质。
struct ComplementOnlyReadDisk<'a> {
    inner: MemoryDisk<'a>,
}

impl BlockDevice for ComplementOnlyReadDisk<'_> {
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
    fn read_block(&mut self, block: u64, buffer: &mut [u8]) -> Result<(), BlockError> {
        if block >= self.block_count() {
            return Err(BlockError::OutOfBounds);
        }
        if buffer.len() != BLOCK_SIZE {
            return Err(BlockError::InvalidBufferLength);
        }
        for byte in buffer.iter_mut() {
            *byte = !*byte;
        }
        Ok(())
    }
    fn write_block(&mut self, block: u64, buffer: &[u8]) -> Result<(), BlockError> {
        self.inner.write_block(block, buffer)
    }
}

#[test]
fn known_data_rejects_buffer_transform_reads() {
    // 只对缓冲区做固定变换（逐字节取反）的假后端能命中单一取反预填；
    // 两种不同预填各读一次，变换无法同时命中两种期望。
    let mut storage = [0u8; BLOCK_SIZE];
    let mut disk = ComplementOnlyReadDisk {
        inner: MemoryDisk::new(&mut storage).unwrap(),
    };
    let expected = [0u8; BLOCK_SIZE];
    assert!(check_known_data(&mut disk, 0, &expected).is_err());
}

/// 非合规设备：首次写入后，块 1 的第二次读取先正确填充缓冲区、
/// 随后破坏块 2。
struct SecondReadOneClobbersTwoDisk<'a> {
    inner: MemoryDisk<'a>,
    first_write_done: bool,
    reads_of_one: u32,
}

impl BlockDevice for SecondReadOneClobbersTwoDisk<'_> {
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
    fn read_block(&mut self, block: u64, buffer: &mut [u8]) -> Result<(), BlockError> {
        self.inner.read_block(block, buffer)?;
        if block == 1 && self.first_write_done {
            self.reads_of_one += 1;
            if self.reads_of_one == 2 {
                self.inner.write_block(2, &[0xEE; BLOCK_SIZE])?;
            }
        }
        Ok(())
    }
    fn write_block(&mut self, block: u64, buffer: &[u8]) -> Result<(), BlockError> {
        self.inner.write_block(block, buffer)?;
        if block == 0 {
            self.first_write_done = true;
        }
        Ok(())
    }
}

#[test]
fn write_isolation_probe_covers_second_read_clobber() {
    // 正反两遍复核后，副作用落在第二遍最后一次读取（降序遍历的块 1）
    // 仍会残留：写入基线前的后缀对照会把破坏暴露在治愈之前。
    let mut storage = [0x33; BLOCK_SIZE * 3];
    let mut disk = SecondReadOneClobbersTwoDisk {
        inner: MemoryDisk::new(&mut storage).unwrap(),
        first_write_done: false,
        reads_of_one: 0,
    };
    assert!(check_write_isolation(&mut disk).is_err());
}
