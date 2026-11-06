//! 初期启动接口。设备、内存和中断契约在相应 spec 中逐步增加。

pub fn console_write(message: &str) {
    for byte in message.bytes() {
        crate::arch::console_putchar(byte);
    }
}

pub fn shutdown(failed: bool) -> ! {
    crate::arch::shutdown(failed)
}

#[cfg(feature = "vf2-bootelf")]
pub fn halt() -> ! {
    crate::arch::halt()
}
