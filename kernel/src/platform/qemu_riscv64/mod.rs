//! QEMU virt 的固定资源与启动内存接入，不代表其他 RISC-V 板型。

mod memory;
mod timebase;
pub use timebase::{TimebaseError, parse_timebase_frequency};

pub use memory::MemoryInitError;
#[cfg(focos_platform = "qemu_riscv64")]
pub use memory::{boot_page_allocator, kernel_layout};

#[cfg(focos_platform = "qemu_riscv64")]
pub use timebase::boot_timebase_frequency;
