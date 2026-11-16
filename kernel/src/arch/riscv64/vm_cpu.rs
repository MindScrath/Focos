//! 只在唯一启动核使用的 satp、权限状态及本核翻译同步。
use super::paging::{Sv39, VmCpu};
use crate::{hal::VmArch, mm::VmError};
use core::{
    arch::asm,
    sync::atomic::{AtomicBool, Ordering, compiler_fence},
};

static INITIALIZED: AtomicBool = AtomicBool::new(false);
/// 唯一启动核的 CSR 访问句柄；只能由 init_vm_arch 构造。
pub struct NativeCpu {
    _private: (),
}
/// 真实 Sv39 表项操作与本核 CPU 控制的组合。
pub type NativeVmArch = Sv39<NativeCpu>;

/// 取得唯一 CPU 后端；重复初始化不覆盖原状态。
///
/// # Safety
/// 仅启动 hart、关中断且 Bare 模式；不得另建操作同一 CPU 的后端。
pub unsafe fn init_vm_arch() -> Result<NativeVmArch, VmError> {
    if INITIALIZED.load(Ordering::Relaxed) {
        return Err(VmError::InvalidState);
    }
    let arch = Sv39 {
        cpu: NativeCpu { _private: () },
    };
    if super::local_irqs_enabled() || arch.current_root()?.is_some() {
        return Err(VmError::InvalidState);
    }
    INITIALIZED.store(true, Ordering::Relaxed);
    Ok(arch)
}

// SAFETY: 直接读写本核 CSR，并通过带访存约束的 sfence.vma 完成同步。
unsafe impl VmCpu for NativeCpu {
    fn read_satp(&self) -> usize {
        let value;
        // SAFETY: 内核在 S 模式，读取自身分页控制状态。
        unsafe {
            asm!("csrr {}, satp", out(reg) value, options(nostack));
        }
        value
    }
    unsafe fn write_satp(&mut self, value: usize) {
        // SAFETY: 上层完成根、模式和执行路径检查；不标 nomem，约束编译器访存。
        unsafe {
            asm!("csrw satp, {}", in(reg) value, options(nostack));
        }
    }
    unsafe fn prepare(&mut self) -> Result<(), VmError> {
        if super::local_irqs_enabled() || !super::fault::installed() {
            return Err(VmError::InvalidState);
        }
        let status: usize;
        // SAFETY: 单核启动阶段仅清除 SUM/MXR，不启用中断，读回检查。
        unsafe {
            asm!(
                "csrc sstatus, {mask}",
                "csrr {status}, sstatus",
                mask = in(reg) ((1usize << 18) | (1 << 19)),
                status = out(reg) status,
                options(nostack),
            );
        }
        if status & ((1 << 1) | (1 << 18) | (1 << 19)) != 0 {
            return Err(VmError::InvalidState);
        }
        Ok(())
    }
    unsafe fn fence(&mut self) {
        compiler_fence(Ordering::SeqCst);
        // SAFETY: 本核全量同步，不使用 nomem；页表仍由公共层持有。
        unsafe {
            asm!("sfence.vma x0, x0", options(nostack));
        }
        compiler_fence(Ordering::SeqCst);
    }
}
