//! MM-003 实际虚拟访问与生命周期验收，通过 HAL 使用架构能力。
use super::*;
use crate::hal::{ExpectedFault, FaultInfo, FaultKind, FaultProbe, ProbeError, ProbeOp, VmArch};
use core::{cell::Cell, ops::Range};

/// 仅供启动验收观察成功申请与归还，不扩充生产 VmMemory 接口。
pub struct ObservedMemory<'a, M> {
    inner: M,
    live: &'a Cell<usize>,
}
impl<'a, M> ObservedMemory<'a, M> {
    /// 从零开始统计此适配器取得的页面，计数器可在地址空间持有期间只读观察。
    pub fn new(inner: M, live: &'a Cell<usize>) -> Self {
        live.set(0);
        Self { inner, live }
    }
}
// SAFETY: 不改变底层页的生命周期或访问路径；只在成功分配/归还后更新外置计数。
unsafe impl<M: VmMemory> VmMemory for ObservedMemory<'_, M> {
    type Page = M::Page;
    fn managed_range(&self) -> Range<PhysAddr> {
        self.inner.managed_range()
    }
    fn alloc_zeroed_page(&mut self) -> Result<Self::Page, PageAllocError> {
        let page = self.inner.alloc_zeroed_page()?;
        self.live.set(self.live.get() + 1);
        Ok(page)
    }
    unsafe fn release_page(
        &mut self,
        page: Self::Page,
    ) -> Result<(), (PageAllocError, Self::Page)> {
        // SAFETY: 原样转交调用者的已撤销、已同步和无引用前提。
        unsafe {
            self.inner.release_page(page)?;
        }
        self.live.set(
            self.live
                .get()
                .checked_sub(1)
                .expect("VM counter underflow"),
        );
        Ok(())
    }
}
/// 验收失败的来源；任何错误都要求调用方终止启动。
#[derive(Debug)]
pub enum VmCheckError {
    /// 地址空间操作失败。
    Vm(VmError),
    /// 探针安装、参数或异常观测失败。
    Probe(ProbeError),
    /// 测试数据页申请或归还失败。
    Allocator(PageAllocError),
    /// 实际内容、状态或计数与验收预期不符。
    Mismatch(&'static str),
}
impl From<VmError> for VmCheckError {
    fn from(e: VmError) -> Self {
        Self::Vm(e)
    }
}
impl From<ProbeError> for VmCheckError {
    fn from(e: ProbeError) -> Self {
        Self::Probe(e)
    }
}
impl From<PageAllocError> for VmCheckError {
    fn from(e: PageAllocError) -> Self {
        Self::Allocator(e)
    }
}

/// 运行分页验收，返回最终活动空间和异常句柄，二者须持有至关机。
///
/// # Safety
/// 单核、关中断、Bare 启动；M 的页面支持恒等访问，布局覆盖当前代码/栈/日志。
/// live 必须是 M 的准确外置计数，初始为零。失败时直接终止启动，不继续复用资源。
/// install 只能安装本核同步入口，返回的探针遵守真实操作及安全恢复约定。
/// return_code 来自同一架构后端，复制到页首并同步取指后能安全返回 Execute 探针。
pub unsafe fn check_paging<'a, M: VmMemory, A: VmArch, F: FaultProbe>(
    memory: &'a mut M,
    arch: &'a mut A,
    layout: KernelLayout,
    live: &Cell<usize>,
    return_code: &[u8],
    install: impl FnOnce() -> Result<F, ProbeError>,
    mut report: impl FnMut(&str),
) -> Result<(KernelAddressSpace<'a, M, A>, F), VmCheckError> {
    ensure(live.get() == 0, "initial page count")?;
    ensure(
        !return_code.is_empty() && return_code.len() <= PAGE_SIZE,
        "probe return code length",
    )?;
    let v0 = layout.test_window.start;
    ensure(
        v0.as_usize()
            .checked_add(4 * PAGE_SIZE)
            .is_some_and(|end| end <= layout.test_window.end.as_usize()),
        "four-page test window",
    )?;
    let v1 = VirtAddr::from_raw(v0.as_usize() + PAGE_SIZE);
    let v2 = VirtAddr::from_raw(v0.as_usize() + 2 * PAGE_SIZE);
    let p_pattern = PagePattern {
        seed: 0x35,
        prefix: &[],
    };
    let q_pattern = PagePattern {
        seed: 0xa7,
        prefix: return_code,
    };
    let mut p = memory.alloc_zeroed_page()?;
    let mut q = memory.alloc_zeroed_page()?;
    p.with_bytes_mut(|bytes| bytes.fill(0x19));
    q.with_bytes_mut(|bytes| fill(bytes, q_pattern));
    ensure(p.phys_addr() != q.phys_addr(), "aliased physical pages")?;
    let mut space = KernelAddressSpace::new(memory, arch, layout.clone())?;
    let base_count = live.get() - 2;
    let mut probe = install()?;
    // SAFETY: 基础映射完整，入口已安装，调用者保证恒等执行路径与关中断。
    unsafe {
        space.activate()?;
    }
    report("FOCOS_VM_ENABLED\n");
    // SAFETY: P/Q 独占持有，窗口在平台布局中独立；每段访问结束才切换别名。
    unsafe {
        space.map_page(v0, p.phys_addr(), MapPerm::RW)?;
        for offset in 0..PAGE_SIZE {
            core::ptr::with_exposed_provenance_mut::<u8>(v0.as_usize() + offset)
                .write_volatile(p_pattern.byte(offset));
        }
        verify_virtual(v0, p_pattern)?;
    }
    verify_page(&p, p_pattern)?;
    report("FOCOS_VM_ALIAS_OK\n");
    // SAFETY: 专用 P 数据页增加只读别名；全部物理路径引用已结束。
    unsafe {
        space.map_page(v1, p.phys_addr(), MapPerm::R)?;
        verify_virtual(v1, p_pattern)?;
    }
    // SAFETY: 写探针只破坏专用测试数据；任何异常未发生都必须失败。
    unsafe {
        expect_fault(
            &mut probe,
            ProbeOp::WriteByte {
                address: v1,
                value: 0xff,
            },
            FaultKind::StorePage,
        )?;
    }
    verify_page(&p, p_pattern)?;
    report("FOCOS_VM_READONLY_OK\n");
    // SAFETY: Q 开头为返回指令；意外允许执行时可以安全回到探针并报告 NotObserved。
    unsafe {
        space.map_page(v2, q.phys_addr(), MapPerm::RW)?;
        probe.sync_instructions();
        expect_fault(
            &mut probe,
            ProbeOp::Execute { address: v2 },
            FaultKind::InstructionPage,
        )?;
    }
    report("FOCOS_VM_NOEXEC_OK\n");
    // SAFETY: V0 的整页访问已结束；撤销返回前会同步，再读只能经异常探针恢复。
    unsafe {
        ensure(
            space.unmap_page(v0)?
                == PageMapping {
                    phys: p.phys_addr(),
                    perm: MapPerm::RW,
                },
            "unmap result",
        )?;
        expect_fault(
            &mut probe,
            ProbeOp::ReadByte { address: v0 },
            FaultKind::LoadPage,
        )?;
    }
    report("FOCOS_VM_UNMAP_OK\n");
    // SAFETY: Q 仍持有，无冲突借用；重映射后必须读到另一页的完整图样。
    unsafe {
        space.map_page(v0, q.phys_addr(), MapPerm::RW)?;
        verify_virtual(v0, q_pattern)?;
    }
    verify_page(&p, p_pattern)?;
    verify_page(&q, q_pattern)?;
    report("FOCOS_VM_REMAP_OK\n");
    // SAFETY: 各段访问均已结束，撤销并同步后临时表可归还，数据页继续持有。
    unsafe {
        for va in [v0, v1, v2] {
            space.unmap_page(va)?;
        }
    }
    ensure(live.get() == base_count + 2, "temporary tables reclaimed")?;
    ensure(
        space
            .query(VirtAddr::from_raw(layout.code.start.as_usize()))?
            .is_some(),
        "base mapping retained",
    )?;
    report("FOCOS_VM_CLEAN_OK\n");
    ensure(
        space.destroy() == Err(VmError::ActiveAddressSpace),
        "active destruction rejected",
    )?;
    // SAFETY: 当前在恒等代码和栈，窗口已全部撤销，仍关中断。
    unsafe {
        space.deactivate()?;
    }
    space.destroy()?;
    drop(space);
    ensure(live.get() == 2, "all owned tables reclaimed")?;
    verify_page(&p, p_pattern)?;
    verify_page(&q, q_pattern)?;
    // SAFETY: 所有临时别名已撤销同步，旧空间销毁；不保留页面引用。
    unsafe {
        release(memory, p)?;
        release(memory, q)?;
    }
    ensure(live.get() == 0, "data pages reclaimed")?;
    let mut final_space = KernelAddressSpace::new(memory, arch, layout)?;
    ensure(live.get() == base_count, "only final base tables remain")?;
    // SAFETY: 同一布局、同一异常入口，已确认 Bare，无窗口引用。
    unsafe {
        final_space.activate()?;
    }
    report("FOCOS_VM_LIFECYCLE_OK\n");
    Ok((final_space, probe))
}

/// 将内容或计数不匹配保留为可定位的验收错误。
fn ensure(ok: bool, detail: &'static str) -> Result<(), VmCheckError> {
    if ok {
        Ok(())
    } else {
        Err(VmCheckError::Mismatch(detail))
    }
}
/// 页首可放架构提供的不透明代码，其余字节随偏移变化以暴露错页和短读写。
#[derive(Clone, Copy)]
struct PagePattern<'a> {
    seed: u8,
    prefix: &'a [u8],
}
impl PagePattern<'_> {
    /// 返回同一图样的指定字节，不在公共层解释指令编码。
    fn byte(self, offset: usize) -> u8 {
        self.prefix
            .get(offset)
            .copied()
            .unwrap_or_else(|| self.seed.wrapping_add(offset as u8))
    }
}
/// 写满测试页；执行同步由探针接口完成。
fn fill(bytes: &mut [u8; PAGE_SIZE], pattern: PagePattern<'_>) {
    for (offset, byte) in bytes.iter_mut().enumerate() {
        *byte = pattern.byte(offset);
    }
}
/// 从页句柄的访问路径逐字节读回，核对虚拟写入或失败探针没有破坏原页。
fn verify_page<P: VmPage>(page: &P, pattern: PagePattern<'_>) -> Result<(), VmCheckError> {
    page.with_bytes(|bytes| {
        for (offset, byte) in bytes.iter().enumerate() {
            // SAFETY: 闭包持有完整有效页，此时不通过虚拟别名访问。
            ensure(
                unsafe { core::ptr::read_volatile(byte) } == pattern.byte(offset),
                "physical page contents",
            )?;
        }
        Ok(())
    })
}
/// 从实际虚拟地址读满一页，禁止用页表查询结果替代访问。
///
/// # Safety
/// 整页映射有效且由调用者持有，没有冲突引用或并发修改。
unsafe fn verify_virtual(address: VirtAddr, pattern: PagePattern<'_>) -> Result<(), VmCheckError> {
    for offset in 0..PAGE_SIZE {
        // SAFETY: 调用者保证整页有效；volatile 强制 CPU 实际读取。
        let actual = unsafe {
            core::ptr::with_exposed_provenance::<u8>(address.as_usize() + offset).read_volatile()
        };
        ensure(actual == pattern.byte(offset), "virtual page contents")?;
    }
    Ok(())
}
/// 用后端标签构造预期，避免公共层猜测指令长度或恢复位置。
fn expected_fault<F: FaultProbe>(probe: &F, op: ProbeOp, kind: FaultKind) -> ExpectedFault {
    let address = match op {
        ProbeOp::ReadByte { address }
        | ProbeOp::WriteByte { address, .. }
        | ProbeOp::Execute { address } => address,
    };
    ExpectedFault {
        kind,
        address,
        site: probe.site(&op),
    }
}

/// 保护验收必须实际观察到完全匹配的异常。
///
/// # Safety
/// 同 FaultProbe::probe，操作目标和意外成功路径均安全。
unsafe fn expect_fault<F: FaultProbe>(
    probe: &mut F,
    op: ProbeOp,
    kind: FaultKind,
) -> Result<(), VmCheckError> {
    let expected = expected_fault(probe, op, kind);
    // SAFETY: 调用者保证探针前提，标签只由后端取得。
    let actual = unsafe { probe.probe(op, expected)? };
    ensure(
        actual
            == FaultInfo {
                kind,
                address: expected.address,
                instruction: expected.site.instruction,
            },
        "fault mismatch",
    )
}
/// 归还数据页；错误直接传播给终止启动的调用方。
///
/// # Safety
/// 页面不再通过任何临时别名访问，所有访问和同步已经完成。
unsafe fn release<M: VmMemory>(memory: &mut M, page: M::Page) -> Result<(), VmCheckError> {
    // SAFETY: 原样转交调用者的释放前提；失败不伪报成功。
    unsafe { memory.release_page(page) }.map_err(|(error, _page)| VmCheckError::Allocator(error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hal::ProbeSite;
    struct Probe {
        result: Result<FaultInfo, ProbeError>,
    }
    // SAFETY: 只用于调用方负例，不执行真实内存操作；结果由用例注入。
    unsafe impl FaultProbe for Probe {
        fn site(&self, _: &ProbeOp) -> ProbeSite {
            ProbeSite {
                instruction: VirtAddr::from_raw(0x800002),
                resume: VirtAddr::from_raw(0x800006),
            }
        }
        unsafe fn probe(&mut self, _: ProbeOp, _: ExpectedFault) -> Result<FaultInfo, ProbeError> {
            self.result
        }
        unsafe fn sync_instructions(&mut self) {}
    }
    #[test]
    fn caller_rejects_missing_and_mismatched_faults() {
        let address = VirtAddr::from_raw(0x900000);
        for result in [
            Err(ProbeError::NotObserved),
            Err(ProbeError::InvalidProbe),
            Err(ProbeError::Busy),
            Ok(FaultInfo {
                kind: FaultKind::LoadPage,
                instruction: VirtAddr::from_raw(0x800002),
                address,
            }),
        ] {
            // SAFETY: 只调用不访问内存的替身；不能把返回或错误种类当成保护生效。
            assert!(
                unsafe {
                    expect_fault(
                        &mut Probe { result },
                        ProbeOp::WriteByte { address, value: 1 },
                        FaultKind::StorePage,
                    )
                }
                .is_err()
            );
        }
        let result = Ok(FaultInfo {
            kind: FaultKind::StorePage,
            instruction: VirtAddr::from_raw(0x800002),
            address,
        });
        // SAFETY: 同上，完整匹配才成功。
        assert!(
            unsafe {
                expect_fault(
                    &mut Probe { result },
                    ProbeOp::WriteByte { address, value: 1 },
                    FaultKind::StorePage,
                )
            }
            .is_ok()
        );
    }
}
