//! 供公共内核调用的早期平台接口：控制台、关机、本核中断开关与状态自检入口。
//!
//! 具体寄存器与固件调用留在架构层；公共模块通过这里的稳定入口使用。

pub use crate::arch::vm::*;

#[cfg(target_arch = "riscv64")]
pub use crate::arch::{
    IrqCheckError, NativeFaultProbe, NativeTimer, NativeVmArch, TimerCheckError, check_boot_timer,
    check_local_irq_state, init_boot_timer, init_vm_arch, install_fault_probe, local_irqs_enabled,
    probe_return_code, with_local_irqs_disabled,
};

/// 按字节输出字符串，不申请堆内存。
#[cfg(target_arch = "riscv64")]
pub fn console_write(message: &str) {
    for byte in message.bytes() {
        crate::arch::console_putchar(byte);
    }
}

/// 将格式化文本写到控制台，用于输出内存范围和失败原因。
#[cfg(target_arch = "riscv64")]
pub fn console_write_fmt(arguments: core::fmt::Arguments<'_>) {
    let _ = core::fmt::Write::write_fmt(&mut Console, arguments);
}

/// 为 core 的格式化接口提供无堆分配的控制台写入器。
#[cfg(target_arch = "riscv64")]
struct Console;

#[cfg(target_arch = "riscv64")]
impl core::fmt::Write for Console {
    /// 输出格式化过程提供的一段字符串；当前字符输出接口不返回错误。
    fn write_str(&mut self, message: &str) -> core::fmt::Result {
        console_write(message);
        Ok(())
    }
}

/// 请求正常或失败关机，不再返回内核调用点。
#[cfg(target_arch = "riscv64")]
pub fn shutdown(failed: bool) -> ! {
    crate::arch::shutdown(failed)
}

#[cfg(any(target_arch = "riscv64", test))]
pub use crate::time::{MonotonicNs, TimerError, TimerSnapshot};
