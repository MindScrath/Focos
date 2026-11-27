#![no_std]
#![no_main]

use focos_kernel::hal;

#[cfg(not(target_arch = "riscv64"))]
compile_error!("BOOT-001 currently supports only RISC-V; LoongArch is a future milestone");

/// 接收固件参数，验收真实页、中断状态和内核分页，持有基础页表至关机。
#[unsafe(no_mangle)]
extern "C" fn kernel_main(_hart_id: usize, dtb_start: usize) -> ! {
    // SAFETY: entry.S 只让启动 hart 进入，已关闭中断与分页，a1 保留固件的 DTB 地址。
    // 这是唯一初始化点，当前 QEMU 配置遵循 MM-002，返回后不再创建同区间分配器。
    let (range, mut allocator) = unsafe { focos_kernel::platform::boot_page_allocator(dtb_start) }
        .unwrap_or_else(|error| panic!("memory initialization failed: {error:?}"));
    hal::console_write_fmt(format_args!(
        "FocOS memory: [{:#x}, {:#x})\n",
        range.start, range.end,
    ));
    // SAFETY: 板级实现已将固件、内核镜像、启动栈、分配标记和设备树排除在可分配范围外。
    // entry.S 已将分页控制寄存器 satp 设为 Bare（地址翻译关闭），此处尚未启用分页，
    // 因而代码使用的地址就是物理地址。按 MM-002 的 128 MiB 配置，可分配页均位于
    // QEMU virt 的普通内存（DRAM）区间 [0x80000000, 0x88000000)，
    // 不属于设备寄存器所在的内存映射 I/O（MMIO）地址。因此写入测试图样只改变这些页的内容，
    // 不会覆盖保留区或触发设备操作，随后可从同一物理页读回。
    // 启动流程独占分配器，自检归还页后不保留引用，也没有其他内核任务访问这些页。
    unsafe { focos_kernel::mm::boot_check::check_pages(&mut allocator) }
        .unwrap_or_else(|error| panic!("memory self-test failed: {error:?}"));
    hal::console_write("FOCOS_MM_OK\n");
    // SAFETY: entry.S 已清零当前 hart 的 sie；此处仍由启动流程独占当前 hart，
    // 尚未安装或启用中断源，前面的页分配与内存自检也未修改 sie。
    unsafe { hal::check_local_irq_state() }
        .unwrap_or_else(|error| panic!("local IRQ state check failed: {error:?}"));
    hal::console_write("FOCOS_IRQ_OK\n");
    let layout = focos_kernel::platform::kernel_layout(range);
    hal::console_write_fmt(format_args!("FocOS VM layout: {layout:?}\n"));
    let live = focos_kernel::mm::vm_check::PageCount::default();
    // SAFETY: MM-002 的唯一分配器管理独占 RAM；该范围在 Bare 和基础页表中恒等可访问。
    let mut physical_memory = unsafe { focos_kernel::mm::IdentityMemory::new(allocator) };
    let mut memory = focos_kernel::mm::vm_check::ObservedMemory::new(&mut physical_memory, &live);
    // SAFETY: 仅启动 hart，IRQ 状态自检已恢复 SIE=0，当前仍是 Bare。
    let mut arch = unsafe { hal::init_vm_arch() }
        .unwrap_or_else(|error| panic!("VM CPU initialization failed: {error:?}"));
    // SAFETY: 布局来自实际链接符号；单核关中断，计数对应此唯一服务。
    // 回调在基础页表建好后安装异常入口；最终空间及探针均持有至关机。
    let (address_space, mut fault_probe) = unsafe {
        focos_kernel::mm::vm_check::check_paging(
            &mut memory,
            &mut arch,
            layout,
            &live,
            hal::probe_return_code(),
            || hal::install_fault_probe(),
            hal::console_write,
        )
    }
    .unwrap_or_else(|error| panic!("paging self-test failed: {error:?}"));
    hal::console_write_fmt(format_args!(
        "FocOS VM retained table pages: {}\n",
        live.get()
    ));
    hal::console_write("FOCOS_VM_OK\n");
    let kernel = address_space
        .into_kernel_base(focos_kernel::platform::user_address_range())
        .unwrap_or_else(|(error, _)| panic!("active kernel handoff: {error:?}"));
    // SAFETY: 继续使用同一启动核、计数器、分配服务和同步入口；基础根持续活动。
    unsafe {
        focos_kernel::mm::user_vm_check::check_user_paging(
            &kernel,
            &mut memory,
            &mut arch,
            &mut fault_probe,
            &live,
            hal::probe_return_code(),
            hal::console_write,
        )
    };
    hal::console_write_fmt(format_args!(
        "FocOS USER VM retained table pages: {}\n",
        live.get()
    ));
    hal::console_write("FOCOS_USER_VM_CLEAN_OK\nFOCOS_USER_VM_OK\n");
    hal::console_write("FocOS: early S-mode boot\nFOCOS_BOOT_OK\n");
    hal::shutdown(false)
}

/// 输出失败标记和具体原因，再请求失败关机。
#[panic_handler]
fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    hal::console_write("FOCOS_PANIC\n");
    hal::console_write_fmt(format_args!("{info}\n"));
    hal::shutdown(true)
}
