//! SBI TIME 与当前启动 hart；共享状态不借用 NativeTimer 句柄。
use super::timer_model::{CpuState, Hardware, STIE, TimerCore};
use super::{fault, local_irqs_enabled, with_local_irqs_disabled};
use crate::time::{MonotonicNs, TimerError, TimerSnapshot};
use core::sync::atomic::{AtomicUsize, Ordering};
use core::{arch::asm, cell::UnsafeCell, marker::PhantomData, num::NonZeroU32};

struct Shared(UnsafeCell<TimerCore>);
// SAFETY: 仅启动 hart；任务代码在 SIE=0 时复制记录，处理器也保持 SIE=0。
// 汇编重入守卫拒绝处理器嵌套异常，复制的局部记录不跨开启中断存活。
unsafe impl Sync for Shared {}
static CORE: Shared = Shared(UnsafeCell::new(TimerCore::new()));
/// 唯一启动时钟句柄，不可复制、跨核传递或共享；持有至关机。
pub struct NativeTimer {
    _local: PhantomData<*mut ()>,
}

fn cpu() -> CpuState {
    let sources: usize;
    // SAFETY: S 模式只读中断源允许寄存器。
    unsafe {
        asm!("csrr {}, sie", out(reg) sources, options(nostack));
    }
    CpuState {
        enabled: local_irqs_enabled(),
        sources,
        entry_valid: fault::installed(),
        in_handler: fault::in_handler(),
    }
}
fn global(enabled: bool) {
    // SAFETY: 调用处已验证入口、共享状态与本核协议；无 nomem，约束两侧访存。
    unsafe {
        if enabled {
            asm!("csrsi sstatus, 2", options(nostack));
        } else {
            asm!("csrci sstatus, 2", options(nostack));
        }
    }
}
fn sbi(eid: usize, fid: usize, argument: u64) -> Result<usize, TimerError> {
    let error: isize;
    let value: usize;
    // SAFETY: SBI 调用只传数值；TIME 及 BASE 在 RV64 中用 a0 传完整 64 位参数。
    unsafe {
        asm!("ecall", inlateout("a0") argument => error, lateout("a1") value,
             in("a6") fid, in("a7") eid, options(nostack));
    }
    if error == 0 {
        Ok(value)
    } else {
        Err(TimerError::Firmware(error))
    }
}
struct NativeHardware;
impl Hardware for NativeHardware {
    fn ticks(&mut self) -> u64 {
        let ticks: u64;
        // SAFETY: 固定 QEMU/OpenSBI 配置允许 S 模式读取 time，不更改硬件状态。
        unsafe {
            asm!("rdtime {}", out(reg) ticks, options(nostack));
        }
        ticks
    }
    fn supported(&mut self) -> Result<bool, TimerError> {
        sbi(0x10, 3, 0x5449_4d45).map(|v| v != 0)
    }
    fn set_timer(&mut self, ticks: u64) -> Result<(), TimerError> {
        sbi(0x5449_4d45, 0, ticks).map(|_| ())
    }
    fn source(&mut self, enabled: bool) {
        // SAFETY: 只修改 STIE，调用点在单核关中断协议内。
        unsafe {
            if enabled {
                asm!("csrs sie, {}", in(reg) STIE, options(nostack));
            } else {
                asm!("csrc sie, {}", in(reg) STIE, options(nostack));
            }
        }
    }
}
/// 仅在关中断协议内使用；闭包不重新开启 SIE，也不调用其他访问 CORE 的函数。
fn with_core<R>(f: impl FnOnce(&mut TimerCore, &mut NativeHardware) -> R) -> R {
    // SAFETY: 所有调用点均在 SIE=0；复制记录后不保留指向共享存储的引用。
    let mut core = unsafe { CORE.0.get().read() };
    let result = f(&mut core, &mut NativeHardware);
    // SAFETY: SIE 仍为 0，嵌套异常只走失败关机，局部记录不会被并发更新。
    unsafe {
        CORE.0.get().write(core);
    }
    result
}

/// 在共用异常入口和基础页表就绪后初始化单次时钟。
///
/// # Safety
/// 唯一启动 hart，SIE=0、sie=0，拥有有效内核栈与基础映射；频率来自已校验的板级资源。
/// 保持共用 stvec 和固件 TIME 能力有效，不直接改写定时寄存器或交给其他所有者。
pub unsafe fn init_boot_timer(frequency: NonZeroU32) -> Result<NativeTimer, TimerError> {
    let state = cpu();
    global(false);
    with_core(|core, hw| core.init(frequency, state, hw))?;
    Ok(NativeTimer {
        _local: PhantomData,
    })
}
impl NativeTimer {
    /// 返回单调纳秒值；短临界区只保护记录复制和更新。
    pub fn now(&self) -> Result<MonotonicNs, TimerError> {
        with_local_irqs_disabled(|| with_core(|core, hw| core.now(hw)))
    }
    /// 一致读取完成计数与 sip.STIP，不把期限已过当作硬件挂起。
    pub fn snapshot(&self) -> TimerSnapshot {
        with_local_irqs_disabled(|| {
            let pending: usize;
            // SAFETY: 读取本 hart 的硬件挂起状态；不修改定时源。
            unsafe {
                asm!("csrr {}, sip", out(reg) pending, options(nostack));
            }
            with_core(|core, _| TimerSnapshot {
                completed: core.completed,
                pending: pending & STIE != 0,
            })
        })
    }
    /// 预约唯一的未来绝对期限；错误不会替换旧预约。
    ///
    /// # Safety
    /// 唯一启动 hart，SIE=0，入口/映射及固件资源仍属于本服务；不得与其他调用并发。
    pub unsafe fn arm_at(&mut self, deadline: MonotonicNs) -> Result<(), TimerError> {
        let state = cpu();
        if state.enabled || state.in_handler {
            return Err(TimerError::InvalidState);
        }
        with_core(|core, hw| core.arm(deadline, state, hw))
    }
    /// 屏蔽 STIE 并撤销固件预约，不增加完成计数。
    ///
    /// # Safety
    /// 同 arm_at：SIE=0，唯一启动 hart，资源与入口仍有效。
    pub unsafe fn cancel(&mut self) -> Result<(), TimerError> {
        let state = cpu();
        if state.enabled || state.in_handler {
            return Err(TimerError::InvalidState);
        }
        with_core(|core, hw| core.cancel(state, hw))
    }
    /// 显式开启/关闭启动期全局中断；允许本服务的合法事件已挂起。
    ///
    /// # Safety
    /// 独占启动 hart，不在临界区闭包/异常处理器内，无处理器冲突借用；栈和映射有效。
    /// 开启还要求 SIE=0；只有本服务的 STIE 可以开放。关闭允许在服务失败后执行。
    pub unsafe fn set_boot_irqs_enabled(&self, enabled: bool) -> Result<(), TimerError> {
        let state = cpu();
        global(false);
        if !enabled {
            return Ok(());
        }
        with_core(|core, _| core.validate_cpu(state))?;
        // 共享记录已写回且不再借用，允许事件在本函数返回前完成。
        global(true);
        Ok(())
    }
}

/// 汇编入口已保存被打断上下文、设置重入守卫且关闭 SIE。
static LAST_PC: AtomicUsize = AtomicUsize::new(0);
pub(super) fn last_pc() -> usize {
    LAST_PC.load(Ordering::Relaxed)
}
pub(super) fn interrupt(cause: usize, pc: usize, status: usize) -> Result<usize, TimerError> {
    let resume = with_core(|core, hw| super::timer_model::dispatch(core, hw, cause, pc, status))?;
    LAST_PC.store(pc, Ordering::Relaxed);
    Ok(resume)
}
