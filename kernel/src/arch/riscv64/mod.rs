use core::arch::{asm, global_asm};

global_asm!(
    include_str!("entry.S"),
    bootelf = const cfg!(feature = "vf2-bootelf") as usize,
);

#[cfg(feature = "vf2-bootelf")]
pub fn halt() -> ! {
    loop {
        // SAFETY: BOARD-001 已关闭 S 模式中断，当前 hart 不再执行内核工作。
        unsafe { asm!("wfi", options(nomem, nostack)) }
    }
}

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
