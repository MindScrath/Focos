//! 初期启动接口。设备、内存和中断契约在相应 spec 中逐步增加。

/// 按字节输出字符串，不申请堆内存。
pub fn console_write(message: &str) {
    for byte in message.bytes() {
        crate::arch::console_putchar(byte);
    }
}

/// 将格式化文本写到控制台，用于输出内存范围和失败原因。
pub fn console_write_fmt(arguments: core::fmt::Arguments<'_>) {
    let _ = core::fmt::Write::write_fmt(&mut Console, arguments);
}

/// 为 core 的格式化接口提供无堆分配的控制台写入器。
struct Console;

impl core::fmt::Write for Console {
    /// 输出格式化过程提供的一段字符串；当前字符输出接口不返回错误。
    fn write_str(&mut self, message: &str) -> core::fmt::Result {
        console_write(message);
        Ok(())
    }
}

/// 请求正常或失败关机，不再返回内核调用点。
pub fn shutdown(failed: bool) -> ! {
    crate::arch::shutdown(failed)
}
