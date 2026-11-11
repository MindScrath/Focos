#![no_std]
#![no_main]

mod hal;

use focos_kernel::arch;

#[cfg(not(target_arch = "riscv64"))]
compile_error!("BOOT-001 currently supports only RISC-V; LoongArch is a future milestone");

/// 接收固件启动参数，初始化并自检真实物理页，成功后报告启动完成并关机。
#[unsafe(no_mangle)]
extern "C" fn kernel_main(_hart_id: usize, dtb_start: usize) -> ! {
    // SAFETY: entry.S 只让启动 hart 进入，已关闭中断与分页，a1 保留固件的 DTB 地址。
    // 这是唯一初始化点，当前 QEMU 配置遵循 MM-002，返回后不再创建同区间分配器。
    let (range, mut allocator) = unsafe { arch::memory::boot_page_allocator(dtb_start) }
        .unwrap_or_else(|error| panic!("memory initialization failed: {error:?}"));
    hal::console_write_fmt(format_args!(
        "FocOS memory: [{:#x}, {:#x})\n",
        range.start, range.end,
    ));
    // SAFETY: arch 排除了固件、内核、启动栈、标记和设备树；关闭分页时这些地址就是 RAM。
    // 启动流程独占分配器，自检归还页后不保留引用，也没有其他内核任务访问这些页。
    unsafe { focos_kernel::mm::boot_check::check_pages(&mut allocator) }
        .unwrap_or_else(|error| panic!("memory self-test failed: {error:?}"));
    hal::console_write("FOCOS_MM_OK\n");
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
