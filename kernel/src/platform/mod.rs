//! 板级统一入口；构建脚本选择板型，开发机仅编译布局测试。

#[cfg(any(focos_platform = "qemu_riscv64", test))]
mod qemu_riscv64;

#[cfg(any(focos_platform = "qemu_riscv64", test))]
pub use qemu_riscv64::MemoryInitError;
#[cfg(focos_platform = "qemu_riscv64")]
pub use qemu_riscv64::boot_page_allocator;
