//! 使用独立分配计数和数值 CPU 验证用户地址空间的生存期、错误优先级及拷贝前置条件。
use super::*;
use crate::hal::{self, UserAccess, VmArch};

#[test]
fn mm004_user_range_rejection_precedes_allocation() {
    let mut flags = [false; 16];
    // 内核与用户区间分别位于不同的顶层槽位；低地址失败应来自板级下界。
    let mut memory = HostMemory::at(&mut flags, 0x8020_0000 + 3 * PAGE_SIZE);
    let mut arch = backend(memory.stats.clone());
    let layout = memory.layout();
    let allowed = crate::platform::user_address_range();
    assert_eq!(allowed, VirtAddr(0x1_0000)..VirtAddr(0x4000_0000));
    // 字节区间虽不同，只要窗口与用户区间共用一个顶层槽位，就不能共享根项。
    let mut overlap = layout.clone();
    let crosses_slot = allowed.start..VirtAddr(allowed.end.0 + 2 * PAGE_SIZE);
    overlap.test_window = VirtAddr(0x6000_0000)..VirtAddr(0x6000_0000 + PAGE_SIZE);
    assert_eq!(
        KernelBase::new(&mut memory, &mut arch, overlap, crosses_slot).err(),
        Some(VmError::InvalidLayout)
    );
    assert_eq!(memory.stats.calls.get(), 0);
    let mut kernel = KernelBase::new(&mut memory, &mut arch, layout, allowed.clone()).unwrap();
    let calls = memory.stats.calls.get();
    let live = memory.stats.live.get();
    memory.stats.budget.set(Some(0));
    for range in [
        0x1000..0x2000,
        0xf000..0x1_1000,
        0x3fff_f000..0x4000_1000,
        0x1_0000..0x1_0000,
        0x1_0001..0x1_1000,
    ] {
        assert_eq!(
            UserAddressSpace::new(
                &kernel,
                &mut memory,
                &mut arch,
                VirtAddr(range.start)..VirtAddr(range.end)
            )
            .err(),
            Some(VmError::InvalidLayout)
        );
        assert_eq!(memory.stats.calls.get(), calls);
        assert_eq!(memory.stats.live.get(), live);
    }
    // 同一个耗尽的分配器遇到合法区间才会被调用。
    assert_eq!(
        UserAddressSpace::new(&kernel, &mut memory, &mut arch, allowed.clone()).err(),
        Some(VmError::Allocator(PageAllocError::OutOfMemory))
    );
    assert_eq!(memory.stats.calls.get(), calls + 1);
    assert_eq!(memory.stats.live.get(), live);
    memory.stats.budget.set(None);
    let mut user = UserAddressSpace::new(&kernel, &mut memory, &mut arch, allowed).unwrap();
    assert_eq!(memory.stats.live.get(), live + 1);
    user.destroy(&mut memory, &arch).unwrap();
    drop(user);
    kernel.destroy(&mut memory, &arch).unwrap();
    assert_eq!(memory.stats.live.get(), 0);
}

#[test]
fn mm004_direct_switch_and_last_unmap_own_only_private_tables() {
    let mut flags = [false; 32];
    let mut memory = HostMemory::at(&mut flags, 0x8020_3000);
    let stats = memory.stats.clone();
    let mut arch = backend(stats.clone());
    let layout = memory.layout();
    let range = VirtAddr(0x10000)..VirtAddr(0x40000000);
    let mut base = KernelBase::new(&mut memory, &mut arch, layout, range.clone()).unwrap();
    let p = memory.alloc_zeroed_page().unwrap();
    let q = memory.alloc_zeroed_page().unwrap();
    let va = range.start;
    // SAFETY: 单核 CPU 替身；两个数据页分别持有至所有测试映射退役。
    unsafe { base.activate(&mut arch).unwrap() };
    let start = stats.live.get();
    let mut a = UserAddressSpace::new(&base, &mut memory, &mut arch, range.clone()).unwrap();
    let mut b = UserAddressSpace::new(&base, &mut memory, &mut arch, range).unwrap();
    // SAFETY: 仅修改私有测试表，两处 Q 映射均由本测试授权；没有并发访问。
    unsafe {
        a.map_page(&mut memory, &mut arch, va, p.pa, MapPerm::USER_RW)
            .unwrap();
        a.map_page(
            &mut memory,
            &mut arch,
            VirtAddr(va.0 + PAGE_SIZE),
            q.pa,
            MapPerm::USER_R,
        )
        .unwrap();
        b.map_page(&mut memory, &mut arch, va, q.pa, MapPerm::USER_RW)
            .unwrap();
        a.activate(&mut arch).unwrap();
        assert_eq!(a.activate(&mut arch), Err(VmError::InvalidState));
        a.switch_to(&mut b, &mut arch).unwrap();
        assert_eq!(arch.current_root(), Ok(Some(b.root())));
        assert_eq!(b.query(&arch, va).unwrap().unwrap().phys, q.pa);
        assert_eq!(
            b.destroy(&mut memory, &arch),
            Err(VmError::ActiveAddressSpace)
        );
        b.switch_to(&mut a, &mut arch).unwrap();
        assert_eq!(a.query(&arch, va).unwrap().unwrap().phys, p.pa);
        assert_eq!(a.unmap_page(&mut memory, &mut arch, va).unwrap().phys, p.pa);
        assert_eq!(stats.live.get(), start + 6);
        stats.events.borrow_mut().clear();
        a.unmap_page(&mut memory, &mut arch, VirtAddr(va.0 + PAGE_SIZE))
            .unwrap();
        assert_eq!(
            *stats.events.borrow(),
            ["clear", "clear", "clear", "sync", "free", "free"]
        );
        a.deactivate(&mut arch).unwrap();
    }
    a.destroy(&mut memory, &arch).unwrap();
    b.destroy(&mut memory, &arch).unwrap();
    drop((a, b));
    assert_eq!(stats.live.get(), start);
    // SAFETY: 唯一活动根为 base；数值模型的执行栈不依赖它。
    unsafe { base.deactivate(&mut arch).unwrap() };
    base.destroy(&mut memory, &arch).unwrap();
    release(&mut memory, p);
    release(&mut memory, q);
    assert_eq!(stats.live.get(), 0);
}

#[test]
fn mm004_handoff_preserves_active_owner_on_invalid_window_and_range() {
    let mut flags = [false; 32];
    let mut memory = HostMemory::at(&mut flags, 0x8020_3000);
    let stats = memory.stats.clone();
    let mut arch = backend(stats.clone());
    let mut layout = memory.layout();
    // 采用低半区合成窗口，以独立检验与用户区共用一个顶层槽位的拒绝路径。
    layout.test_window = VirtAddr(0x6000_0000)..VirtAddr(0x6000_2000);
    let window = VirtAddr(layout.test_window.start.0 + PAGE_SIZE);
    let allowed = crate::platform::user_address_range();
    let data = memory.alloc_zeroed_page().unwrap();
    let space = KernelAddressSpace::new(&mut memory, &mut arch, layout).unwrap();
    let (err, mut space) = space.into_kernel_base(allowed.clone()).err().unwrap();
    assert_eq!(err, VmError::InvalidState);
    // SAFETY: 单核数值后端；被映射的数据页持续持有，窗口在交接前撤销。
    unsafe {
        space.activate().unwrap();
        space.map_page(window, data.pa, MapPerm::R).unwrap();
    }
    let (err, mut space) = space.into_kernel_base(allowed.clone()).err().unwrap();
    assert_eq!(err, VmError::InvalidState);
    assert_eq!(space.query(window).unwrap().unwrap().phys, data.pa);
    // SAFETY: 无窗口使用者；原对象仍独占旧根和分配服务。
    unsafe {
        space.unmap_page(window).unwrap();
    }
    let before = stats.live.get();
    let (err, space) = space
        .into_kernel_base(allowed.start..VirtAddr(0x4000_1000))
        .err()
        .unwrap();
    assert_eq!(err, VmError::InvalidLayout);
    let old_root = space.root;
    let mut base = space
        .into_kernel_base(allowed.clone())
        .map_err(|(e, _)| e)
        .unwrap();
    assert_eq!(stats.live.get(), before);
    assert_eq!(arch.current_root(), Ok(Some(old_root)));
    assert_eq!(
        base.destroy(&mut memory, &arch),
        Err(VmError::ActiveAddressSpace)
    );
    let mut user = UserAddressSpace::new(&base, &mut memory, &mut arch, allowed).unwrap();
    assert_eq!(user.query(&arch, window), Ok(None));
    user.destroy(&mut memory, &arch).unwrap();
    drop(user);
    // SAFETY: 仅有原基础根活动，数值 CPU 的执行栈不依赖该根。
    unsafe {
        base.deactivate(&mut arch).unwrap();
    }
    base.destroy(&mut memory, &arch).unwrap();
    release(&mut memory, data);
    assert_eq!(stats.live.get(), 0);
}

struct BusyAccess(usize);

#[test]
fn mm004_different_kernel_owners_cannot_switch_and_stale_state_cannot_destroy() {
    let mut flags = [false; 40];
    let mut memory = HostMemory::at(&mut flags, 0x8020_3000);
    let mut arch = backend(memory.stats.clone());
    let layout = memory.layout();
    let range = crate::platform::user_address_range();
    let mut k1 = KernelBase::new(&mut memory, &mut arch, layout.clone(), range.clone()).unwrap();
    let mut k2 = KernelBase::new(&mut memory, &mut arch, layout, range.clone()).unwrap();
    // SAFETY: 双内核基址仅作数值模型；实际状态由唯一测试 CPU 记录。
    unsafe {
        k1.activate(&mut arch).unwrap();
    }
    let mut a = UserAddressSpace::new(&k1, &mut memory, &mut arch, range.clone()).unwrap();
    let mut b = UserAddressSpace::new(&k2, &mut memory, &mut arch, range).unwrap();
    // SAFETY: 未使用任何用户数据；人为改变模拟根验证报错时仍保留所有权。
    unsafe {
        a.activate(&mut arch).unwrap();
        assert_eq!(a.switch_to(&mut b, &mut arch), Err(VmError::InvalidState));
        let active = arch.cpu.satp;
        arch.cpu.satp = 0;
        assert_eq!(
            a.destroy(&mut memory, &arch),
            Err(VmError::ActiveAddressSpace)
        );
        assert_eq!(a.deactivate(&mut arch), Err(VmError::InvalidState));
        arch.cpu.satp = active;
        a.deactivate(&mut arch).unwrap();
        a.destroy(&mut memory, &arch).unwrap();
        b.destroy(&mut memory, &arch).unwrap();
        drop((a, b));
        assert_eq!(
            k2.destroy(&mut memory, &arch),
            Err(VmError::ActiveAddressSpace)
        );
        k1.deactivate(&mut arch).unwrap();
    }
    k1.destroy(&mut memory, &arch).unwrap();
    k2.destroy(&mut memory, &arch).unwrap();
    assert_eq!(memory.stats.live.get(), 0);
}
// SAFETY: 测试后端只返回前置错误、统计入口次数，绝不访问调用者的目标地址或修改缓冲。
unsafe impl UserAccess for BusyAccess {
    unsafe fn probe_user_isolation(
        &mut self,
        _: hal::ProbeOp,
        _: hal::ExpectedFault,
    ) -> Result<hal::FaultInfo, hal::ProbeError> {
        Err(hal::ProbeError::Busy)
    }
    unsafe fn copy_from_user(&mut self, _: VirtAddr, _: &mut [u8]) -> Result<(), UserCopyError> {
        self.0 += 1;
        Err(UserCopyError::Busy)
    }
    unsafe fn copy_to_user(&mut self, _: VirtAddr, _: &[u8]) -> Result<(), UserCopyError> {
        self.0 += 1;
        Err(UserCopyError::Busy)
    }
}

#[test]
fn mm004_copy_preflight_rejects_before_touching_backend_or_buffer() {
    let mut flags = [false; 32];
    let mut memory = HostMemory::at(&mut flags, 0x8020_3000);
    let mut arch = backend(memory.stats.clone());
    let range = crate::platform::user_address_range();
    let layout = memory.layout();
    let mut base = KernelBase::new(&mut memory, &mut arch, layout, range.clone()).unwrap();
    // SAFETY: 唯一数值 CPU，所有调用均不访问真实用户地址。
    unsafe {
        base.activate(&mut arch).unwrap();
    }
    let mut space = UserAddressSpace::new(&base, &mut memory, &mut arch, range.clone()).unwrap();
    let mut access = BusyAccess(0);
    let mut buf = [0x63; 2];
    // SAFETY: 使用不访存的后端和独立栈缓冲；状态注入只改变数值 CPU。
    unsafe {
        assert_eq!(
            space.copy_from_user(&arch, &mut access, VirtAddr(usize::MAX), &mut buf),
            Err(UserCopyError::InvalidState)
        );
        space.activate(&mut arch).unwrap();
        for (addr, error) in [
            (0, UserCopyError::OutsideUserRange),
            (0x1000, UserCopyError::OutsideUserRange),
            (range.end.0, UserCopyError::OutsideUserRange),
            (base.base_range().start.0, UserCopyError::OutsideUserRange),
            (1usize << 38, UserCopyError::InvalidAddress),
        ] {
            for n in [0, 2] {
                assert_eq!(
                    space.copy_from_user(&arch, &mut access, VirtAddr(addr), &mut buf[..n]),
                    Err(error)
                );
                assert_eq!(
                    space.copy_to_user(&arch, &mut access, VirtAddr(addr), &buf[..n]),
                    Err(error)
                );
            }
        }
        assert_eq!(
            space.copy_to_user(&arch, &mut access, VirtAddr(usize::MAX), &buf),
            Err(UserCopyError::InvalidAddress)
        );
        assert_eq!(
            space.copy_from_user(&arch, &mut access, VirtAddr(range.end.0 - 1), &mut buf),
            Err(UserCopyError::OutsideUserRange)
        );
        // 不构造超出 Rust 切片长度上限的假缓冲，通过同一校验器检查横跨规范地址空洞。
        assert_eq!(
            space.validate_copy(&arch, range.start, 0xffff_ffc0_0000_0001 - range.start.0),
            Err(UserCopyError::InvalidAddress)
        );
        assert_eq!(
            space.copy_from_user(&arch, &mut access, range.start, &mut []),
            Ok(())
        );
        assert_eq!(
            space.copy_to_user(&arch, &mut access, range.start, &[]),
            Ok(())
        );
        assert_eq!(access.0, 0);
        assert_eq!(
            space.copy_from_user(&arch, &mut access, range.start, &mut buf),
            Err(UserCopyError::Busy)
        );
        assert_eq!(
            space.copy_to_user(&arch, &mut access, range.start, &buf),
            Err(UserCopyError::Busy)
        );
        assert_eq!(access.0, 2);
        let actual = arch.cpu.satp;
        arch.cpu.satp = 0;
        assert_eq!(
            space.copy_from_user(&arch, &mut access, range.start, &mut buf),
            Err(UserCopyError::InvalidState)
        );
        assert_eq!(
            space.unmap_page(&mut memory, &mut arch, VirtAddr(1)),
            Err(VmError::InvalidState)
        );
        arch.cpu.satp = actual;
        space.deactivate(&mut arch).unwrap();
        space.destroy(&mut memory, &arch).unwrap();
        assert_eq!(
            space.copy_from_user(&arch, &mut access, VirtAddr(0), &mut buf),
            Err(UserCopyError::Destroyed)
        );
    }
    assert_eq!(buf, [0x63; 2]);
    assert_eq!(access.0, 2);
    drop(space);
    // SAFETY: 本核已切回基础根；没有存活的用户根。
    unsafe { base.deactivate(&mut arch).unwrap() };
    base.destroy(&mut memory, &arch).unwrap();
}

#[test]
fn mm004_map_failures_preserve_old_leaves_and_return_each_spare() {
    let mut flags = [false; 80];
    let mut memory = HostMemory::at(&mut flags, 0x8020_3000);
    let stats = memory.stats.clone();
    let mut arch = backend(stats.clone());
    let layout = memory.layout();
    let range = crate::platform::user_address_range();
    let mut base = KernelBase::new(&mut memory, &mut arch, layout, range.clone()).unwrap();
    let mut space = UserAddressSpace::new(&base, &mut memory, &mut arch, range.clone()).unwrap();
    let p = memory.alloc_zeroed_page().unwrap();
    let start = stats.live.get();
    let va = range.start;
    // SAFETY: 数值后端没有开启分页；p 是本测试授权的数据页，直至销毁后才释放。
    unsafe {
        for (bad, err) in [
            (VirtAddr((1usize << 38) + 1), VmError::InvalidAddress),
            (VirtAddr(1), VmError::UnalignedAddress),
            (VirtAddr(0), VmError::ProtectedMapping),
        ] {
            assert_eq!(
                space.map_page(&mut memory, &mut arch, bad, p.pa, MapPerm::R),
                Err(err)
            );
            assert_eq!(space.unmap_page(&mut memory, &mut arch, bad), Err(err));
        }
        for perm in [
            MapPerm::R,
            MapPerm {
                read: false,
                ..MapPerm::USER_RW
            },
            MapPerm {
                read: false,
                ..MapPerm::USER_RX
            },
            MapPerm {
                read: false,
                ..MapPerm::USER_R
            },
            MapPerm {
                write: true,
                ..MapPerm::USER_RX
            },
        ] {
            assert_eq!(
                space.map_page(&mut memory, &mut arch, va, p.pa, perm),
                Err(VmError::UnsupportedPermissions)
            );
        }
        assert_eq!(
            space.unmap_page(&mut memory, &mut arch, va),
            Err(VmError::NotMapped)
        );
        for budget in [0, 1] {
            stats.budget.set(Some(budget));
            assert_eq!(
                space.map_page(&mut memory, &mut arch, va, p.pa, MapPerm::USER_RW),
                Err(VmError::Allocator(PageAllocError::OutOfMemory))
            );
            assert_eq!(space.query(&arch, va), Ok(None));
            assert_eq!(stats.live.get(), start);
        }
        stats.budget.set(None);
        for i in 0..30 {
            space
                .map_page(
                    &mut memory,
                    &mut arch,
                    VirtAddr(va.0 + i * 0x20_0000),
                    p.pa,
                    MapPerm::USER_R,
                )
                .unwrap();
        }
        let full = stats.live.get();
        assert_eq!(
            space.map_page(
                &mut memory,
                &mut arch,
                VirtAddr(va.0 + 30 * 0x20_0000),
                p.pa,
                MapPerm::USER_RW
            ),
            Err(VmError::Allocator(PageAllocError::OutOfMemory))
        );
        assert_eq!(
            space.map_page(&mut memory, &mut arch, va, p.pa, MapPerm::USER_RW),
            Err(VmError::AlreadyMapped)
        );
        assert_eq!(
            space.query(&arch, VirtAddr(va.0 + 1)),
            Ok(Some(Translation {
                phys: PhysAddr(p.pa.0 + 1),
                perm: MapPerm::USER_R
            }))
        );
        assert_eq!(stats.live.get(), full);
    }
    space.destroy(&mut memory, &arch).unwrap();
    drop(space);
    base.destroy(&mut memory, &arch).unwrap();
    release(&mut memory, p);
    assert_eq!(stats.live.get(), 0);
}
