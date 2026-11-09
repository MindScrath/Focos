pub mod memory;

#[cfg(target_arch = "riscv64")]
use core::arch::{asm, global_asm};

#[cfg(target_arch = "riscv64")]
global_asm!(include_str!("entry.S"));

/// 通过 OpenSBI 控制台输出一个字节。
#[cfg(target_arch = "riscv64")]
pub fn console_putchar(byte: u8) {
    // SAFETY: 在 S 模式向 OpenSBI 调用传统控制台接口，无指针参数。
    unsafe {
        asm!(
            "ecall",
            inlateout("a0") usize::from(byte) => _,
            lateout("a1") _,
            in("a7") 1usize,
            options(nostack),
        );
    }
}

/// 请求固件正常或失败关机；不支持关机时停机等待。
#[cfg(target_arch = "riscv64")]
pub fn shutdown(failed: bool) -> ! {
    // SAFETY: SBI SRST 的类型 0 为关机，原因 0/1 分别为正常/失败。
    unsafe {
        asm!(
            "ecall",
            inlateout("a0") 0usize => _,
            inlateout("a1") usize::from(failed) => _,
            in("a6") 0usize,
            in("a7") 0x5352_5354usize,
            options(nostack),
        );
    }
    // 固件不支持 SRST 时停在这里，冒烟脚本将通过超时报告失败。
    loop {
        // SAFETY: 当前 hart 不再执行其他内核逻辑。
        unsafe { asm!("wfi", options(nomem, nostack)) }
    }
}
