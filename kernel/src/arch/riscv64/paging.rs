//! Sv39 编码和遍历；开发机只替换 CPU，不替换页表操作。
use crate::hal::{RemovedMapping, TableAccess, VmArch};
use crate::mm::{
    MapPerm, PAGE_SIZE, PageMapping, PhysAddr, Translation, VirtAddr, VmError, VmPage,
};
use core::sync::atomic::{Ordering, compiler_fence};

// 本后端的 12 位页内偏移与 512 个 8 字节表项要求公共页大小固定为 4 KiB。
const _: () = assert!(PAGE_SIZE == 4096);

/// 内部 CPU 适配；开发机只替换寄存器和屏障，页表操作保持同一实现。
///
/// # Safety
/// 必须准确报告 CPU 当前状态；写寄存器与全量同步遵守本核分页语义，
/// prepare 验证关中断和异常入口。不能以缓存的期望值冒充真实读回。
pub unsafe trait VmCpu {
    /// 读取实际分页控制寄存器，不能返回缓存的期望值。
    fn read_satp(&self) -> usize;
    /// 写入分页模式、编号和根页号；不隐式完成同步。
    ///
    /// # Safety
    /// 调用者确保新旧执行路径、栈和表页有效，且切换前后完成同步。
    unsafe fn write_satp(&mut self, value: usize);
    /// 验证异常入口和关中断状态，清除 SUM/MXR 并读回；失败为 InvalidState。
    ///
    /// # Safety
    /// 唯一启动核，允许清除 SUM/MXR；失败不能继续启用。
    unsafe fn prepare(&mut self) -> Result<(), VmError>;
    /// 完成本核全部地址转换同步及编译器访存约束。
    ///
    /// # Safety
    /// 当前核串行修改表页，写入已结束，页面仍被持有。
    unsafe fn fence(&mut self);
}
/// 无分配器的 Sv39 后端；CPU 控制可替换，表项操作始终使用同一实现。
pub struct Sv39<C> {
    pub(crate) cpu: C,
}

#[cfg(test)]
mod tests;
/// 遍历结果：depth 是最后访问的表层，0 为根，2 为叶表。
struct Path {
    tables: [PhysAddr; 3],
    slots: [usize; 3],
    depth: usize,
    leaf: Option<PageMapping>,
}

/// 完整 64 位、对齐、volatile 访问；不把数字 PA 直接当指针。
fn read<P: VmPage>(page: &P, slot: usize) -> u64 {
    page.read_word(slot)
}
/// 硬件共享页内写入完整表项，不创建整页引用；父项发布顺序由调用方控制。
fn write<P: VmPage>(page: &mut P, slot: usize, value: u64) {
    page.write_word(slot, value);
}
/// 本阶段允许的空项、中间项和 4 KiB 叶项。
enum Entry {
    Empty,
    Table(PhysAddr),
    Leaf(PageMapping),
}
/// 本阶段只接受精确的 V、PPN 及 R/RW/RX+A/D；拒绝大页、扩展和软件位。
fn decode(raw: u64, depth: usize) -> Result<Entry, VmError> {
    if raw == 0 {
        return Ok(Entry::Empty);
    }
    if raw >> 54 != 0 || raw & 1 == 0 {
        return Err(VmError::InvalidTable);
    }
    let pa = PhysAddr::from_raw(((raw >> 10) << 12) as usize);
    let flags = raw & 0x3ff;
    if flags == 1 && depth < 2 {
        return Ok(Entry::Table(pa));
    }
    if depth != 2 {
        return Err(VmError::InvalidTable);
    }
    let perm = match flags {
        0xc3 => MapPerm::R,
        0xc7 => MapPerm::RW,
        0xcb => MapPerm::RX,
        _ => return Err(VmError::InvalidTable),
    };
    Ok(Entry::Leaf(PageMapping { phys: pa, perm }))
}
/// 只编码已校验的页首地址与本阶段支持的低位标记。
fn pte(pa: PhysAddr, flags: u64) -> u64 {
    ((pa.as_usize() as u64 >> 12) << 10) | flags
}

impl<C: VmCpu> Sv39<C> {
    /// 每次解引用都通过登记表；循环引用与错误层级不继续访问。
    fn walk<T: TableAccess>(
        &self,
        tables: &T,
        root: PhysAddr,
        va: VirtAddr,
    ) -> Result<Path, VmError> {
        if !self.valid_virt(va) {
            return Err(VmError::InvalidAddress);
        }
        if !self.valid_phys(root) || !root.as_usize().is_multiple_of(PAGE_SIZE) {
            return Err(VmError::InvalidTable);
        }
        let mut path = Path {
            tables: [root; 3],
            slots: [0; 3],
            depth: 0,
            leaf: None,
        };
        for depth in 0..3 {
            path.depth = depth;
            let slot = (va.as_usize() >> (30 - depth * 9)) & 511;
            path.slots[depth] = slot;
            match decode(read(tables.table(path.tables[depth])?, slot), depth)? {
                Entry::Empty => return Ok(path),
                Entry::Leaf(mapping) => {
                    path.leaf = Some(mapping);
                    return Ok(path);
                }
                Entry::Table(pa) => {
                    if path.tables[..=depth].contains(&pa) {
                        return Err(VmError::InvalidTable);
                    }
                    path.tables[depth + 1] = pa;
                }
            }
        }
        Err(VmError::InvalidTable)
    }
    /// 新页不能已连接在其他分支；这里只查表引用，不解释底层数据页。
    fn connected<T: TableAccess>(
        tables: &T,
        root: PhysAddr,
        candidate: PhysAddr,
    ) -> Result<bool, VmError> {
        if root == candidate {
            return Ok(true);
        }
        for i in 0..512 {
            if let Entry::Table(mid) = decode(read(tables.table(root)?, i), 0)? {
                if mid == candidate {
                    return Ok(true);
                }
                if mid == root {
                    return Err(VmError::InvalidTable);
                }
                for j in 0..512 {
                    if let Entry::Table(leaf) = decode(read(tables.table(mid)?, j), 1)?
                        && leaf == candidate
                    {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }
}

// SAFETY: 全部页面访问经登记表，地址/索引/项格式先校验；写入最后发布，CPU 切换读回确认。
unsafe impl<C: VmCpu> VmArch for Sv39<C> {
    fn valid_phys(&self, pa: PhysAddr) -> bool {
        pa.as_usize() < (1usize << 56)
    }
    fn valid_virt(&self, va: VirtAddr) -> bool {
        let value = va.as_usize();
        !((1usize << 38)..(0usize.wrapping_sub(1usize << 38))).contains(&value)
    }
    fn needed_tables<T: TableAccess>(
        &self,
        tables: &T,
        root: PhysAddr,
        va: VirtAddr,
    ) -> Result<usize, VmError> {
        let path = self.walk(tables, root, va)?;
        if path.leaf.is_some() {
            return Err(VmError::AlreadyMapped);
        }
        Ok(2 - path.depth)
    }
    unsafe fn map_page<T: TableAccess>(
        &mut self,
        tables: &mut T,
        root: PhysAddr,
        va: VirtAddr,
        mapping: PageMapping,
        spare: &[PhysAddr],
    ) -> Result<(), VmError> {
        if !self.valid_virt(va) || !self.valid_phys(mapping.phys) {
            return Err(VmError::InvalidAddress);
        }
        if !va.as_usize().is_multiple_of(PAGE_SIZE)
            || !mapping.phys.as_usize().is_multiple_of(PAGE_SIZE)
        {
            return Err(VmError::UnalignedAddress);
        }
        if !mapping.perm.supported() {
            return Err(VmError::UnsupportedPermissions);
        }
        let mut path = self.walk(tables, root, va)?;
        if path.leaf.is_some() {
            return Err(VmError::AlreadyMapped);
        }
        if spare.len() != 2 - path.depth {
            return Err(VmError::InvalidState);
        }
        for (i, pa) in spare.iter().copied().enumerate() {
            let page = tables.table(pa)?;
            if !self.valid_phys(pa)
                || !pa.as_usize().is_multiple_of(PAGE_SIZE)
                || spare[..i].contains(&pa)
                || Self::connected(tables, root, pa)?
                || (0..512).any(|slot| read(page, slot) != 0)
            {
                return Err(VmError::InvalidState);
            }
            tables.table_mut(pa)?;
            path.tables[path.depth + 1 + i] = pa;
        }
        for depth in path.depth..3 {
            tables.table_mut(path.tables[depth])?;
            path.slots[depth] = (va.as_usize() >> (30 - depth * 9)) & 511;
        }
        let flags = 0xc3
            | if mapping.perm.write { 4 } else { 0 }
            | if mapping.perm.execute { 8 } else { 0 };
        // 所有可返回错误的步骤均已结束；先初始化叶项，再由深到浅发布父项。
        write(
            tables.table_mut(path.tables[2]).expect("validated leaf"),
            path.slots[2],
            pte(mapping.phys, flags),
        );
        for depth in (path.depth..2).rev() {
            compiler_fence(Ordering::Release);
            #[cfg(target_arch = "riscv64")]
            // SAFETY: 只排序本核内存访问，不改变寄存器或栈；不能标 nomem。
            unsafe {
                core::arch::asm!("fence rw, rw", options(nostack));
            }
            write(
                tables
                    .table_mut(path.tables[depth])
                    .expect("validated parent"),
                path.slots[depth],
                pte(path.tables[depth + 1], 1),
            );
        }
        Ok(())
    }
    fn query<T: TableAccess>(
        &self,
        tables: &T,
        root: PhysAddr,
        va: VirtAddr,
    ) -> Result<Option<Translation>, VmError> {
        Ok(self.walk(tables, root, va)?.leaf.map(|leaf| Translation {
            phys: PhysAddr::from_raw(leaf.phys.as_usize() + (va.as_usize() & (PAGE_SIZE - 1))),
            perm: leaf.perm,
        }))
    }
    unsafe fn unmap_page<T: TableAccess>(
        &mut self,
        tables: &mut T,
        root: PhysAddr,
        va: VirtAddr,
    ) -> Result<RemovedMapping, VmError> {
        if !self.valid_virt(va) {
            return Err(VmError::InvalidAddress);
        }
        if !va.as_usize().is_multiple_of(PAGE_SIZE) {
            return Err(VmError::UnalignedAddress);
        }
        let path = self.walk(tables, root, va)?;
        let mapping = path.leaf.ok_or(VmError::NotMapped)?;
        let mut retire = [false; 2];
        for depth in (1..=2).rev() {
            let page = tables.table(path.tables[depth])?;
            retire[depth - 1] =
                (0..512).all(|slot| slot == path.slots[depth] || read(page, slot) == 0);
            if depth == 1 {
                retire[0] &= retire[1];
            }
        }
        for pa in path.tables {
            tables.table_mut(pa)?;
        }
        write(
            tables.table_mut(path.tables[2]).expect("validated leaf"),
            path.slots[2],
            0,
        );
        let mut retired = [None; 2];
        for depth in (1..=2).rev() {
            if retire[depth - 1] {
                write(
                    tables
                        .table_mut(path.tables[depth - 1])
                        .expect("validated parent"),
                    path.slots[depth - 1],
                    0,
                );
                retired[2 - depth] = Some(path.tables[depth]);
            }
        }
        Ok(RemovedMapping { mapping, retired })
    }
    fn current_root(&self) -> Result<Option<PhysAddr>, VmError> {
        let satp = self.cpu.read_satp();
        if satp == 0 {
            return Ok(None);
        }
        if satp >> 60 != 8 || (satp >> 44) & 0xffff != 0 {
            return Err(VmError::InvalidState);
        }
        Ok(Some(PhysAddr::from_raw(
            (satp & ((1usize << 44) - 1)) << 12,
        )))
    }
    unsafe fn activate(&mut self, root: PhysAddr) -> Result<(), VmError> {
        if !self.valid_phys(root) {
            return Err(VmError::InvalidAddress);
        }
        if !root.as_usize().is_multiple_of(PAGE_SIZE) {
            return Err(VmError::UnalignedAddress);
        }
        if self.current_root()?.is_some() {
            return Err(VmError::InvalidState);
        }
        let value = (8usize << 60) | (root.as_usize() >> 12);
        // SAFETY: VmArch 调用者保证转换前后执行路径有效，后端检查启动状态。
        unsafe {
            self.cpu.prepare()?;
            self.cpu.fence();
            self.cpu.write_satp(value);
            self.cpu.fence();
        }
        if self.cpu.read_satp() != value {
            // SAFETY: 未能启用时恢复 Bare；不能确认恢复则不允许继续回收页面。
            unsafe {
                self.cpu.fence();
                self.cpu.write_satp(0);
                self.cpu.fence();
            }
            assert_eq!(self.cpu.read_satp(), 0, "cannot restore Bare mode");
            return Err(VmError::ActivationFailed);
        }
        #[cfg(target_arch = "riscv64")]
        crate::hal::console_write_fmt(format_args!(
            "FocOS VM CPU: satp={:#x} mode=Sv39 asid=0 root={:#x}\n",
            self.cpu.read_satp(),
            root.as_usize()
        ));
        Ok(())
    }
    unsafe fn deactivate(&mut self, root: PhysAddr) -> Result<(), VmError> {
        if self.current_root()? != Some(root) {
            return Err(VmError::InvalidState);
        }
        // SAFETY: 当前根已核对；唯一启动核在恒等地址上执行且关中断。
        unsafe {
            self.cpu.fence();
            self.cpu.write_satp(0);
            self.cpu.fence();
        }
        assert_eq!(self.cpu.read_satp(), 0, "cannot confirm Bare mode");
        Ok(())
    }
    unsafe fn sync_translations(&mut self) {
        // SAFETY: 调用者持有全部表页并已完成串行修改。
        unsafe {
            self.cpu.fence();
        }
    }
}
