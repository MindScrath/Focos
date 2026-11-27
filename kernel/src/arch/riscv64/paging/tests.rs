//! 独立读取实际 PTE 字节，不调用生产编码/解码函数来生成期望值。
extern crate std;
use super::*;
use crate::mm::{
    KernelAddressSpace, KernelBase, KernelLayout, UserAddressSpace, VmMemory,
    vm::tests::{HostMemory, HostPage, backend},
};
use std::vec::Vec;

/// 模拟 CPU 不接受切换、写回其他状态或前置检查失败，观察后端是否安全收尾。
struct WriteFaultCpu {
    satp: usize,
    activation_value: Option<usize>,
    reject_bare: bool,
    reject_prepare: bool,
    events: Vec<&'static str>,
}
// SAFETY: 仅运行数值模型，读回确实返回模型的实际状态；不修改开发机 CPU。
unsafe impl VmCpu for WriteFaultCpu {
    fn read_satp(&self) -> usize {
        self.satp
    }
    unsafe fn write_satp(&mut self, value: usize) {
        self.events.push("write");
        if value == 0 {
            if !self.reject_bare {
                self.satp = 0;
            }
        } else {
            self.satp = self.activation_value.unwrap_or(value);
        }
    }
    unsafe fn prepare(&mut self) -> Result<(), VmError> {
        if self.reject_prepare {
            Err(VmError::InvalidState)
        } else {
            Ok(())
        }
    }
    unsafe fn fence(&mut self) {
        self.events.push("sync");
    }
}

#[test]
fn malformed_bare_is_rejected_before_any_cpu_write() {
    for satp in [1, 1usize << 44] {
        let mut arch = backend(Default::default());
        arch.cpu.satp = satp;
        assert_eq!(arch.current_root(), Err(VmError::InvalidState));
        // SAFETY: 只运行数值 CPU 模型；异常初态不能被覆盖成新页表。
        assert_eq!(
            unsafe { arch.activate(PhysAddr::from_raw(0x804000)) },
            Err(VmError::InvalidState)
        );
        assert_eq!(arch.cpu.satp, satp);
        assert!(arch.cpu.stats.events.borrow().is_empty());
    }
}

#[test]
fn user_root_switch_rejects_invalid_inputs_and_confirms_rollback() {
    let old = PhysAddr::from_raw(0x804000);
    let next = PhysAddr::from_raw(0x807000);
    let old_satp = (8usize << 60) | 0x804;
    let mut arch = Sv39 {
        cpu: WriteFaultCpu {
            satp: old_satp,
            activation_value: Some(old_satp),
            reject_bare: false,
            reject_prepare: false,
            events: Vec::new(),
        },
    };
    // SAFETY: 数值后端，错误参数不访问物理地址。恢复失败用 panic 捕获器观察。
    unsafe {
        for (from, to, error) in [
            (
                PhysAddr::from_raw(1),
                PhysAddr::from_raw(1usize << 56),
                VmError::InvalidAddress,
            ),
            (old, PhysAddr::from_raw(5), VmError::UnalignedAddress),
            (next, old, VmError::InvalidState),
        ] {
            assert_eq!(arch.switch_root(from, to), Err(error));
            assert!(arch.cpu.events.is_empty());
        }
        arch.cpu.reject_prepare = true;
        assert_eq!(arch.switch_root(old, next), Err(VmError::InvalidState));
        assert!(arch.cpu.events.is_empty());
        arch.cpu.reject_prepare = false;
        assert_eq!(arch.switch_root(old, next), Err(VmError::ActivationFailed));
        assert_eq!(arch.cpu.satp, old_satp);
        assert_eq!(
            arch.cpu.events,
            ["sync", "write", "sync", "sync", "write", "sync"]
        );
        arch.cpu.activation_value = Some(9usize << 60);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| arch.switch_root(old, next)))
                .is_err()
        );
    }
}

#[test]
fn activation_readback_mismatch_restores_bare_before_error() {
    // 分别覆盖未切换、模式错误、根页错误和地址空间编号错误。
    for actual in [
        0,
        9usize << 60,
        (8usize << 60) | 0x805,
        (8usize << 60) | (1 << 44) | 0x804,
    ] {
        let mut arch = Sv39 {
            cpu: WriteFaultCpu {
                satp: 0,
                activation_value: Some(actual),
                reject_bare: false,
                reject_prepare: false,
                events: Vec::new(),
            },
        };
        // SAFETY: 只修改数值 CPU 模型；错误读回之后必须恢复并确认 Bare。
        assert_eq!(
            unsafe { arch.activate(PhysAddr::from_raw(0x804000)) },
            Err(VmError::ActivationFailed)
        );
        assert_eq!(arch.current_root(), Ok(None));
        assert_eq!(
            arch.cpu.events,
            ["sync", "write", "sync", "sync", "write", "sync"]
        );
    }
}

#[test]
fn unconfirmed_bare_is_fatal_on_both_switch_paths() {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    for deactivate in [false, true] {
        let mut arch = Sv39 {
            cpu: WriteFaultCpu {
                satp: if deactivate {
                    (8usize << 60) | 0x804
                } else {
                    0
                },
                activation_value: Some(9usize << 60),
                reject_bare: true,
                reject_prepare: false,
                events: Vec::new(),
            },
        };
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            // SAFETY: 模型故意拒绝 Bare 写入；真实内核须 panic 后关机。
            unsafe {
                if deactivate {
                    arch.deactivate(PhysAddr::from_raw(0x804000))
                } else {
                    arch.activate(PhysAddr::from_raw(0x804000))
                }
            }
        }));
        assert!(outcome.is_err(), "uncertain state must not return normally");
        assert_ne!(arch.cpu.satp, 0);
    }
}

#[test]
fn rejected_preparation_does_not_write_cpu_state() {
    let mut arch = Sv39 {
        cpu: WriteFaultCpu {
            satp: 0,
            activation_value: None,
            reject_bare: false,
            reject_prepare: true,
            events: Vec::new(),
        },
    };
    // SAFETY: 只运行 CPU 模型，模拟异常入口或关中断前提不满足。
    assert_eq!(
        unsafe { arch.activate(PhysAddr::from_raw(0x804000)) },
        Err(VmError::InvalidState)
    );
    assert_eq!(arch.current_root(), Ok(None));
    assert!(arch.cpu.events.is_empty());
}

fn raw<T: TableAccess>(tables: &T, pa: PhysAddr, index: usize) -> u64 {
    tables
        .table(pa)
        .unwrap()
        .with_bytes(|bytes| u64::from_le_bytes(bytes[index * 8..index * 8 + 8].try_into().unwrap()))
}
fn set_raw<T: TableAccess>(tables: &mut T, pa: PhysAddr, index: usize, value: u64) {
    tables.table_mut(pa).unwrap().with_bytes_mut(|bytes| {
        bytes[index * 8..index * 8 + 8].copy_from_slice(&value.to_le_bytes())
    });
}
fn leaf_location<T: TableAccess>(tables: &T, root: PhysAddr, va: usize) -> (PhysAddr, usize) {
    let middle = PhysAddr::from_raw(((raw(tables, root, (va >> 30) & 511) >> 10) << 12) as usize);
    let leaf = PhysAddr::from_raw(((raw(tables, middle, (va >> 21) & 511) >> 10) << 12) as usize);
    (leaf, (va >> 12) & 511)
}
fn audit<T: TableAccess>(
    tables: &T,
    root: PhysAddr,
    layout: &KernelLayout,
) -> Result<(), &'static str> {
    for (range, flags) in [
        (&layout.code, 0xcbu64),
        (&layout.rodata, 0xc3),
        (&layout.writable, 0xc7),
        (&layout.allocatable, 0xc7),
    ] {
        for pa in (range.start.as_usize()..range.end.as_usize()).step_by(4096) {
            let mut table = root;
            for shift in [30, 21] {
                let entry = raw(tables, table, (pa >> shift) & 511);
                if entry & 0x3ff != 1 || entry >> 54 != 0 {
                    return Err("intermediate bits");
                }
                table = PhysAddr::from_raw(((entry >> 10) << 12) as usize);
            }
            let entry = raw(tables, table, (pa >> 12) & 511);
            if entry & 0x3ff != flags {
                return Err("leaf permissions");
            }
            if entry >> 10 != (pa >> 12) as u64 {
                return Err("leaf physical page or reserved bits");
            }
        }
    }
    Ok(())
}

#[test]
fn actual_base_entries_have_independently_checked_permissions_and_ppns() {
    let mut flags = [false; 600];
    let mut memory = HostMemory::new(&mut flags);
    let layout = memory.layout();
    let mut arch = backend(memory.stats.clone());
    let mut space = KernelAddressSpace::new(&mut memory, &mut arch, layout.clone()).unwrap();
    assert_eq!(audit(&*space.tables, space.root, &layout), Ok(()));
    // 两个有意破坏的负例证明审计会拒绝只读变可写、数据变可执行。
    for (va, clear, set) in [
        (layout.rodata.start.as_usize(), 0, 4),
        (layout.writable.start.as_usize(), 4, 8),
    ] {
        let (table, slot) = leaf_location(&*space.tables, space.root, va);
        let original = raw(&*space.tables, table, slot);
        set_raw(&mut *space.tables, table, slot, (original & !clear) | set);
        assert_eq!(
            audit(&*space.tables, space.root, &layout),
            Err("leaf permissions")
        );
        set_raw(&mut *space.tables, table, slot, original);
    }
    space.destroy().unwrap();
}

#[test]
fn illegal_entries_and_unknown_tables_are_rejected_without_dereferencing_pas() {
    let mut flags = [false; 32];
    let mut memory = HostMemory::new(&mut flags);
    let layout = memory.layout();
    let va = layout.code.start.as_usize();
    let mut arch = backend(memory.stats.clone());
    let mut space = KernelAddressSpace::new(&mut memory, &mut arch, layout).unwrap();
    let (table, slot) = leaf_location(&*space.tables, space.root, va);
    let original = raw(&*space.tables, table, slot);
    for corrupt in [
        original | (1 << 63),
        original | (1 << 8),
        original | 16,
        original | 32,
        original & !64,
        original & !128,
        original & !1,
        original & !2,
        original | 4,
        1,
    ] {
        set_raw(&mut *space.tables, table, slot, corrupt);
        assert_eq!(
            space.query(VirtAddr::from_raw(va)),
            Err(VmError::InvalidTable)
        );
    }
    set_raw(&mut *space.tables, table, slot, original);
    let root = space.root;
    let root_slot = (va >> 30) & 511;
    let original = raw(&*space.tables, root, root_slot);
    for corrupt in [
        0xdead0001,
        original | 64,
        original | 2,
        ((root.as_usize() as u64 >> 12) << 10) | 1,
    ] {
        set_raw(&mut *space.tables, root, root_slot, corrupt);
        assert_eq!(
            space.query(VirtAddr::from_raw(va)),
            Err(VmError::InvalidTable)
        );
    }
    set_raw(&mut *space.tables, root, root_slot, original);
    space.destroy().unwrap();
}

struct LooseTables(Vec<HostPage>);

#[test]
fn user_range_and_reserved_slots_reject_holes_and_edges() {
    let arch = backend(Default::default());
    let range = |a, b| VirtAddr::from_raw(a)..VirtAddr::from_raw(b);
    let high = 0usize.wrapping_sub(1usize << 38);
    let ok = range(0x10000, 1 << 30);
    let kernel = range(0x80200000, 0x88000000);
    assert!(arch.valid_user_range(ok.clone(), &[kernel.clone(), range(high, high + 0x4000)]));
    assert!(arch.valid_user_range(range(PAGE_SIZE, 2 * PAGE_SIZE), &[]));
    for bad in [
        range(0, 0x2000),
        range(0x2100, 0x4000),
        range(0x2000, 0x4001),
        range(0x2000, 0x2000),
        range(high, 0x2000),
        range(high, high + 0x4000),
        range(1usize << 38, (1usize << 38) + PAGE_SIZE),
        range(0x10000, high + PAGE_SIZE),
        range((1usize << 38) - PAGE_SIZE, (1usize << 38) + PAGE_SIZE),
    ] {
        assert!(!arch.valid_user_range(bad, &[]));
    }
    for reserved in [
        range(0x8000, 0x9000),
        range(0x40000001, 0x40002000),
        range(0x40000000, high + PAGE_SIZE),
    ] {
        assert!(!arch.valid_user_range(ok.clone(), &[kernel.clone(), reserved]));
    }
}

#[test]
fn copying_shared_kernel_root_validates_every_slot_before_first_write() {
    let mut flags = [false; 12];
    let mut memory = HostMemory::new(&mut flags);
    let mut arch = backend(memory.stats.clone());
    let mut tables = LooseTables(
        (0..4)
            .map(|_| memory.alloc_zeroed_page().unwrap())
            .collect(),
    );
    let (dst, src, mid, other) = (
        tables.0[0].pa,
        tables.0[1].pa,
        tables.0[2].pa,
        tables.0[3].pa,
    );
    let range = VirtAddr::from_raw(0x40000000)..VirtAddr::from_raw(0x80001000);
    let ptr = |pa: PhysAddr| ((pa.as_usize() as u64 >> 12) << 10) | 1;
    set_raw(&mut tables, src, 1, ptr(mid));
    set_raw(&mut tables, src, 256, ptr(other)); // 模拟不应复制的窗口顶层项。
    // SAFETY: 两棵数值树均未启用；故意破坏输入，检查后端在写第一个槽位前拒绝。
    unsafe {
        for bad in [
            0,
            0xc3,
            ptr(src),
            ptr(dst),
            ptr(PhysAddr::from_raw(0xdead000)),
        ] {
            set_raw(&mut tables, src, 2, bad);
            assert_eq!(
                arch.copy_kernel_root(&mut tables, dst, src, range.clone()),
                Err(VmError::InvalidTable)
            );
            assert_eq!(raw(&tables, dst, 1), 0);
        }
        set_raw(&mut tables, src, 2, ptr(other));
        set_raw(&mut tables, dst, 5, 1);
        assert_eq!(
            arch.copy_kernel_root(&mut tables, dst, src, range.clone()),
            Err(VmError::InvalidTable)
        );
        set_raw(&mut tables, dst, 5, 0);
        for root in [src, PhysAddr::from_raw(1), PhysAddr::from_raw(1usize << 56)] {
            assert_eq!(
                arch.copy_kernel_root(&mut tables, root, src, range.clone()),
                Err(VmError::InvalidTable)
            );
        }
        arch.copy_kernel_root(&mut tables, dst, src, range).unwrap();
        assert_eq!(raw(&tables, dst, 1), ptr(mid));
        assert_eq!(raw(&tables, dst, 2), ptr(other));
        assert_eq!(raw(&tables, dst, 256), 0);
        for page in tables.0 {
            assert!(memory.release_page(page).is_ok());
        }
    }
}
impl TableAccess for LooseTables {
    type Page = HostPage;
    fn table(&self, pa: PhysAddr) -> Result<&HostPage, VmError> {
        self.0
            .iter()
            .find(|p| p.pa == pa)
            .ok_or(VmError::InvalidTable)
    }
    fn table_mut(&mut self, pa: PhysAddr) -> Result<&mut HostPage, VmError> {
        self.0
            .iter_mut()
            .find(|p| p.pa == pa)
            .ok_or(VmError::InvalidTable)
    }
}

#[test]
fn spare_contract_is_checked_before_publishing_and_permissions_round_trip() {
    let mut flags = [false; 32];
    let mut memory = HostMemory::new(&mut flags);
    let mut tables = LooseTables(
        (0..4)
            .map(|_| memory.alloc_zeroed_page().unwrap())
            .collect(),
    );
    let pas: Vec<_> = tables.0.iter().map(|p| p.pa).collect();
    let mut arch = backend(memory.stats.clone());
    let va = VirtAddr::from_raw(0xffffffc000000000);
    let mapping = PageMapping {
        phys: pas[3],
        perm: MapPerm::R,
    };
    for spare in [&pas[1..2], &[pas[1], pas[1]], &[pas[0], pas[1]]] {
        // SAFETY: 未活动的有效表；故意提供错误 spare 验证发布前拒绝。
        assert_eq!(
            unsafe { arch.map_page(&mut tables, pas[0], va, mapping, spare) },
            Err(VmError::InvalidState)
        );
        assert_eq!(arch.query(&tables, pas[0], va), Ok(None));
    }
    set_raw(&mut tables, pas[1], 0, 1);
    // SAFETY: 非零 spare 必须在发布前被拒绝，不修改既有根。
    assert_eq!(
        unsafe { arch.map_page(&mut tables, pas[0], va, mapping, &pas[1..3]) },
        Err(VmError::InvalidState)
    );
    set_raw(&mut tables, pas[1], 0, 0);
    for perm in [MapPerm::R, MapPerm::RW, MapPerm::RX] {
        // SAFETY: 所有页独占、串行、未活动；断开的表归还前先清零供下轮复用。
        unsafe {
            arch.map_page(
                &mut tables,
                pas[0],
                va,
                PageMapping { perm, ..mapping },
                &pas[1..3],
            )
            .unwrap();
        }
        let entry = raw(&tables, pas[2], 0);
        let flags = if perm == MapPerm::R {
            0xc3
        } else if perm == MapPerm::RW {
            0xc7
        } else {
            0xcb
        };
        assert_eq!(entry & 0x3ff, flags);
        assert_eq!(
            arch.query(&tables, pas[0], VirtAddr::from_raw(va.as_usize() + 93)),
            Ok(Some(Translation {
                phys: PhysAddr::from_raw(mapping.phys.as_usize() + 93),
                perm
            }))
        );
        // SAFETY: 没有真实别名或 CPU 翻译，撤销只改测试页表。
        let removed = unsafe { arch.unmap_page(&mut tables, pas[0], va) }.unwrap();
        assert_eq!(removed.mapping, PageMapping { perm, ..mapping });
        assert_eq!(removed.retired, [Some(pas[2]), Some(pas[1])]);
    }
    for page in tables.0 {
        // SAFETY: 根未启用，所有数据映射均已撤销，无保留引用。
        assert!(unsafe { memory.release_page(page) }.is_ok());
    }
}

#[test]
fn mm004_leaf_bytes_distinguish_user_and_kernel_permissions() {
    let mut flags = [false; 16];
    let mut memory = HostMemory::at(&mut flags, 0x8020_0000 + 3 * PAGE_SIZE);
    let mut arch = backend(memory.stats.clone());
    let layout = memory.layout();
    let allowed = crate::platform::user_address_range();
    let mut kernel =
        KernelBase::new(&mut memory, &mut arch, layout.clone(), allowed.clone()).unwrap();
    let mut space =
        UserAddressSpace::new(&kernel, &mut memory, &mut arch, allowed.clone()).unwrap();
    let mut data = Vec::new();

    for (n, (perm, expected)) in [
        (MapPerm::USER_R, 0xd3),
        (MapPerm::USER_RW, 0xd7),
        (MapPerm::USER_RX, 0xdb),
    ]
    .into_iter()
    .enumerate()
    {
        let va = allowed.start.as_usize() + n * PAGE_SIZE;
        let page = memory.alloc_zeroed_page().unwrap();
        let pa = page.phys_addr();
        data.push(page);
        if n == 0 {
            let live = memory.stats.live.get();
            // SAFETY: 同一张授权的测试页；错误权限不应分配，第二张子表耗尽不应留下第一张。
            unsafe {
                assert_eq!(
                    space.map_page(
                        &mut memory,
                        &mut arch,
                        VirtAddr::from_raw(va),
                        pa,
                        MapPerm::R
                    ),
                    Err(VmError::UnsupportedPermissions)
                );
                memory.stats.budget.set(Some(1));
                assert_eq!(
                    space.map_page(&mut memory, &mut arch, VirtAddr::from_raw(va), pa, perm),
                    Err(VmError::Allocator(crate::mm::PageAllocError::OutOfMemory))
                );
                memory.stats.budget.set(None);
            }
            assert_eq!(space.query(&arch, VirtAddr::from_raw(va)), Ok(None));
            assert_eq!(memory.stats.live.get(), live);
        }
        // SAFETY: 未启用的独占测试树；每个 VA 对应独立、零初始化的数据页，持有至销毁后。
        unsafe { space.map_page(&mut memory, &mut arch, VirtAddr::from_raw(va), pa, perm) }
            .unwrap();
        let (root, tables) = space.test_tree();
        let mut table = root;
        for shift in [30, 21] {
            let entry = raw(tables, table, (va >> shift) & 511);
            assert_eq!(entry & 0x3ff, 1, "non-leaf at VA {va:#x}");
            table = PhysAddr::from_raw(((entry >> 10) << 12) as usize);
        }
        let slot = (va >> 12) & 511;
        let entry = raw(tables, table, slot);
        assert_eq!(entry & 0x3ff, expected, "leaf at VA {va:#x}");
        assert_eq!(entry >> 10, (pa.as_usize() >> 12) as u64);
        set_raw(tables, table, slot, entry & !0x10);
        assert_eq!(
            space.query(&arch, VirtAddr::from_raw(va)),
            Err(VmError::InvalidTable)
        );
        set_raw(space.test_tree().1, table, slot, entry);
        assert_eq!(
            space.query(&arch, VirtAddr::from_raw(va)).unwrap(),
            Some(Translation { phys: pa, perm })
        );
    }
    // 从用户根独立遍历所有内核段，核对内核叶项未带 U、中间项只带 V。
    let (root, tables) = space.test_tree();
    assert_eq!(audit(tables, root, &layout), Ok(()));
    let (shared, _) = leaf_location(tables, root, layout.code.start.as_usize());
    assert!(matches!(
        tables.table_mut(shared),
        Err(VmError::InvalidTable)
    ));
    // 已连接的内核表不能作为新用户路径的备用页。返回错误前不得改写任一共享表项。
    let va = VirtAddr::from_raw(allowed.start.as_usize() + 0x20_0000);
    let mapping = PageMapping {
        phys: data[0].phys_addr(),
        perm: MapPerm::USER_R,
    };
    // SAFETY: 未发布的数值模型，故意传入无效 spare 以核对后端拒绝条件。
    assert_eq!(
        unsafe { arch.map_page(tables, root, va, mapping, &[shared]) },
        Err(VmError::InvalidState)
    );
    assert_eq!(audit(tables, root, &layout), Ok(()));
    space.destroy(&mut memory, &arch).unwrap();
    drop(space);
    kernel.destroy(&mut memory, &arch).unwrap();
    assert_eq!(memory.stats.live.get(), data.len());
    for page in data {
        // SAFETY: 测试树均未启用且已经销毁，独占数据页没有存活别名。
        assert!(unsafe { memory.release_page(page) }.is_ok());
    }
}

#[test]
fn corrupt_shared_kernel_leaf_is_rejected_and_bad_root_copy_releases_new_page() {
    let mut flags = [false; 16];
    let mut memory = HostMemory::at(&mut flags, 0x8020_3000);
    let mut arch = backend(memory.stats.clone());
    let layout = memory.layout();
    let code = VirtAddr::from_raw(layout.code.start.as_usize());
    let range = crate::platform::user_address_range();
    let mut kernel = KernelBase::new(&mut memory, &mut arch, layout, range.clone()).unwrap();
    let (root, tables) = kernel.test_tree();
    let (leaf, slot) = leaf_location(tables, root, code.as_usize());
    let original = raw(tables, leaf, slot);
    set_raw(tables, leaf, slot, original | 0x10);
    assert_eq!(kernel.query(&arch, code), Err(VmError::InvalidTable));
    let mut space = UserAddressSpace::new(&kernel, &mut memory, &mut arch, range.clone()).unwrap();
    assert_eq!(space.query(&arch, code), Err(VmError::InvalidTable));
    space.destroy(&mut memory, &arch).unwrap();
    drop(space);
    let (root, tables) = kernel.test_tree();
    set_raw(tables, leaf, slot, original);
    let top = (code.as_usize() >> 30) & 511;
    let pointer = raw(tables, root, top);
    set_raw(tables, root, top, 1);
    let before = memory.stats.live.get();
    let calls = memory.stats.calls.get();
    assert_eq!(
        UserAddressSpace::new(&kernel, &mut memory, &mut arch, range).err(),
        Some(VmError::InvalidTable)
    );
    assert_eq!(memory.stats.calls.get(), calls + 1);
    assert_eq!(memory.stats.live.get(), before);
    set_raw(kernel.test_tree().1, root, top, pointer);
    kernel.destroy(&mut memory, &arch).unwrap();
    assert_eq!(memory.stats.live.get(), 0);
}

#[test]
fn handoff_rejects_unreadable_window_and_preserves_destroyed_state() {
    let mut flags = [false; 16];
    let mut memory = HostMemory::at(&mut flags, 0x8020_3000);
    let mut arch = backend(memory.stats.clone());
    let range = crate::platform::user_address_range();
    let layout = memory.layout();
    let slot = (layout.test_window.start.as_usize() >> 30) & 511;
    let mut space = KernelAddressSpace::new(&mut memory, &mut arch, layout).unwrap();
    // SAFETY: 开发机数值 CPU 不读取测试树；篡改仅涉及空窗口的顶层项。
    unsafe { space.activate().unwrap() };
    let root = space.root;
    set_raw(&mut *space.tables, root, slot, 1);
    let (error, mut space) = space.into_kernel_base(range.clone()).err().unwrap();
    assert_eq!(error, VmError::InvalidState);
    set_raw(&mut *space.tables, root, slot, 0);
    // SAFETY: 根与完整基础映射仍持有，窗口已恢复，数值后端安全返回 Bare。
    unsafe { space.deactivate().unwrap() };
    space.destroy().unwrap();
    assert_eq!(
        space.into_kernel_base(range).err().unwrap().0,
        VmError::Destroyed
    );
}
