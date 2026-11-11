//! MM-002 的启动页读写检查；页分配器本身仍只管理地址和标记。

use super::{PAGE_SIZE, PageAllocError, PageAllocator};

/// 页读写、自检所需的分配或回收失败的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageTestError {
    /// 分配或回收失败，保留原始错误。
    Allocator(PageAllocError),
    /// 两次分配错误地返回了同一个仍在使用的页。
    AliasedPages,
    /// 实际读回的某个字节与写入图样不同。
    ReadbackMismatch {
        address: usize,
        expected: u8,
        actual: u8,
    },
}

impl From<PageAllocError> for PageTestError {
    /// 保留分配或回收错误，使启动日志能够报告实际失败原因。
    fn from(error: PageAllocError) -> Self {
        Self::Allocator(error)
    }
}

/// 申请两页并验证整页写读、释放后再申请及另一页内容保持，最后归还测试页。
/// 发生错误时也尝试归还所有已取得的页，不保留页指针或引用。
///
/// # Safety
/// 分配器中的空闲页必须对应互不重叠、可读写的真实内存；调用者保证自检独占访问。
/// 自检期间不得并发分配、释放或访问这些页，且地址映射必须保持有效。
pub unsafe fn check_pages(allocator: &mut PageAllocator<'_>) -> Result<(), PageTestError> {
    let mut pages = [None; 2];
    let mut result = (|| {
        let first = allocator.alloc()?;
        pages[0] = Some(first);
        let second = allocator.alloc()?;
        if first == second {
            return Err(PageTestError::AliasedPages);
        }
        pages[1] = Some(second);

        // SAFETY: 两页均已成功分配且地址不同；调用者保证映射有效和独占访问。
        // 写满后再读回，每次辅助函数返回都不留下指针或引用。
        unsafe {
            write_page(first, 0x35);
            write_page(second, 0xa7);
            verify_page(first, 0x35)?;
            verify_page(second, 0xa7)?;
        }

        allocator.dealloc(first)?;
        pages[0] = None;
        let reused = allocator.alloc()?;
        if reused == second {
            return Err(PageTestError::AliasedPages);
        }
        pages[0] = Some(reused);

        // SAFETY: reused 是新申请的有效页，不再使用已释放的 first 地址。
        // second 始终持有，检查它可发现新页写入错误地覆盖了仍在使用的页。
        unsafe {
            write_page(reused, 0x6c);
            verify_page(reused, 0x6c)?;
            verify_page(second, 0xa7)?;
        }
        Ok(())
    })();

    for address in pages.into_iter().flatten() {
        if let Err(error) = allocator.dealloc(address) {
            result = Err(error.into());
        }
    }
    result
}

/// 用随页内偏移变化的字节图样写满一页，不依赖页的初始内容。
///
/// # Safety
/// `address` 起的完整页须由调用者独占持有且可写，地址加页大小不能溢出。
unsafe fn write_page(address: usize, seed: u8) {
    for offset in 0..PAGE_SIZE {
        let pointer = core::ptr::with_exposed_provenance_mut::<u8>(address + offset);
        // SAFETY: 调用者保证完整页有效且独占；offset 始终在页内，u8 无额外对齐要求。
        // volatile 保证真实写入，不作为并发同步手段。
        unsafe { pointer.write_volatile(seed.wrapping_add(offset as u8)) };
    }
}

/// 逐字节读回一整页，返回首个不符合图样的位置。
///
/// # Safety
/// `address` 起的完整页须仍由调用者持有、已写入且可读，地址加页大小不能溢出。
/// 读回期间不能被修改或释放。
unsafe fn verify_page(address: usize, seed: u8) -> Result<(), PageTestError> {
    for offset in 0..PAGE_SIZE {
        let pointer = core::ptr::with_exposed_provenance::<u8>(address + offset);
        // SAFETY: 此页在写入后仍被持有；地址在页内，u8 的所有位模式均有效。
        // volatile 要求实际读取每个字节，不能复用编译器已知的写入值。
        let actual = unsafe { pointer.read_volatile() };
        let expected = seed.wrapping_add(offset as u8);
        if actual != expected {
            return Err(PageTestError::ReadbackMismatch {
                address: address + offset,
                expected,
                actual,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
