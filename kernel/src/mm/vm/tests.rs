//! 使用真实 Sv39 后端与有效对齐缓冲区验证公共内存接口。
//! Sv39 地址与 CPU 模型需要 64 位开发机；资源服务用例在两种位宽上均执行。
extern crate std;
use super::*;
#[cfg(target_pointer_width = "64")]
use crate::arch::riscv64::paging::{Sv39, VmCpu};
use crate::mm::PageAllocator;
use core::cell::{Cell, RefCell};
use std::{boxed::Box, rc::Rc, vec::Vec};

#[derive(Default)]
pub(crate) struct Stats {
    pub live: Cell<usize>,
    pub calls: Cell<usize>,
    pub budget: Cell<Option<usize>>,
    pub events: RefCell<Vec<&'static str>>,
    pub byte_borrows: Cell<usize>,
    pub writes: RefCell<Vec<(usize, usize, u64)>>,
}
#[repr(C, align(4096))]
pub(crate) struct Buffer(pub [u8; PAGE_SIZE]);
pub(crate) struct HostPage {
    pub pa: PhysAddr,
    pub bytes: Box<Buffer>,
    pub owner: Rc<Stats>,
}
impl HostPage {
    fn record_write(&self, slot: usize, value: u64) {
        self.owner
            .writes
            .borrow_mut()
            .push((self.pa.0, slot, value));
        self.owner.events.borrow_mut().push(if value == 0 {
            "clear"
        } else if value & 0x3ff == 1 {
            "publish"
        } else {
            "leaf"
        });
    }
}
// SAFETY: Box 独占完整对齐缓冲区，不借用服务；数字 PA 不转换为指针。
unsafe impl VmPage for HostPage {
    fn phys_addr(&self) -> PhysAddr {
        self.pa
    }
    fn with_bytes<R>(&self, f: impl FnOnce(&[u8; PAGE_SIZE]) -> R) -> R {
        self.owner
            .byte_borrows
            .set(self.owner.byte_borrows.get() + 1);
        f(&self.bytes.0)
    }
    fn with_bytes_mut<R>(&mut self, f: impl FnOnce(&mut [u8; PAGE_SIZE]) -> R) -> R {
        self.owner
            .byte_borrows
            .set(self.owner.byte_borrows.get() + 1);
        let before = self.bytes.0;
        let result = f(&mut self.bytes.0);
        for (slot, (old, new)) in before
            .as_chunks::<8>()
            .0
            .iter()
            .zip(self.bytes.0.as_chunks::<8>().0)
            .enumerate()
        {
            if old != new {
                let value = u64::from_le_bytes(*new);
                self.record_write(slot, value);
            }
        }
        result
    }
    fn read_word(&self, slot: usize) -> u64 {
        assert!(slot < PAGE_SIZE / size_of::<u64>());
        // SAFETY: Box 持有已初始化的对齐缓冲区，索引在页内；只使用裸指针。
        unsafe {
            (&raw const self.bytes.0)
                .cast::<u64>()
                .add(slot)
                .read_volatile()
        }
    }
    fn write_word(&mut self, slot: usize, value: u64) {
        let old = self.read_word(slot);
        // SAFETY: read_word 已检查边界，当前独占句柄；测试缓冲区不交给硬件。
        unsafe {
            (&raw mut self.bytes.0)
                .cast::<u64>()
                .add(slot)
                .write_volatile(value);
        }
        if old != value {
            self.record_write(slot, value);
        }
    }
}
pub(crate) struct HostMemory<'a> {
    allocator: PageAllocator<'a>,
    pub stats: Rc<Stats>,
    start: usize,
    end: usize,
}
impl<'a> HostMemory<'a> {
    pub fn new(flags: &'a mut [bool]) -> Self {
        let start = 0x803000;
        let end = start + flags.len() * PAGE_SIZE;
        Self {
            allocator: PageAllocator::new(start, end, flags).unwrap(),
            stats: Rc::default(),
            start,
            end,
        }
    }
    #[cfg(target_pointer_width = "64")]
    pub fn layout(&self) -> KernelLayout {
        KernelLayout {
            code: PhysAddr(0x800000)..PhysAddr(0x801000),
            rodata: PhysAddr(0x801000)..PhysAddr(0x802000),
            writable: PhysAddr(0x802000)..PhysAddr(self.start),
            allocatable: self.managed_range(),
            test_window: VirtAddr(0xffffffc000000000)..VirtAddr(0xffffffc000004000),
        }
    }
}
// SAFETY: MM-001 管理唯一数值页；独立 Box 持有内容，服务销毁不会释放持有页。
unsafe impl VmMemory for HostMemory<'_> {
    type Page = HostPage;
    fn managed_range(&self) -> Range<PhysAddr> {
        PhysAddr(self.start)..PhysAddr(self.end)
    }
    fn alloc_zeroed_page(&mut self) -> Result<HostPage, PageAllocError> {
        self.stats.calls.set(self.stats.calls.get() + 1);
        if let Some(left) = self.stats.budget.get() {
            if left == 0 {
                return Err(PageAllocError::OutOfMemory);
            }
            self.stats.budget.set(Some(left - 1));
        }
        let pa = PhysAddr(self.allocator.alloc()?);
        self.stats.live.set(self.stats.live.get() + 1);
        self.stats.events.borrow_mut().push("alloc");
        Ok(HostPage {
            pa,
            bytes: Box::new(Buffer([0; PAGE_SIZE])),
            owner: self.stats.clone(),
        })
    }
    unsafe fn release_page(&mut self, page: HostPage) -> Result<(), (PageAllocError, HostPage)> {
        if !Rc::ptr_eq(&page.owner, &self.stats) {
            return Err((PageAllocError::InvalidAddress, page));
        }
        if let Err(error) = self.allocator.dealloc(page.pa.0) {
            return Err((error, page));
        }
        self.stats.live.set(self.stats.live.get() - 1);
        self.stats.events.borrow_mut().push("free");
        Ok(())
    }
}
#[cfg(target_pointer_width = "64")]
pub(crate) struct Cpu {
    pub satp: usize,
    pub reject: bool,
    pub stats: Rc<Stats>,
}
// SAFETY: 只用于 host 数值模型；写入、读回和同步事件均由测试观察，不执行裸机指令。
#[cfg(target_pointer_width = "64")]
unsafe impl VmCpu for Cpu {
    fn read_satp(&self) -> usize {
        self.satp
    }
    unsafe fn write_satp(&mut self, value: usize) {
        if !self.reject || value == 0 {
            self.satp = value;
        }
        self.stats.events.borrow_mut().push("satp");
    }
    unsafe fn prepare(&mut self) -> Result<(), VmError> {
        Ok(())
    }
    unsafe fn fence(&mut self) {
        self.stats.events.borrow_mut().push("sync");
    }
}
#[cfg(target_pointer_width = "64")]
pub(crate) fn backend(stats: Rc<Stats>) -> Sv39<Cpu> {
    Sv39 {
        cpu: Cpu {
            satp: 0,
            reject: false,
            stats,
        },
    }
}

/// 注入切换后读回失败；不访问真实 CPU，状态由测试独立观察。
#[cfg(target_pointer_width = "64")]
struct UncertainCpu {
    satp: Rc<Cell<usize>>,
    reads: Rc<Cell<usize>>,
    uncertain_read: Rc<Cell<Option<usize>>>,
}
// SAFETY: host 状态模型准确返回注入的硬件观测值；切换不影响开发机的地址翻译。
#[cfg(target_pointer_width = "64")]
unsafe impl VmCpu for UncertainCpu {
    fn read_satp(&self) -> usize {
        let count = self.reads.get() + 1;
        self.reads.set(count);
        if self.uncertain_read.get() == Some(count) {
            9usize << 60
        } else {
            self.satp.get()
        }
    }
    unsafe fn write_satp(&mut self, value: usize) {
        self.satp.set(value);
    }
    unsafe fn prepare(&mut self) -> Result<(), VmError> {
        Ok(())
    }
    unsafe fn fence(&mut self) {}
}

#[test]
#[cfg(target_pointer_width = "64")]
fn uncertain_post_switch_state_is_fatal_and_keeps_table_pages_owned() {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    for deactivate in [false, true] {
        let mut flags = [false; 32];
        let mut memory = HostMemory::new(&mut flags);
        let layout = memory.layout();
        let stats = memory.stats.clone();
        let satp = Rc::new(Cell::new(0));
        let reads = Rc::new(Cell::new(0));
        let uncertain_read = Rc::new(Cell::new(None));
        let mut arch = Sv39 {
            cpu: UncertainCpu {
                satp: satp.clone(),
                reads: reads.clone(),
                uncertain_read: uncertain_read.clone(),
            },
        };
        let mut space = KernelAddressSpace::new(&mut memory, &mut arch, layout).unwrap();
        // SAFETY: 只使用数值 CPU 和有效 host 表缓冲区，不实际切换地址空间。
        if deactivate {
            unsafe {
                space.activate().unwrap();
            }
        }
        let active_root = (8usize << 60) | (space.root.0 >> 12);
        let owned = stats.live.get();
        reads.set(0);
        // 公共前置检查、后端前置检查、后端读回之后，公共层的最终核对不能传播普通错误。
        uncertain_read.set(Some(4));
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            // SAFETY: 同上；测试故意让第四次状态观测报告不支持的模式。
            unsafe {
                if deactivate {
                    space.deactivate()
                } else {
                    space.activate()
                }
            }
        }));
        assert_eq!(
            stats.live.get(),
            owned,
            "uncertain CPU state must retain every table"
        );
        // 只为 host 测试恢复确定的模型状态后清理；真实启动遇到该分支须终止。
        uncertain_read.set(None);
        if deactivate {
            satp.set(active_root);
            // SAFETY: 测试已经恢复模型根，没有真实 CPU 或别名访问。
            unsafe {
                space.deactivate().unwrap();
            }
        } else {
            satp.set(0);
        }
        space.destroy().unwrap();
        assert!(
            outcome.is_err(),
            "a post-switch read failure must terminate instead of returning {outcome:?}"
        );
    }
}

#[test]
#[cfg(target_pointer_width = "64")]
fn creates_base_mappings_before_activation() {
    let mut flags = [false; 32];
    let mut memory = HostMemory::new(&mut flags);
    let layout = memory.layout();
    let mut arch = backend(memory.stats.clone());
    let mut space = KernelAddressSpace::new(&mut memory, &mut arch, layout.clone())
        .expect("valid layout must create base mappings without CPU activation");
    for (range, perm) in [
        (layout.code, MapPerm::RX),
        (layout.rodata, MapPerm::R),
        (layout.writable, MapPerm::RW),
        (layout.allocatable, MapPerm::RW),
    ] {
        for base in (range.start.0..range.end.0).step_by(PAGE_SIZE) {
            for offset in [0, 37, (PAGE_SIZE - 1)] {
                assert_eq!(
                    space.query(VirtAddr(base + offset)),
                    Ok(Some(Translation {
                        phys: PhysAddr(base + offset),
                        perm
                    }))
                );
            }
        }
    }
    assert_eq!(space.query(layout.test_window.start), Ok(None));
    space.destroy().unwrap();
    drop(space);
    assert_eq!(memory.stats.live.get(), 0);
    assert_eq!(arch.cpu.satp, 0);
}

/// 测试调用点已停止所有别名访问，资源归还必须成功。
#[cfg(target_pointer_width = "64")]
fn release(memory: &mut HostMemory<'_>, page: HostPage) {
    // SAFETY: 每个用例先销毁空间或撤销所有窗口，且不保留缓冲区引用。
    unsafe { memory.release_page(page) }.unwrap_or_else(|(error, _)| panic!("release: {error:?}"));
}

#[test]
#[cfg(target_pointer_width = "64")]
fn invalid_layouts_allocate_nothing() {
    let mut flags = [false; 32];
    let mut memory = HostMemory::new(&mut flags);
    let original = memory.layout();
    for case in 0..11 {
        let mut layout = original.clone();
        match case {
            0 => layout.code.end = layout.code.start,
            1 => layout.code.start = PhysAddr(layout.code.start.0 + 1),
            2 => layout.rodata.end = PhysAddr(layout.rodata.start.0 - PAGE_SIZE),
            3 => layout.rodata.start = layout.code.start,
            4 => layout.allocatable.end = PhysAddr(layout.allocatable.end.0 + PAGE_SIZE),
            5 => layout.allocatable.end = PhysAddr(layout.allocatable.end.0 - PAGE_SIZE),
            6 => layout.test_window = VirtAddr(0x800000)..VirtAddr(0x801000),
            7 => layout.test_window = VirtAddr(0)..VirtAddr(0xffffffc000000000),
            8 => layout.test_window = VirtAddr(1usize << 38)..VirtAddr((1usize << 38) + PAGE_SIZE),
            9 => layout.test_window.end = VirtAddr(usize::MAX),
            _ => layout.writable.start = PhysAddr(usize::MAX - (PAGE_SIZE - 1)),
        }
        let mut arch = backend(memory.stats.clone());
        assert!(
            matches!(
                KernelAddressSpace::new(&mut memory, &mut arch, layout),
                Err(VmError::InvalidLayout)
            ),
            "case {case}"
        );
        assert_eq!(memory.stats.calls.get(), 0);
        assert_eq!(memory.stats.live.get(), 0);
    }
}

#[test]
#[cfg(target_pointer_width = "64")]
fn new_rolls_back_every_allocation_point_with_data_still_owned() {
    let mut flags = [false; 600];
    let mut memory = HostMemory::new(&mut flags);
    let stats = memory.stats.clone();
    let mut data = memory.alloc_zeroed_page().unwrap();
    data.with_bytes_mut(|bytes| bytes.fill(0x76));
    let layout = memory.layout();
    let mut arch = backend(stats.clone());
    let before = stats.live.get();
    let mut successful = KernelAddressSpace::new(&mut memory, &mut arch, layout.clone()).unwrap();
    let needed = stats.live.get() - before;
    successful.destroy().unwrap();
    drop(successful);
    for fail_at in 0..needed {
        stats.budget.set(Some(fail_at));
        assert!(matches!(
            KernelAddressSpace::new(&mut memory, &mut arch, layout.clone()),
            Err(VmError::Allocator(PageAllocError::OutOfMemory))
        ));
        assert_eq!(stats.live.get(), before);
        data.with_bytes(|bytes| assert!(bytes.iter().all(|b| *b == 0x76)));
    }
    stats.budget.set(None);
    release(&mut memory, data);
}

#[test]
#[cfg(target_pointer_width = "64")]
fn map_rolls_back_each_new_table_and_preserves_existing_mapping() {
    let mut flags = [false; 32];
    let mut memory = HostMemory::new(&mut flags);
    let stats = memory.stats.clone();
    let mut data = memory.alloc_zeroed_page().unwrap();
    data.with_bytes_mut(|bytes| bytes.fill(0x53));
    let layout = memory.layout();
    let va = layout.test_window.start;
    let mut arch = backend(stats.clone());
    let mut space = KernelAddressSpace::new(&mut memory, &mut arch, layout).unwrap();
    let before = stats.live.get();
    let original = space.query(VirtAddr(0x800017)).unwrap();
    for fail_at in 0..2 {
        stats.budget.set(Some(fail_at));
        // SAFETY: 数据句柄保持持有，没有真实虚拟访问；CPU 替身只记录状态。
        assert_eq!(
            unsafe { space.map_page(va, data.pa, MapPerm::RW) },
            Err(VmError::Allocator(PageAllocError::OutOfMemory))
        );
        assert_eq!(stats.live.get(), before);
        assert_eq!(space.query(va), Ok(None));
        assert_eq!(space.query(VirtAddr(0x800017)).unwrap(), original);
        data.with_bytes(|bytes| assert!(bytes.iter().all(|b| *b == 0x53)));
    }
    stats.budget.set(None);
    // SAFETY: 本用例只访问数值映射，页面句柄保持有效。
    unsafe {
        stats.events.borrow_mut().clear();
        stats.writes.borrow_mut().clear();
        space.map_page(va, data.pa, MapPerm::RW).unwrap();
        assert_eq!(
            *stats.events.borrow(),
            ["alloc", "alloc", "leaf", "publish", "publish"]
        );
        let writes = stats.writes.borrow();
        assert_eq!(writes.len(), 3);
        // 子表内容先完成，随后父项才指向它；两个 publish 不能只比较同名事件。
        for pair in writes.windows(2) {
            assert_eq!((pair[1].2 >> 10) << 12, pair[0].0 as u64);
            assert_eq!(pair[1].2 & 0x3ff, 1);
        }
    }
    assert_eq!(
        space.query(VirtAddr(va.0 + (PAGE_SIZE - 1))),
        Ok(Some(Translation {
            phys: PhysAddr(data.pa.0 + (PAGE_SIZE - 1)),
            perm: MapPerm::RW
        }))
    );
    assert_eq!(stats.live.get(), before + 2);
    // SAFETY: 未保留别名，允许断开；不会释放 data。
    unsafe {
        space.unmap_page(va).unwrap();
    }
    assert_eq!(stats.live.get(), before);
    space.destroy().unwrap();
    drop(space);
    assert_eq!(stats.live.get(), 1);
    release(&mut memory, data);
}

#[test]
#[cfg(target_pointer_width = "64")]
fn page_tables_do_not_borrow_whole_pages() {
    let mut flags = [false; 32];
    let mut memory = HostMemory::new(&mut flags);
    let stats = memory.stats.clone();
    let data = memory.alloc_zeroed_page().unwrap();
    let layout = memory.layout();
    let va = layout.test_window.start;
    let mut arch = backend(stats.clone());
    let mut space = KernelAddressSpace::new(&mut memory, &mut arch, layout).unwrap();
    // SAFETY: 仅使用数值 CPU 和持有的测试页，不执行真实地址翻译。
    unsafe {
        space.activate().unwrap();
        space.map_page(va, data.pa, MapPerm::RW).unwrap();
        assert_eq!(space.query(va).unwrap().unwrap().phys, data.pa);
        space.unmap_page(va).unwrap();
        space.deactivate().unwrap();
    }
    space.destroy().unwrap();
    assert_eq!(stats.byte_borrows.get(), 0);
    drop(space);
    release(&mut memory, data);
}

#[test]
#[cfg(target_pointer_width = "64")]
fn indexes_cross_two_mebibytes_and_one_gibibyte_without_aliasing() {
    for boundary in [0x200000, 0x40000000] {
        let mut flags = [false; 32];
        let mut memory = HostMemory::new(&mut flags);
        let data = memory.alloc_zeroed_page().unwrap();
        let other = memory.alloc_zeroed_page().unwrap();
        let mut layout = memory.layout();
        layout.test_window = VirtAddr(boundary - PAGE_SIZE)..VirtAddr(boundary + PAGE_SIZE);
        let mut arch = backend(memory.stats.clone());
        let mut space = KernelAddressSpace::new(&mut memory, &mut arch, layout).unwrap();
        // SAFETY: 两个独占页只参与数值映射；没有指针别名或并发。
        unsafe {
            space
                .map_page(VirtAddr(boundary - PAGE_SIZE), data.pa, MapPerm::R)
                .unwrap();
            space
                .map_page(VirtAddr(boundary), other.pa, MapPerm::RX)
                .unwrap();
        }
        assert_eq!(
            space.query(VirtAddr(boundary - 1)),
            Ok(Some(Translation {
                phys: PhysAddr(data.pa.0 + (PAGE_SIZE - 1)),
                perm: MapPerm::R
            }))
        );
        assert_eq!(
            space.query(VirtAddr(boundary)),
            Ok(Some(Translation {
                phys: other.pa,
                perm: MapPerm::RX
            }))
        );
        space.destroy().unwrap();
        drop(space);
        release(&mut memory, data);
        release(&mut memory, other);
    }
}

#[test]
#[cfg(target_pointer_width = "64")]
fn errors_follow_public_priority_and_never_replace_mapping() {
    let mut flags = [false; 32];
    let mut memory = HostMemory::new(&mut flags);
    let data = memory.alloc_zeroed_page().unwrap();
    let layout = memory.layout();
    let va = layout.test_window.start;
    let stats = memory.stats.clone();
    let mut arch = backend(stats.clone());
    let mut space = KernelAddressSpace::new(&mut memory, &mut arch, layout).unwrap();
    let bad_perm = MapPerm {
        user: true,
        ..MapPerm::RW
    };
    let cases = [
        (
            VirtAddr(1usize << 38),
            data.pa,
            MapPerm::RW,
            VmError::InvalidAddress,
        ),
        (
            va,
            PhysAddr(1usize << 56),
            MapPerm::RW,
            VmError::InvalidAddress,
        ),
        (
            VirtAddr(1usize << 38),
            PhysAddr(data.pa.0 + 1),
            MapPerm::RW,
            VmError::InvalidAddress,
        ),
        (
            VirtAddr(va.0 + 1),
            PhysAddr(1usize << 56),
            MapPerm::RW,
            VmError::InvalidAddress,
        ),
        (
            VirtAddr(0x800001),
            data.pa,
            bad_perm,
            VmError::UnalignedAddress,
        ),
        (
            VirtAddr(va.0 + 1),
            data.pa,
            MapPerm::RW,
            VmError::UnalignedAddress,
        ),
        (
            VirtAddr(0x800000),
            data.pa,
            MapPerm::RW,
            VmError::ProtectedMapping,
        ),
        (
            VirtAddr(0x800000),
            data.pa,
            bad_perm,
            VmError::ProtectedMapping,
        ),
        (
            va,
            PhysAddr(data.pa.0 + 1),
            MapPerm::RW,
            VmError::UnalignedAddress,
        ),
        (va, data.pa, bad_perm, VmError::UnsupportedPermissions),
        (
            va,
            data.pa,
            MapPerm {
                read: false,
                ..MapPerm::RW
            },
            VmError::UnsupportedPermissions,
        ),
        (
            va,
            data.pa,
            MapPerm {
                execute: true,
                ..MapPerm::RW
            },
            VmError::UnsupportedPermissions,
        ),
    ];
    for (v, p, perm, error) in cases {
        let calls = stats.calls.get();
        let owned = stats.live.get();
        let original = space.query(VirtAddr(0x800017)).unwrap();
        // SAFETY: 非法参数在访问/发布前被拒绝，唯一有效 PA 由 data 持有。
        assert_eq!(unsafe { space.map_page(v, p, perm) }, Err(error));
        assert_eq!(stats.calls.get(), calls);
        assert_eq!(stats.live.get(), owned);
        assert_eq!(space.query(VirtAddr(0x800017)).unwrap(), original);
        assert_eq!(space.query(va), Ok(None));
        data.with_bytes(|bytes| assert!(bytes.iter().all(|byte| *byte == 0)));
    }
    // SAFETY: 独占测试页，未保留任何真实别名。
    unsafe {
        assert_eq!(space.unmap_page(va), Err(VmError::NotMapped));
        assert_eq!(
            space.unmap_page(VirtAddr(1usize << 38)),
            Err(VmError::InvalidAddress)
        );
        assert_eq!(
            space.unmap_page(VirtAddr((1usize << 38) + 1)),
            Err(VmError::InvalidAddress)
        );
        assert_eq!(
            space.unmap_page(VirtAddr(0x800001)),
            Err(VmError::UnalignedAddress)
        );
        assert_eq!(
            space.unmap_page(VirtAddr(0x800000)),
            Err(VmError::ProtectedMapping)
        );
        assert_eq!(
            space.unmap_page(VirtAddr(0)),
            Err(VmError::ProtectedMapping)
        );
        space.map_page(va, data.pa, MapPerm::R).unwrap();
        assert_eq!(
            space.map_page(va, data.pa, bad_perm),
            Err(VmError::UnsupportedPermissions)
        );
        stats.budget.set(Some(0));
        assert_eq!(
            space.map_page(va, data.pa, MapPerm::RW),
            Err(VmError::AlreadyMapped)
        );
    }
    assert_eq!(space.query(va).unwrap().unwrap().perm, MapPerm::R);
    assert_eq!(
        space.query(VirtAddr(1usize << 38)),
        Err(VmError::InvalidAddress)
    );
    space.destroy().unwrap();
    drop(space);
    release(&mut memory, data);
}

#[test]
#[cfg(target_pointer_width = "64")]
fn lifecycle_syncs_before_reclaim_and_preserves_data_ownership() {
    let mut flags = [false; 32];
    let mut memory = HostMemory::new(&mut flags);
    let data = memory.alloc_zeroed_page().unwrap();
    let layout = memory.layout();
    let va = layout.test_window.start;
    let stats = memory.stats.clone();
    let mut arch = backend(stats.clone());
    let mut space = KernelAddressSpace::new(&mut memory, &mut arch, layout).unwrap();
    // SAFETY: CPU 替身不切换开发机地址，所有映射仅操作有效表缓冲区。
    unsafe {
        assert_eq!(space.deactivate(), Err(VmError::InvalidState));
        stats.events.borrow_mut().clear();
        space.activate().unwrap();
        assert_eq!(*stats.events.borrow(), ["sync", "satp", "sync"]);
        assert_eq!(space.activate(), Err(VmError::InvalidState));
        assert_eq!(space.destroy(), Err(VmError::ActiveAddressSpace));
        stats.events.borrow_mut().clear();
        space.map_page(va, data.pa, MapPerm::RW).unwrap();
        assert_eq!(
            *stats.events.borrow(),
            ["alloc", "alloc", "leaf", "publish", "publish", "sync"]
        );
        stats.events.borrow_mut().clear();
        assert_eq!(
            space.unmap_page(va),
            Ok(PageMapping {
                phys: data.pa,
                perm: MapPerm::RW
            })
        );
        assert_eq!(
            *stats.events.borrow(),
            ["clear", "clear", "clear", "sync", "free", "free"]
        );
        space.deactivate().unwrap();
        assert_eq!(space.deactivate(), Err(VmError::InvalidState));
        space.activate().unwrap();
        space.deactivate().unwrap();
    }
    space.destroy().unwrap();
    assert_eq!(stats.live.get(), 1);
    assert_eq!(space.destroy(), Err(VmError::Destroyed));
    assert_eq!(space.query(VirtAddr(usize::MAX)), Err(VmError::Destroyed));
    assert_eq!(space.query(VirtAddr(1usize << 38)), Err(VmError::Destroyed));
    // SAFETY: 销毁状态必须在任何地址操作前拒绝调用。
    unsafe {
        assert_eq!(space.activate(), Err(VmError::Destroyed));
        assert_eq!(space.deactivate(), Err(VmError::Destroyed));
        assert_eq!(
            space.map_page(VirtAddr(1), PhysAddr(1), MapPerm::RW),
            Err(VmError::Destroyed)
        );
        assert_eq!(space.unmap_page(VirtAddr(1)), Err(VmError::Destroyed));
        assert_eq!(
            space.map_page(VirtAddr(1usize << 38), PhysAddr(1usize << 56), MapPerm::RW),
            Err(VmError::Destroyed)
        );
        assert_eq!(
            space.unmap_page(VirtAddr(1usize << 38)),
            Err(VmError::Destroyed)
        );
    }
    drop(space);
    release(&mut memory, data);
}

#[test]
#[cfg(target_pointer_width = "64")]
fn unsupported_cpu_restores_bare_and_other_roots_are_not_overwritten() {
    use crate::hal::VmArch;
    let stats = Rc::new(Stats::default());
    let mut arch = backend(stats);
    arch.cpu.reject = true;
    // SAFETY: 数值 CPU 替身，没有真实转换。
    unsafe {
        assert_eq!(
            arch.activate(PhysAddr(0x804000)),
            Err(VmError::ActivationFailed)
        );
    }
    assert_eq!(arch.cpu.satp, 0);
    arch.cpu.reject = false;
    for value in [
        9usize << 60,
        (8usize << 60) | (1usize << 44),
        (8usize << 60) | 0x800,
    ] {
        arch.cpu.satp = value;
        // SAFETY: 数值替身，错误状态不能被覆盖。
        unsafe {
            assert_eq!(
                arch.activate(PhysAddr(0x804000)),
                Err(VmError::InvalidState)
            );
            assert_eq!(
                arch.deactivate(PhysAddr(0x804000)),
                Err(VmError::InvalidState)
            );
        }
        assert_eq!(arch.cpu.satp, value);
    }
}

#[test]
fn resource_rejection_keeps_handle_and_content_and_service_drop_keeps_page() {
    let mut flags = [false; 8];
    let mut other_flags = [false; 8];
    let mut memory = HostMemory::new(&mut flags);
    let mut other = HostMemory::new(&mut other_flags);
    let mut data = memory.alloc_zeroed_page().unwrap();
    data.with_bytes_mut(|bytes| bytes.fill(0x31));
    // SAFETY: 无映射/引用；故意交给错误服务，必须保留句柄。
    let (error, data) = unsafe { other.release_page(data) }.err().unwrap();
    assert_eq!(error, PageAllocError::InvalidAddress);
    assert_eq!(memory.stats.live.get(), 1);
    assert_eq!(other.stats.live.get(), 0);
    drop(memory);
    data.with_bytes(|bytes| assert!(bytes.iter().all(|b| *b == 0x31)));
}

#[test]
#[cfg(target_pointer_width = "64")]
fn active_allocation_rollback_syncs_before_free_and_one_new_table_can_fail() {
    let mut flags = [false; 32];
    let mut memory = HostMemory::new(&mut flags);
    let data = memory.alloc_zeroed_page().unwrap();
    let stats = memory.stats.clone();
    let mut arch = backend(stats.clone());
    let mut layout = memory.layout();
    let start = layout.test_window.start;
    layout.test_window.end = VirtAddr(start.0 + 0x201000);
    let mut space = KernelAddressSpace::new(&mut memory, &mut arch, layout).unwrap();
    // SAFETY: 数值 CPU 替身，持有数据，不访问真实虚拟地址。
    unsafe {
        space.activate().unwrap();
        let before = stats.live.get();
        stats.budget.set(Some(1));
        stats.events.borrow_mut().clear();
        assert_eq!(
            space.map_page(start, data.pa, MapPerm::RW),
            Err(VmError::Allocator(PageAllocError::OutOfMemory))
        );
        assert_eq!(*stats.events.borrow(), ["alloc", "sync", "free"]);
        assert_eq!(stats.live.get(), before);
        stats.budget.set(None);
        space.map_page(start, data.pa, MapPerm::R).unwrap();
        let before = stats.live.get();
        stats.budget.set(Some(0));
        assert_eq!(
            space.map_page(VirtAddr(start.0 + 0x200000), data.pa, MapPerm::R),
            Err(VmError::Allocator(PageAllocError::OutOfMemory))
        );
        assert_eq!(stats.live.get(), before);
        assert_eq!(space.query(start).unwrap().unwrap().phys, data.pa);
        assert_eq!(space.query(VirtAddr(start.0 + 0x200000)), Ok(None));
        space.deactivate().unwrap();
    }
    space.destroy().unwrap();
    drop(space);
    release(&mut memory, data);
}

#[test]
#[cfg(target_pointer_width = "64")]
fn occupied_valid_range_is_exhaustion_not_invalid_layout() {
    let mut flags = [false; 1];
    let mut memory = HostMemory::new(&mut flags);
    let data = memory.alloc_zeroed_page().unwrap();
    let layout = memory.layout();
    let mut arch = backend(memory.stats.clone());
    assert!(matches!(
        KernelAddressSpace::new(&mut memory, &mut arch, layout),
        Err(VmError::Allocator(PageAllocError::OutOfMemory))
    ));
    assert_eq!(memory.stats.live.get(), 1);
    release(&mut memory, data);
}

#[test]
#[cfg(target_pointer_width = "64")]
fn fixed_table_metadata_exhaustion_rolls_back_without_losing_old_entries() {
    let mut flags = [false; 256];
    let mut memory = HostMemory::new(&mut flags);
    let data = memory.alloc_zeroed_page().unwrap();
    let stats = memory.stats.clone();
    let mut layout = memory.layout();
    let start = layout.test_window.start;
    layout.test_window.end = VirtAddr(start.0 + 70 * (1 << 30));
    let mut arch = backend(stats.clone());
    let mut space = KernelAddressSpace::new(&mut memory, &mut arch, layout).unwrap();
    let mut exhausted = false;
    for index in 0..70 {
        let before = stats.live.get();
        // SAFETY: 未活动，数据页仍持有；每次操作一个不同的合成窗口地址。
        let result =
            unsafe { space.map_page(VirtAddr(start.0 + index * (1 << 30)), data.pa, MapPerm::R) };
        if result == Err(VmError::Allocator(PageAllocError::OutOfMemory)) {
            assert_eq!(stats.live.get(), before);
            assert_eq!(space.query(start).unwrap().unwrap().phys, data.pa);
            exhausted = true;
            break;
        }
        result.unwrap();
    }
    assert!(exhausted);
    space.destroy().unwrap();
    drop(space);
    assert_eq!(stats.live.get(), 1);
    release(&mut memory, data);
}
