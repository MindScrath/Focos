//! 用开发机真实分配的对齐缓冲区验证自检与清理，不解引用平台物理地址。

use super::*;

#[repr(align(4096))]
struct Pages([u8; 3 * PAGE_SIZE]);

/// 自检结束后归还两页，同时保留调用者已经持有的页及其内容。
#[test]
fn self_test_returns_pages_without_touching_existing_allocations() {
    let mut storage = Pages([0xa5; 3 * PAGE_SIZE]);
    let start = storage.0.as_mut_ptr() as usize;
    let mut used = [false; 3];
    {
        let mut allocator = PageAllocator::new(start, start + storage.0.len(), &mut used).unwrap();
        let held = allocator.alloc().unwrap();
        // SAFETY: 空闲页来自此对齐缓冲区，自检独占访问；缓冲区在归还所有页前始终存活。
        unsafe { check_pages(&mut allocator) }.unwrap();
        let offset = held - start;
        assert!(
            storage.0[offset..offset + PAGE_SIZE]
                .iter()
                .all(|&byte| byte == 0xa5)
        );
        let first = allocator.alloc().unwrap();
        let second = allocator.alloc().unwrap();
        assert_ne!(first, second);
        assert_ne!(held, first);
        assert_ne!(held, second);
        assert_eq!(allocator.alloc(), Err(PageAllocError::OutOfMemory));
        for page in [held, first, second] {
            allocator.dealloc(page).unwrap();
        }
    }
    assert_eq!(used, [false; 3]);
}

/// 第二页申请失败时，先取得的页必须归还，且不能提前改写页内容。
#[test]
fn insufficient_pages_do_not_leak_or_modify_memory() {
    let mut storage = Pages([0x5a; 3 * PAGE_SIZE]);
    let start = storage.0.as_mut_ptr() as usize;
    let mut used = [false];
    {
        let mut allocator = PageAllocator::new(start, start + PAGE_SIZE, &mut used).unwrap();
        // SAFETY: 分配范围是此缓冲区的第一页，调用期间没有其他访问者。
        assert_eq!(
            unsafe { check_pages(&mut allocator) },
            Err(PageTestError::Allocator(PageAllocError::OutOfMemory))
        );
        assert_eq!(allocator.alloc(), Ok(start));
        allocator.dealloc(start).unwrap();
    }
    assert_eq!(used, [false]);
    assert_eq!(storage.0, [0x5a; 3 * PAGE_SIZE]);
}

/// 页末字节损坏也必须被检查到，并报告准确地址及期望值。
#[test]
fn readback_checks_the_last_byte() {
    let mut storage = Pages([0; 3 * PAGE_SIZE]);
    for (offset, byte) in storage.0[..PAGE_SIZE].iter_mut().enumerate() {
        *byte = 0x35u8.wrapping_add(offset as u8);
    }
    let expected = storage.0[PAGE_SIZE - 1];
    storage.0[PAGE_SIZE - 1] ^= 1;
    let start = storage.0.as_mut_ptr() as usize;
    // SAFETY: 第一页已经完整写入，缓冲区仍存活，读回期间无人修改。
    assert_eq!(
        unsafe { verify_page(start, 0x35) },
        Err(PageTestError::ReadbackMismatch {
            address: start + PAGE_SIZE - 1,
            expected,
            actual: expected ^ 1,
        })
    );
}
