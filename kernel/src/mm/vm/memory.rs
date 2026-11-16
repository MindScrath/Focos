//! 将 MM-001 的地址分配适配为 MM-003 独占页句柄，不新建页池。
use super::*;
use crate::mm::PageAllocator;

/// 真实内存页；字段私有，不复制、不自动归还。
pub struct IdentityPage {
    address: PhysAddr,
    origin: usize,
}
/// 分配器的唯一所有者；只在整个管理范围保持恒等可访问时使用。
pub struct IdentityMemory<'a> {
    allocator: PageAllocator<'a>,
    origin: usize,
}
impl<'a> IdentityMemory<'a> {
    /// 接管已有分配器，保留全部占用标记。
    ///
    /// # Safety
    /// 管理范围为按 PAGE_SIZE 对齐的独占 RAM，非空页地址可完整读写；初始内容可未初始化。
    /// 这一保证覆盖所有页句柄的持有期，包括服务被销毁以后；不得回收底层存储，
    /// 不得重复初始化管理仍被持有页的分配器，也不得改变这些页的恒等访问路径。
    /// 页内容不与标记等元数据重叠；开发机用裸地址适配时须保留指针来源信息。
    pub unsafe fn new(allocator: PageAllocator<'a>) -> Self {
        let origin = allocator.used.as_ptr().addr();
        Self { allocator, origin }
    }
}
// SAFETY: 私有句柄只能来自已校验的服务。RAM 生命周期不依赖服务，闭包期间借用整页。
unsafe impl VmPage for IdentityPage {
    fn phys_addr(&self) -> PhysAddr {
        self.address
    }
    fn with_bytes<R>(&self, f: impl FnOnce(&[u8; PAGE_SIZE]) -> R) -> R {
        let pointer = core::ptr::with_exposed_provenance::<[u8; PAGE_SIZE]>(self.address.0);
        // SAFETY: new 的调用者保证持有期内恒等可读，访问别名必须分时。
        f(unsafe { &*pointer })
    }
    fn with_bytes_mut<R>(&mut self, f: impl FnOnce(&mut [u8; PAGE_SIZE]) -> R) -> R {
        let pointer = core::ptr::with_exposed_provenance_mut::<[u8; PAGE_SIZE]>(self.address.0);
        // SAFETY: 当前独占句柄，调用协议禁止同时使用虚拟别名。
        f(unsafe { &mut *pointer })
    }
}
// SAFETY: MM-001 保证地址唯一，私有来源标识绑定标记存储；归还失败不消耗句柄。
unsafe impl VmMemory for IdentityMemory<'_> {
    type Page = IdentityPage;
    fn managed_range(&self) -> Range<PhysAddr> {
        PhysAddr(self.allocator.start)..PhysAddr(self.allocator.end)
    }
    fn alloc_zeroed_page(&mut self) -> Result<IdentityPage, PageAllocError> {
        let address = PhysAddr(self.allocator.alloc()?);
        let pointer = core::ptr::with_exposed_provenance_mut::<u8>(address.0);
        // SAFETY: new 保证整页可写且独占，MM-001 刚将其标为已占用。
        // 在任何 Rust 字节引用形成前初始化整页，不要求分配到的 RAM 已有有效内容。
        unsafe {
            pointer.write_bytes(0, PAGE_SIZE);
        }
        Ok(IdentityPage {
            address,
            origin: self.origin,
        })
    }
    unsafe fn release_page(
        &mut self,
        page: IdentityPage,
    ) -> Result<(), (PageAllocError, IdentityPage)> {
        if page.origin != self.origin {
            return Err((PageAllocError::InvalidAddress, page));
        }
        match self.allocator.dealloc(page.address.0) {
            Ok(()) => Ok(()),
            Err(error) => Err((error, page)),
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::mm::vm::tests::Buffer;
    use std::boxed::Box;

    #[test]
    fn identity_adapter_initializes_raw_storage_before_page_access() {
        let mut storage = Box::<Buffer>::new_uninit();
        let start = storage.as_mut_ptr().expose_provenance();
        let mut flags = [false];
        {
            let allocator = PageAllocator::new(start, start + PAGE_SIZE, &mut flags).unwrap();
            // SAFETY: 未初始化的对齐存储可写，来源已暴露；所有句柄结束前不访问或销毁 Box。
            let mut memory = unsafe { IdentityMemory::new(allocator) };
            let page = memory.alloc_zeroed_page().unwrap();
            page.with_bytes(|bytes| assert!(bytes.iter().all(|byte| *byte == 0)));
            // SAFETY: 无映射、无页面引用；归还后才核对底层存储。
            assert!(unsafe { memory.release_page(page) }.is_ok());
        }
        // SAFETY: 分配适配已初始化完整 Buffer，归还不修改内容且没有活跃句柄。
        let storage = unsafe { storage.assume_init() };
        assert!(storage.0.iter().all(|byte| *byte == 0));
    }

    #[test]
    fn identity_adapter_zeros_reuses_and_preserves_handle_on_rejected_release() {
        let mut storage = Box::new(Buffer([0x91; 4096]));
        let start = storage.0.as_mut_ptr().expose_provenance();
        let mut flags = [false];
        let mut wrong_flags = [false];
        let allocator = PageAllocator::new(start, start + 4096, &mut flags).unwrap();
        // SAFETY: Box 是完整对齐且独占的真实缓冲区，存活至所有页归还以后。
        let mut memory = unsafe { IdentityMemory::new(allocator) };
        let mut page = memory.alloc_zeroed_page().unwrap();
        page.with_bytes(|bytes| assert!(bytes.iter().all(|b| *b == 0)));
        page.with_bytes_mut(|bytes| bytes.fill(0x42));
        // 空服务不会访问同一 RAM，只用于错误来源检测。
        let allocator = PageAllocator::new(0, 0, &mut wrong_flags).unwrap();
        // SAFETY: 空管理范围，不会形成重叠的页句柄。
        let mut wrong = unsafe { IdentityMemory::new(allocator) };
        // SAFETY: 没有映射或保留引用；错误来源必须保留句柄。
        let (error, page) = unsafe { wrong.release_page(page) }.err().unwrap();
        assert_eq!(error, PageAllocError::InvalidAddress);
        page.with_bytes(|bytes| assert!(bytes.iter().all(|b| *b == 0x42)));
        // 注入标记损坏，验证 NotAllocated 也返回原句柄且不修改内容。
        memory.allocator.dealloc(start).unwrap();
        // SAFETY: 仅测试标记校验，无硬件映射，缓冲区仍有效。
        let (error, page) = unsafe { memory.release_page(page) }.err().unwrap();
        assert_eq!(error, PageAllocError::NotAllocated);
        page.with_bytes(|bytes| assert!(bytes.iter().all(|b| *b == 0x42)));
        assert_eq!(memory.allocator.alloc(), Ok(start));
        // SAFETY: 恢复原占用标记后正常归还，无别名。
        assert!(unsafe { memory.release_page(page) }.is_ok());
        let page = memory.alloc_zeroed_page().unwrap();
        page.with_bytes(|bytes| assert!(bytes.iter().all(|b| *b == 0)));
        // SAFETY: 无映射和引用，结束后才销毁底层 Box。
        assert!(unsafe { memory.release_page(page) }.is_ok());
    }
}
