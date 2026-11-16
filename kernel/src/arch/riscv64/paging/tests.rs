//! 独立读取实际 PTE 字节，不调用生产编码/解码函数来生成期望值。
extern crate std;
use super::*;
use crate::mm::{
    KernelAddressSpace, KernelLayout, VmMemory,
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
