//! MM-001 的开发机行为测试。地址只作数值使用，不解引用物理内存。

use super::{PageAllocError, PageAllocator};

// 使用 SPEC 的固定页大小，避免测试与实现同时采用错误的常量。
const PAGE_SIZE: usize = 4096;
const START: usize = 0x1000;
const END: usize = 0x4000;

fn allocate_three_pages(allocator: &mut PageAllocator<'_>) -> [usize; 3] {
    [
        allocator.alloc().unwrap(),
        allocator.alloc().unwrap(),
        allocator.alloc().unwrap(),
    ]
}

/// 比较完整地址集合，同时检查对齐、范围与唯一性，不限制分配顺序。
fn assert_managed_pages(mut pages: [usize; 3]) {
    pages.sort_unstable();
    assert_eq!(pages, [START, START + PAGE_SIZE, END - PAGE_SIZE]);
}

/// 构造必须清除原有占用标记，但不能修改管理范围之外的存储。
#[test]
fn initializes_only_managed_flags() {
    let mut used = [true, true, true, false, true];
    {
        let _allocator = PageAllocator::new(START, END, &mut used).unwrap();
    }
    // 借用结束后检查调用者的数组，不读取分配器的私有字段。
    assert_eq!(used, [false, false, false, false, true]);
}

/// 单独验收空范围的构造，不依赖后续的分配和释放实现。
#[test]
fn accepts_empty_ranges_without_modifying_metadata() {
    for start in [0, START] {
        for length in [0, 2] {
            let original = [true, false];
            let mut used = original;
            {
                let _allocator = PageAllocator::new(start, start, &mut used[..length]).unwrap();
            }
            assert_eq!(used, original);
        }
    }
}

/// 反向范围和不对齐边界都必须失败，原标记保持不变。
#[test]
fn rejects_invalid_ranges_without_changing_flags() {
    for (start, end) in [
        (END, START),
        (START + 1, END),
        (START, END - 1),
        (START + 1, START + 1),
        (usize::MAX, usize::MAX),
    ] {
        let original = [true, false, true];
        let mut used = original;
        assert!(matches!(
            PageAllocator::new(start, end, &mut used),
            Err(PageAllocError::InvalidRange)
        ));
        assert_eq!(used, original);
    }
}

/// 范围和标记容量同时有错时，范围错误必须优先返回。
#[test]
fn checks_range_before_metadata_capacity() {
    for (start, end) in [(END, START), (START + 1, END), (START, END + 1)] {
        let original = [true];
        let mut used = original;
        assert!(matches!(
            PageAllocator::new(start, end, &mut used),
            Err(PageAllocError::InvalidRange)
        ));
        assert_eq!(used, original);
    }
}

/// 容量不足时不能先清除部分标记；也检查传入切片之外的存储。
#[test]
fn rejects_short_metadata_without_changing_flags() {
    for length in [0, 1, 2] {
        let original = [true, false, true];
        let mut used = original;
        assert!(matches!(
            PageAllocator::new(START, END, &mut used[..length]),
            Err(PageAllocError::MetadataTooSmall)
        ));
        assert_eq!(used, original);
    }
}

/// 巨大但合法的范围也应报告标记不足，不能因地址计算溢出而崩溃。
#[test]
fn rejects_huge_range_with_insufficient_metadata() {
    let end = usize::MAX & !(PAGE_SIZE - 1);
    let original = [true, false, true];
    let mut used = original;
    assert!(matches!(
        PageAllocator::new(0, end, &mut used),
        Err(PageAllocError::MetadataTooSmall)
    ));
    assert_eq!(used, original);
}

#[test]
fn allocates_all_managed_pages_once() {
    let mut used = [false; 3];
    {
        let mut allocator = PageAllocator::new(START, END, &mut used).unwrap();
        assert_managed_pages(allocate_three_pages(&mut allocator));
        assert_eq!(allocator.alloc(), Err(PageAllocError::OutOfMemory));
    }
    assert_eq!(used, [true; 3]);
}

/// 多余的空闲标记不能变成额外页，分配和释放也不能改动它们。
#[test]
fn allocation_and_release_preserve_extra_flags() {
    let mut used = [false, false, false, false, true];
    {
        let mut allocator = PageAllocator::new(START, END, &mut used).unwrap();
        let pages = allocate_three_pages(&mut allocator);
        assert_managed_pages(pages);
        assert_eq!(allocator.alloc(), Err(PageAllocError::OutOfMemory));
        allocator.dealloc(pages[1]).unwrap();
    }
    assert_eq!(used[3..], [false, true]);
}

/// 耗尽不能损坏已占用的页；只归还一页后，唯一可分配的就是该页。
#[test]
fn exhaustion_and_reuse_preserve_live_pages() {
    let mut used = [false; 3];
    let mut allocator = PageAllocator::new(START, END, &mut used).unwrap();
    let pages = allocate_three_pages(&mut allocator);
    assert_managed_pages(pages);
    for _ in 0..2 {
        assert_eq!(allocator.alloc(), Err(PageAllocError::OutOfMemory));
    }
    allocator.dealloc(pages[1]).unwrap();
    assert_eq!(allocator.alloc(), Ok(pages[1]));
    assert_eq!(allocator.alloc(), Err(PageAllocError::OutOfMemory));
    // 原本仍在使用的页也必须保持已分配状态，能够各自正常归还。
    for addr in pages {
        allocator.dealloc(addr).unwrap();
    }
    assert_managed_pages(allocate_three_pages(&mut allocator));
    assert_eq!(allocator.alloc(), Err(PageAllocError::OutOfMemory));
}

#[test]
fn rejects_double_free_without_creating_extra_capacity() {
    let mut used = [false; 3];
    let mut allocator = PageAllocator::new(START, END, &mut used).unwrap();
    let pages = allocate_three_pages(&mut allocator);
    assert_managed_pages(pages);
    allocator.dealloc(pages[0]).unwrap();
    assert_eq!(
        allocator.dealloc(pages[0]),
        Err(PageAllocError::NotAllocated)
    );
    assert_eq!(allocator.alloc(), Ok(pages[0]));
    assert_eq!(allocator.alloc(), Err(PageAllocError::OutOfMemory));
    allocator.dealloc(pages[1]).unwrap();
    allocator.dealloc(pages[2]).unwrap();
}

#[test]
fn rejects_never_allocated_page_without_changing_capacity() {
    let mut used = [false; 3];
    let mut allocator = PageAllocator::new(START, END, &mut used).unwrap();
    assert_eq!(
        allocator.dealloc(START + PAGE_SIZE),
        Err(PageAllocError::NotAllocated)
    );
    assert_managed_pages(allocate_three_pages(&mut allocator));
    assert_eq!(allocator.alloc(), Err(PageAllocError::OutOfMemory));
}

/// 在已有占用和仍有空闲的状态下，非法释放不能增加或减少容量。
#[test]
fn rejects_invalid_addresses_without_changing_allocations() {
    for addr in [
        0,
        START - 1,
        START + 1,
        END - 1,
        END,
        END + PAGE_SIZE,
        usize::MAX,
        usize::MAX & !(PAGE_SIZE - 1),
    ] {
        let mut used = [false; 3];
        let mut allocator = PageAllocator::new(START, END, &mut used).unwrap();
        let first = allocator.alloc().unwrap();
        assert_eq!(allocator.dealloc(addr), Err(PageAllocError::InvalidAddress));
        let pages = [
            first,
            allocator.alloc().unwrap(),
            allocator.alloc().unwrap(),
        ];
        assert_managed_pages(pages);
        assert_eq!(allocator.alloc(), Err(PageAllocError::OutOfMemory));
        for page in pages {
            allocator.dealloc(page).unwrap();
        }
    }
}

/// 即使范围内的页都空闲，非法地址也不能被误报为“尚未分配”。
#[test]
fn validates_address_before_allocation_state() {
    let mut used = [true; 3];
    {
        let mut allocator = PageAllocator::new(START, END, &mut used).unwrap();
        for addr in [0, START + 1, END, usize::MAX] {
            assert_eq!(allocator.dealloc(addr), Err(PageAllocError::InvalidAddress));
        }
    }
    assert_eq!(used, [false; 3]);
}

#[test]
fn allows_empty_ranges_without_changing_metadata() {
    for start in [0, START] {
        for length in [0, 2] {
            let original = [true, false];
            let mut used = original;
            {
                let mut allocator = PageAllocator::new(start, start, &mut used[..length]).unwrap();
                assert_eq!(allocator.alloc(), Err(PageAllocError::OutOfMemory));
                assert_eq!(allocator.alloc(), Err(PageAllocError::OutOfMemory));
                assert_eq!(
                    allocator.dealloc(start),
                    Err(PageAllocError::InvalidAddress)
                );
            }
            assert_eq!(used, original);
        }
    }
}

/// SPEC 允许从零开始的地址范围；地址零同样只作为数值验证。
#[test]
fn reuses_single_page_including_address_zero() {
    for start in [0, START] {
        let mut used = [true];
        {
            let mut allocator = PageAllocator::new(start, start + PAGE_SIZE, &mut used).unwrap();
            for _ in 0..16 {
                assert_eq!(allocator.alloc(), Ok(start));
                assert_eq!(allocator.alloc(), Err(PageAllocError::OutOfMemory));
                allocator.dealloc(start).unwrap();
                assert_eq!(allocator.dealloc(start), Err(PageAllocError::NotAllocated));
            }
        }
        assert_eq!(used, [false]);
    }
}

#[test]
fn repeated_cycles_restore_full_capacity() {
    let mut used = [false; 3];
    {
        let mut allocator = PageAllocator::new(START, END, &mut used).unwrap();
        for _ in 0..16 {
            let pages = allocate_three_pages(&mut allocator);
            assert_managed_pages(pages);
            assert_eq!(allocator.alloc(), Err(PageAllocError::OutOfMemory));
            // 归还顺序与申请顺序不同，避免测试依赖栈式回收。
            for addr in [pages[1], pages[2], pages[0]] {
                allocator.dealloc(addr).unwrap();
            }
        }
    }
    assert_eq!(used, [false; 3]);
}

/// 接近 usize 上界的合法范围仍可分配，且半开区间不包含 end。
#[test]
fn handles_high_aligned_addresses_without_overflow() {
    let end = usize::MAX & !(PAGE_SIZE - 1);
    let start = end - 3 * PAGE_SIZE;
    let mut used = [false; 3];
    let mut allocator = PageAllocator::new(start, end, &mut used).unwrap();
    let mut pages = allocate_three_pages(&mut allocator);
    pages.sort_unstable();
    assert_eq!(pages, [start, start + PAGE_SIZE, end - PAGE_SIZE]);
    assert_eq!(allocator.alloc(), Err(PageAllocError::OutOfMemory));
    for addr in [end, usize::MAX] {
        assert_eq!(allocator.dealloc(addr), Err(PageAllocError::InvalidAddress));
    }
    allocator.dealloc(pages[2]).unwrap();
    assert_eq!(allocator.alloc(), Ok(pages[2]));
    assert_eq!(allocator.alloc(), Err(PageAllocError::OutOfMemory));
}
