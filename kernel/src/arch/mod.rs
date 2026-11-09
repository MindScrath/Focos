//! 平台接口；开发机测试只编译布局计算，不执行目标机器的汇编。

#[cfg(any(target_arch = "riscv64", test))]
mod riscv64;
#[cfg(any(target_arch = "riscv64", test))]
pub use riscv64::memory;
#[cfg(target_arch = "riscv64")]
pub use riscv64::{console_putchar, shutdown};
