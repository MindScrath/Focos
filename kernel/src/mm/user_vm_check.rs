//! MM-004 在单核 S 模式下验收用户页权限、实际字节拷贝和私有表页回收。
use super::{vm_check::PageCount, *};
use crate::hal::{ExpectedFault, FaultInfo, FaultKind, FaultProbe, ProbeOp, UserAccess, VmArch};

const DATA: VirtAddr = VirtAddr::from_raw(0x1_0000);
const READ_ONLY: VirtAddr = VirtAddr::from_raw(0x1_1000);
const CODE: VirtAddr = VirtAddr::from_raw(0x1_2000);
const HOLE: VirtAddr = VirtAddr::from_raw(0x1_3000);
const EDGE: VirtAddr = VirtAddr::from_raw(0x1_4000);
const CROSS: VirtAddr = VirtAddr::from_raw(0x1_4ff8);
const END: VirtAddr = VirtAddr::from_raw(0x1_5000);
const PATTERN_A: &[u8; 16] = b"space A private!";
const PATTERN_B: &[u8; 16] = b"space B sharing!";

fn expect_fault(
    result: Result<(), UserCopyError>,
    kind: FaultKind,
    address: VirtAddr,
    copied: usize,
) {
    assert!(
        matches!(result, Err(UserCopyError::Fault { fault, copied: n })
        if fault.kind == kind && fault.address == address && n == copied),
        "unexpected user copy result: {result:?}"
    );
}

/// 检查三种 S 模式隔离故障；若取指权限错误，目标页也能正常返回。
unsafe fn isolation(access: &mut (impl FaultProbe + UserAccess)) {
    // SAFETY: 测试代码已由本核写好；唯一启动核串行执行，目标不再被修改。
    unsafe { access.sync_instructions() };
    for (op, kind, address) in [
        (
            ProbeOp::ReadByte { address: DATA },
            FaultKind::LoadPage,
            DATA,
        ),
        (
            ProbeOp::WriteByte {
                address: DATA,
                value: 0xfc,
            },
            FaultKind::StorePage,
            DATA,
        ),
        (
            ProbeOp::Execute { address: CODE },
            FaultKind::InstructionPage,
            CODE,
        ),
    ] {
        let site = access.site(&op);
        // SAFETY: 只操作专用数据页或含返回指令的用户代码页；栈、日志及恢复点在内核。
        let fault = unsafe {
            access.probe_user_isolation(
                op,
                ExpectedFault {
                    kind,
                    address,
                    site,
                },
            )
        }
        .unwrap();
        assert_eq!(
            fault,
            FaultInfo {
                kind,
                instruction: site.instruction,
                address
            }
        );
    }
}

/// 验证首字节和跨页故障的进度、未读后缀，以及范围错误的零访存语义。
unsafe fn check_copies<P: VmPage>(
    space: &UserAddressSpace<'_, P>,
    arch: &impl VmArch,
    access: &mut impl UserAccess,
    code: VirtAddr,
    p: &P,
    q: &P,
    r: &P,
) {
    let mut buf = [0u8; 16];
    // SAFETY: 此时只有本核使用活动空间。内核栈与所有用户页独立；物理页内容只在每次拷贝结束后借用。
    unsafe {
        space.copy_to_user(arch, access, DATA, PATTERN_A).unwrap();
        space.copy_from_user(arch, access, DATA, &mut buf).unwrap();
        assert_eq!(&buf, PATTERN_A);
        p.with_bytes(|b| assert_eq!(&b[..16], PATTERN_A));
        space
            .copy_from_user(arch, access, READ_ONLY, &mut buf)
            .unwrap();
        assert_eq!(buf, [0x51; 16]);
        // 独立的栈上见证字节可检测误写；代码段可读且受 RX 保护。
        let mut witness = [0x6bu8; 16];
        let writable = VirtAddr::from_raw((&raw mut witness).addr());
        let code_bytes = core::ptr::with_exposed_provenance::<[u8; 16]>(code.as_usize());
        let code_before = code_bytes.read_volatile();
        for address in [
            code,
            writable,
            VirtAddr::from_raw(0),
            VirtAddr::from_raw(0x1000),
        ] {
            buf.fill(0x93);
            assert_eq!(
                space.copy_from_user(arch, access, address, &mut buf),
                Err(UserCopyError::OutsideUserRange)
            );
            assert_eq!(buf, [0x93; 16]);
            assert_eq!(
                space.copy_to_user(arch, access, address, PATTERN_B),
                Err(UserCopyError::OutsideUserRange)
            );
        }
        assert_eq!(witness, [0x6b; 16]);
        assert_eq!(code_bytes.read_volatile(), code_before);
        expect_fault(
            space.copy_from_user(arch, access, HOLE, &mut buf),
            FaultKind::LoadPage,
            HOLE,
            0,
        );
        assert_eq!(buf, [0; 16]);
        expect_fault(
            space.copy_to_user(arch, access, READ_ONLY, PATTERN_B),
            FaultKind::StorePage,
            READ_ONLY,
            0,
        );
        q.with_bytes(|b| assert!(b.iter().all(|v| *v == 0x51)));
        buf.fill(0x93);
        expect_fault(
            space.copy_from_user(arch, access, CROSS, &mut buf),
            FaultKind::LoadPage,
            END,
            8,
        );
        assert_eq!(&buf[..8], &[0x72; 8]);
        assert_eq!(&buf[8..], &[0; 8]);
        expect_fault(
            space.copy_to_user(arch, access, CROSS, PATTERN_B),
            FaultKind::StorePage,
            END,
            8,
        );
        r.with_bytes(|b| {
            assert!(b[..PAGE_SIZE - 8].iter().all(|v| *v == 0x72));
            assert_eq!(&b[PAGE_SIZE - 8..], &PATTERN_B[..8]);
        });
    }
}

/// 复用 MM-003 活动根和唯一同步异常入口，返回时基础根仍活动。
///
/// # Safety
/// 仅启动核，关中断，kernel 为已确认的活动根。memory 与 live 对应同一分配服务；
/// 基础代码区可读，栈和日志始终映射，return_code 为当前架构的安全返回指令。失败终止启动。
pub unsafe fn check_user_paging<M: VmMemory>(
    kernel: &KernelBase<M::Page>,
    memory: &mut M,
    arch: &mut impl VmArch,
    access: &mut (impl FaultProbe + UserAccess),
    live: &PageCount,
    return_code: &[u8],
    mut report: impl FnMut(&str),
) {
    let baseline = live.get();
    assert_eq!(arch.current_root(), Ok(Some(kernel.root())));
    let range = kernel.allowed_user_range();
    let mut p = memory.alloc_zeroed_page().unwrap();
    let mut q = memory.alloc_zeroed_page().unwrap();
    let mut r = memory.alloc_zeroed_page().unwrap();
    let mut c = memory.alloc_zeroed_page().unwrap();
    p.with_bytes_mut(|b| b.fill(0x37));
    q.with_bytes_mut(|b| b.fill(0x51));
    r.with_bytes_mut(|b| b.fill(0x72));
    assert!(!return_code.is_empty() && return_code.len() <= PAGE_SIZE);
    c.with_bytes_mut(|b| {
        b.fill(0x34);
        b[..return_code.len()].copy_from_slice(return_code);
    });
    let pages = [
        (DATA, p.phys_addr(), MapPerm::USER_RW),
        (READ_ONLY, q.phys_addr(), MapPerm::USER_R),
        (CODE, c.phys_addr(), MapPerm::USER_RX),
        (EDGE, r.phys_addr(), MapPerm::USER_RW),
    ];
    let mut a = UserAddressSpace::new(kernel, memory, arch, range.clone()).unwrap();
    let mut b = UserAddressSpace::new(kernel, memory, arch, range).unwrap();
    // SAFETY: P、Q、R、C 均为自检专用 RAM；仅 Q 在两空间间显式共享。四页持续持有，
    // 无栈或表页别名。内核执行路径由两根共享，数据引用不跨越切根与退役操作。
    unsafe {
        for (va, pa, perm) in pages {
            a.map_page(memory, arch, va, pa, perm).unwrap();
        }
        b.map_page(memory, arch, DATA, q.phys_addr(), MapPerm::USER_RW)
            .unwrap();
        assert_eq!(live.get(), baseline + 10);
        a.activate(arch).unwrap();
        assert_eq!(arch.current_root(), Ok(Some(a.root())));
        isolation(access);
        p.with_bytes(|bytes| assert!(bytes.iter().all(|v| *v == 0x37)));
        report("FOCOS_USER_VM_ISOLATION_OK\n");
        check_copies(&a, arch, access, kernel.base_range().start, &p, &q, &r);
        report("FOCOS_USER_VM_COPY_OK\n");
        a.switch_to(&mut b, arch).unwrap();
        assert_eq!(arch.current_root(), Ok(Some(b.root())));
        let mut buf = [0; 16];
        b.copy_from_user(arch, access, DATA, &mut buf).unwrap();
        assert_eq!(buf, [0x51; 16]);
        assert_eq!(b.query(arch, DATA).unwrap().unwrap().phys, q.phys_addr());
        b.copy_to_user(arch, access, DATA, PATTERN_B).unwrap();
        q.with_bytes(|bytes| assert_eq!(&bytes[..16], PATTERN_B));
        p.with_bytes(|bytes| assert_eq!(&bytes[..16], PATTERN_A));
        assert_eq!(b.destroy(memory, arch), Err(VmError::ActiveAddressSpace));
        b.switch_to(&mut a, arch).unwrap();
        a.copy_from_user(arch, access, DATA, &mut buf).unwrap();
        assert_eq!(&buf, PATTERN_A);
        report("FOCOS_USER_VM_SWITCH_OK\n");
        a.deactivate(arch).unwrap();
        assert_eq!(arch.current_root(), Ok(Some(kernel.root())));
        for (i, (va, phys, perm)) in pages.into_iter().enumerate() {
            assert_eq!(
                a.unmap_page(memory, arch, va),
                Ok(PageMapping { phys, perm })
            );
            assert_eq!(live.get(), baseline + if i == 3 { 8 } else { 10 });
        }
        a.destroy(memory, arch).unwrap();
        assert_eq!(live.get(), baseline + 7);
        b.destroy(memory, arch).unwrap();
        drop((a, b));
        assert_eq!(live.get(), baseline + 4);
        for page in [p, q, r, c] {
            memory
                .release_page(page)
                .unwrap_or_else(|(e, _)| panic!("user self-test release: {e:?}"));
        }
    }
    assert_eq!(live.get(), baseline);
}
