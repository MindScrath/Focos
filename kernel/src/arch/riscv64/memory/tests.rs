//! 校验 QEMU 保留区和页边界。地址仅作数值使用，不访问开发机上的对应位置。

use super::*;

/// 规格示例向上保留内核尾页，向下排除设备树所在页。
#[test]
fn aligns_spec_example_without_exposing_reserved_pages() {
    assert_eq!(
        usable_range(0x8021_f123, 0x87e0_0000),
        Ok(0x8022_0000..0x87e0_0000)
    );
    assert_eq!(
        usable_range(0x8021_f123, 0x87e0_0128),
        Ok(0x8022_0000..0x87e0_0000)
    );
}

/// 已对齐的内核末尾不应额外丢失一页；最小合法范围恰有一个完整页。
#[test]
fn preserves_aligned_edges_and_single_page_ranges() {
    assert_eq!(
        usable_range(KERNEL_START, KERNEL_START + PAGE_SIZE),
        Ok(KERNEL_START..KERNEL_START + PAGE_SIZE)
    );
}

/// 无效启动参数、没有完整页和整数边界都不能成为可分配范围。
#[test]
fn rejects_invalid_or_empty_layouts() {
    for (kernel_end, dtb_start) in [
        (KERNEL_START, 0),
        (KERNEL_START - 1, 0x87e0_0000),
        (RAM_START, 0x87e0_0000),
        (RAM_END, 0x87e0_0000),
        (KERNEL_START, RAM_END),
        (KERNEL_START, RAM_END + PAGE_SIZE),
        (KERNEL_START, 0x87e0_0001),
        (KERNEL_START, KERNEL_START),
        (KERNEL_START + PAGE_SIZE, KERNEL_START),
        (KERNEL_START + 1, KERNEL_START + PAGE_SIZE),
        (KERNEL_START, KERNEL_START + PAGE_SIZE - 8),
        (usize::MAX, usize::MAX),
        (KERNEL_START, usize::MAX - 7),
    ] {
        assert_eq!(
            usable_range(kernel_end, dtb_start),
            Err(MemoryInitError::InvalidLayout),
            "{kernel_end:#x}, {dtb_start:#x}"
        );
    }
}

/// 布局错误先于标记容量错误，失败不改变传入标记。
#[test]
fn invalid_layout_preserves_flags_and_precedes_capacity_check() {
    let original = [true, false, true];
    for length in [0, 1, 3] {
        let mut used = original;
        assert!(matches!(
            init_page_allocator(KERNEL_START, 0, &mut used[..length]),
            Err(MemoryInitError::InvalidLayout)
        ));
        assert_eq!(used, original);
    }
}

/// 标记不足时透传 MM-001 错误，不缩小范围或清除已有标记。
#[test]
fn insufficient_flags_preserve_metadata() {
    let original = [true, false, true];
    let mut used = original;
    assert!(matches!(
        init_page_allocator(KERNEL_START, KERNEL_START + 4 * PAGE_SIZE, &mut used),
        Err(MemoryInitError::Allocator(PageAllocError::MetadataTooSmall))
    ));
    assert_eq!(used, original);
}

/// 分配全部可用页后耗尽；额外标记不能被当作越过保留区的内存。
#[test]
fn initialized_allocator_stays_inside_usable_range() {
    let mut used = [true; 4];
    {
        let (range, mut allocator) = init_page_allocator(
            KERNEL_START + 1,
            KERNEL_START + 4 * PAGE_SIZE + 8,
            &mut used,
        )
        .unwrap();
        assert_eq!(
            range,
            KERNEL_START + PAGE_SIZE..KERNEL_START + 4 * PAGE_SIZE
        );
        let mut pages = [
            allocator.alloc().unwrap(),
            allocator.alloc().unwrap(),
            allocator.alloc().unwrap(),
        ];
        pages.sort_unstable();
        assert_eq!(
            pages,
            [range.start, range.start + PAGE_SIZE, range.end - PAGE_SIZE]
        );
        assert_eq!(allocator.alloc(), Err(PageAllocError::OutOfMemory));
        for page in pages {
            allocator.dealloc(page).unwrap();
        }
    }
    assert_eq!(used, [false, false, false, true]);
}

/// 静态标记足以覆盖约定环境中的最大可用页数。
#[test]
fn static_metadata_covers_largest_valid_layout() {
    let range = usable_range(KERNEL_START, RAM_END - 8).unwrap();
    assert!((range.end - range.start) / PAGE_SIZE <= MAX_PAGE_COUNT);
}
