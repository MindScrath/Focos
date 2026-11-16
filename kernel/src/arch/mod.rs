//! CPU 架构入口与固件调用，不持有板级内存布局。

pub mod vm;

#[cfg(any(target_arch = "riscv64", test))]
pub(crate) mod riscv64;
#[cfg(target_arch = "riscv64")]
pub use riscv64::{
    IrqCheckError, NativeFaultProbe, NativeVmArch, check_local_irq_state, console_putchar,
    init_vm_arch, install_fault_probe, local_irqs_enabled, probe_return_code, shutdown,
    with_local_irqs_disabled,
};
