//! ARCH-002：QEMU 固件设备树中的时间基频，不借用运行期设备树。
use core::num::NonZeroU32;

/// 板级设备树读取失败；不映射为 HAL 时钟错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimebaseError {
    InvalidRange,
    MalformedDtb,
    UnsupportedVersion,
    MissingFrequency,
    InvalidFrequency,
}

const HEADER_LEN: usize = 40;
use TimebaseError::MalformedDtb;

/// 每个范围先受检相加，再切片；不以不可信的偏移构造指针。
fn take(bytes: &[u8], start: usize, len: usize) -> Result<&[u8], TimebaseError> {
    bytes
        .get(start..start.checked_add(len).ok_or(MalformedDtb)?)
        .ok_or(MalformedDtb)
}
fn word(bytes: &[u8], start: usize) -> Result<u32, TimebaseError> {
    Ok(u32::from_be_bytes(
        take(bytes, start, 4)?
            .try_into()
            .map_err(|_| MalformedDtb)?,
    ))
}
fn terminated(bytes: &[u8], start: usize) -> Result<&[u8], TimebaseError> {
    let tail = bytes.get(start..).ok_or(MalformedDtb)?;
    let len = tail
        .iter()
        .position(|byte| *byte == 0)
        .ok_or(MalformedDtb)?;
    Ok(&tail[..len])
}
fn align4(value: usize) -> Result<usize, TimebaseError> {
    Ok(value.checked_add(3).ok_or(MalformedDtb)? & !3)
}

/// 从有界 DTB 中读取唯一的 /cpus/timebase-frequency，完整校验结构后才返回。
/// 接受与格式版本 17 兼容的头部；输入尾部可以包含 totalsize 之外的字节。
pub fn parse_timebase_frequency(bytes: &[u8]) -> Result<NonZeroU32, TimebaseError> {
    take(bytes, 0, HEADER_LEN)?;
    if word(bytes, 0)? != 0xd00d_feed {
        return Err(MalformedDtb);
    }
    let total = word(bytes, 4)? as usize;
    if total < HEADER_LEN {
        return Err(MalformedDtb);
    }
    let bytes = take(bytes, 0, total)?;
    if word(bytes, 20)? < 17 || word(bytes, 24)? > 17 {
        return Err(TimebaseError::UnsupportedVersion);
    }
    let structure_start = word(bytes, 8)? as usize;
    let strings_start = word(bytes, 12)? as usize;
    let reserve_start = word(bytes, 16)? as usize;
    let strings_len = word(bytes, 32)? as usize;
    let structure_len = word(bytes, 36)? as usize;
    let structure = take(bytes, structure_start, structure_len)?;
    let strings = take(bytes, strings_start, strings_len)?;
    // 固定 QEMU 路径采用规范的头部、保留表、结构、字符串顺序。
    if reserve_start < HEADER_LEN
        || !reserve_start.is_multiple_of(8)
        || structure_start < reserve_start
        || !structure_start.is_multiple_of(4)
        || structure_start
            .checked_add(structure_len)
            .ok_or(MalformedDtb)?
            > strings_start
    {
        return Err(MalformedDtb);
    }
    let reserves = bytes
        .get(reserve_start..structure_start)
        .ok_or(MalformedDtb)?;
    let mut cursor = 0;
    loop {
        let pair = take(reserves, cursor, 16)?;
        cursor = cursor.checked_add(16).ok_or(MalformedDtb)?;
        if pair.iter().all(|v| *v == 0) {
            break;
        }
    }
    parse_structure(structure, strings)
}

/// 常量额外空间遍历；位置每步前进且受结构块长度约束，无递归或堆分配。
fn parse_structure(bytes: &[u8], strings: &[u8]) -> Result<NonZeroU32, TimebaseError> {
    let mut cursor = 0usize;
    let mut depth = 0usize;
    let mut root_seen = false;
    let mut cpus_seen = false;
    let mut in_cpus = false;
    let mut properties_allowed = false;
    let mut frequency = None;
    loop {
        let token = word(bytes, cursor)?;
        cursor = cursor.checked_add(4).ok_or(MalformedDtb)?;
        match token {
            1 => {
                let name = terminated(bytes, cursor)?;
                if name.contains(&b'/') {
                    return Err(MalformedDtb);
                }
                if depth == 0 {
                    if root_seen || !name.is_empty() {
                        return Err(MalformedDtb);
                    }
                    root_seen = true;
                } else if name.is_empty() {
                    return Err(MalformedDtb);
                }
                if depth == 1 && name == b"cpus" {
                    if cpus_seen {
                        return Err(MalformedDtb);
                    }
                    cpus_seen = true;
                    in_cpus = true;
                }
                let end = cursor
                    .checked_add(name.len())
                    .and_then(|v| v.checked_add(1))
                    .ok_or(MalformedDtb)?;
                cursor = align4(end)?;
                // 固件可在对齐填充中保留任意字节；只验证这些字节仍在块内。
                take(bytes, end, cursor - end)?;
                depth = depth.checked_add(1).ok_or(MalformedDtb)?;
                properties_allowed = true;
            }
            2 => {
                if depth == 0 {
                    return Err(MalformedDtb);
                }
                if depth == 2 {
                    in_cpus = false;
                }
                depth -= 1;
                properties_allowed = false;
            }
            3 => {
                if depth == 0 || !properties_allowed {
                    return Err(MalformedDtb);
                }
                let len = word(bytes, cursor)? as usize;
                let name_offset = word(bytes, cursor.checked_add(4).ok_or(MalformedDtb)?)? as usize;
                cursor = cursor.checked_add(8).ok_or(MalformedDtb)?;
                let value = take(bytes, cursor, len)?;
                let name = terminated(strings, name_offset)?;
                if name.is_empty() {
                    return Err(MalformedDtb);
                }
                if in_cpus && depth == 2 && name == b"timebase-frequency" {
                    if frequency.is_some() {
                        return Err(MalformedDtb);
                    }
                    // 先记录结果，完整遍历成功后才向调用方交付，尾部损坏不能被忽略。
                    frequency = Some(if len == 4 {
                        NonZeroU32::new(word(value, 0)?).ok_or(TimebaseError::InvalidFrequency)
                    } else {
                        Err(TimebaseError::InvalidFrequency)
                    });
                }
                let end = cursor.checked_add(len).ok_or(MalformedDtb)?;
                cursor = align4(end)?;
                // 固件可在对齐填充中保留任意字节；只验证这些字节仍在块内。
                take(bytes, end, cursor - end)?;
            }
            4 => {}
            9 if root_seen && depth == 0 && cursor == bytes.len() => {
                return frequency.unwrap_or(Err(TimebaseError::MissingFrequency));
            }
            _ => return Err(MalformedDtb),
        }
    }
}

/// 裸地址验证独立于读内存；保留头部或整树前先证明上界与加法有效。
fn checked_boot_range(
    kernel_end: usize,
    dtb_start: usize,
    len: usize,
) -> Result<(), TimebaseError> {
    super::memory::usable_range(kernel_end, dtb_start).map_err(|_| TimebaseError::InvalidRange)?;
    if len < HEADER_LEN
        || dtb_start
            .checked_add(len)
            .is_none_or(|end| end > super::memory::RAM_END)
    {
        return Err(TimebaseError::InvalidRange);
    }
    Ok(())
}

/// 在任何读取发生前验证范围。注入有界读操作供 host 核对实际访问顺序。
/// 不保存回调或返回引用；真实入口只在下面的闭包中构造借用。
fn read_boot_frequency<'a>(
    kernel_end: usize,
    dtb_start: usize,
    mut read: impl FnMut(usize, usize) -> &'a [u8],
) -> Result<NonZeroU32, TimebaseError> {
    checked_boot_range(kernel_end, dtb_start, HEADER_LEN)?;
    let header = read(dtb_start, HEADER_LEN);
    if word(header, 0)? != 0xd00d_feed {
        return Err(MalformedDtb);
    }
    let total = word(header, 4)? as usize;
    if total < HEADER_LEN {
        return Err(MalformedDtb);
    }
    checked_boot_range(kernel_end, dtb_start, total)?;
    parse_timebase_frequency(read(dtb_start, total))
}

/// 分页前读取固件 DTB，只返回频率值；不保留设备树指针或引用。
///
/// # Safety
/// 仅唯一启动 hart、SIE=0、分页关闭时调用，使用 MM-002 的固定 QEMU 配置。
/// 地址来自当前固件。完整头部，以及头部总长通过范围检查后声明的全部字节
/// （含对齐填充），必须属于同一连续、已初始化且可读的固件存储，调用期间无写者。
/// 检查只证明地址、长度和 RAM 边界，不证明该存储或其已初始化状态。
/// 不可将任意地址或仅头部有效的缓冲区作为固件 DTB 传入；不可信字节用安全解析入口。
#[cfg(focos_platform = "qemu_riscv64")]
pub unsafe fn boot_timebase_frequency(dtb_start: usize) -> Result<NonZeroU32, TimebaseError> {
    unsafe extern "C" {
        static __kernel_end: u8;
    }
    let kernel_end = (&raw const __kernel_end).addr();
    read_boot_frequency(kernel_end, dtb_start, |address, len| {
        // SAFETY: read_boot_frequency 在每次回调前检查完整范围；地址非零、8 字节对齐，
        // 128 MiB RAM 边界同时保证长度小于 isize::MAX、加法不绕回。
        // 调用方保证 Bare、同一连续固件存储、全部字节已初始化且无写者；
        // MM-002 已将此范围排除在分配器外。借用只在本次解析内使用，不向外返回。
        unsafe { core::slice::from_raw_parts(address as *const u8, len) }
    })
}

#[cfg(test)]
mod tests;
