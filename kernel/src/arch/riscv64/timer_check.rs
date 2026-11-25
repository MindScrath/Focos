//! 有界启动自检：真实单次事件、全寄存器恢复及嵌套临界区内的硬件挂起。
use super::{NativeTimer, local_irqs_enabled, with_local_irqs_disabled};
use crate::{
    hal,
    time::{MonotonicNs, PollBudget, TimerError, TimerSnapshot},
};
use core::{
    arch::global_asm,
    sync::atomic::{AtomicUsize, Ordering},
};

global_asm!(include_str!("timer_check.S"));
#[unsafe(no_mangle)]
static FOCOS_CONTEXT_TEST: AtomicUsize = AtomicUsize::new(0);
/// 自检错误保留固件原因，不能以成功输出代替实际观察。
#[derive(Debug)]
pub enum TimerCheckError {
    Timer(TimerError),
    Timeout(&'static str),
    Mismatch(&'static str),
}
impl From<TimerError> for TimerCheckError {
    fn from(value: TimerError) -> Self {
        Self::Timer(value)
    }
}
fn ensure(value: bool, detail: &'static str) -> Result<(), TimerCheckError> {
    if value {
        Ok(())
    } else {
        Err(TimerCheckError::Mismatch(detail))
    }
}
fn wait(
    timer: &NativeTimer,
    detail: &'static str,
    done: impl Fn(TimerSnapshot) -> bool,
) -> Result<(), TimerCheckError> {
    let mut budget = PollBudget::new(timer.now()?, 100_000_000, 1_000_000)?;
    loop {
        if !budget.advance(timer.now()?) {
            return Err(TimerCheckError::Timeout(detail));
        }
        if done(timer.snapshot()) {
            return Ok(());
        }
        core::hint::spin_loop();
    }
}
fn quiet(timer: &NativeTimer, count: u64) -> Result<(), TimerCheckError> {
    let start = timer.now()?;
    let end = start.0.checked_add(1_000_000).ok_or(TimerError::Overflow)?;
    let mut budget = PollBudget::new(start, 100_000_000, 1_000_000)?;
    loop {
        let now = timer.now()?;
        if !budget.advance(now) {
            return Err(TimerCheckError::Timeout("quiet"));
        }
        ensure(timer.snapshot().completed == count, "duplicate interrupt")?;
        if now.0 >= end {
            return Ok(());
        }
    }
}
/// 调用方必须满足 NativeTimer::arm_at 的安全前提。
unsafe fn arm_short(timer: &mut NativeTimer) -> Result<MonotonicNs, TimerError> {
    super::timer_check_deadline::arm_short(
        timer,
        |timer| timer.now(),
        // SAFETY: 调用方在整个预约期间满足 arm_at 的前提。
        |timer, target| unsafe { timer.arm_at(target) },
    )
}

#[repr(C)]
struct Context {
    gpr: [usize; 32],
    fpr: [u64; 32],
    fcsr: usize,
    status: usize,
    sp: usize,
    before: usize,
}
// 汇编输出布局必须与 Rust 完全一致；字段偏移变化在编译期拒绝。
const _: () = {
    assert!(core::mem::size_of::<Context>() == 544);
    assert!(core::mem::offset_of!(Context, fpr) == 256);
    assert!(core::mem::offset_of!(Context, fcsr) == 512);
    assert!(core::mem::offset_of!(Context, status) == 520);
    assert!(core::mem::offset_of!(Context, sp) == 528);
    assert!(core::mem::offset_of!(Context, before) == 536);
};
unsafe extern "C" {
    fn focos_timer_context_probe(out: *mut Context, fp: usize);
    static focos_timer_context_start: u8;
    static focos_timer_context_end: u8;
}

/// # Safety
/// 初始化后的唯一时钟；SIE=0、基础页表及共用入口一直有效，无其他用户或预约。
/// 自检主动切换中断，不能包在临界区中，也不能从异常处理器调用。
pub unsafe fn check_boot_timer(timer: &mut NativeTimer) -> Result<(), TimerCheckError> {
    ensure(!local_irqs_enabled(), "initial SIE")?;
    // 三次独立预约，每次都等待真实计数；普通 Rust 循环也实际被异步打断。
    for n in 1..=3 {
        let before = timer.now()?;
        // SAFETY: 启动自检独占时钟，SIE=0，且上一轮已完成。
        let target = unsafe { arm_short(timer) }?;
        // SAFETY: 当前由启动自检独占，入口已安装且仅开放本定时源。
        unsafe {
            timer.set_boot_irqs_enabled(true)?;
        }
        wait(timer, "one-shot", |s| s.completed >= n)?;
        ensure(timer.snapshot().completed == n, "one-shot count")?;
        let after = timer.now()?;
        ensure(
            after >= target && after >= before,
            "early or backward clock",
        )?;
        quiet(timer, n)?;
        // SAFETY: 不在临界区/处理器，关闭后为下一轮预约恢复初态。
        unsafe {
            timer.set_boot_irqs_enabled(false)?;
        }
        hal::console_write_fmt(format_args!(
            "FOCOS_TIMER_EVENT n={n} deadline={} observed={}\n",
            target.0, after.0
        ));
    }
    // 先确认 pending，再在已填满寄存器的汇编窗口内开启 SIE，避免测试 setup 被打断。
    for floating in [false, true] {
        let count = timer.snapshot().completed;
        // SAFETY: 启动自检独占时钟，此时 SIE=0。
        unsafe { arm_short(timer) }?;
        wait(timer, "context pending", |s| s.pending)?;
        let mut context = Context {
            gpr: [0; 32],
            fpr: [0; 32],
            fcsr: 0,
            status: 0,
            sp: 0,
            before: 0,
        };
        FOCOS_CONTEXT_TEST.store(1, Ordering::SeqCst);
        // SAFETY: 待处理事件只访问独立共享记录；有效输出对象不被处理器借用。
        // 汇编保存调用者上下文，填入哨兵值，固定次数执行后关闭 SIE 并恢复 ABI。
        unsafe {
            focos_timer_context_probe(&mut context, usize::from(floating));
        }
        FOCOS_CONTEXT_TEST.store(0, Ordering::SeqCst);
        ensure(
            !local_irqs_enabled() && timer.snapshot().completed == count + 1,
            "context delivery",
        )?;
        let pc = super::timer::last_pc();
        ensure(
            (&raw const focos_timer_context_start).addr() <= pc
                && pc <= (&raw const focos_timer_context_end).addr(),
            "context PC window",
        )?;
        for (i, value) in context.gpr.iter().enumerate() {
            let expected = if i == 0 {
                0
            } else if i == 2 {
                context.sp
            } else {
                0x1000 + i
            };
            ensure(*value == expected, "integer register restore")?;
        }
        // sret 把 SPP 清零并把 SPIE 置一；它们不是被打断代码可要求原样保留的位。
        let return_bits = (1 << 8) | (1 << 5);
        ensure(
            context.status & !return_bits == context.before & !return_bits,
            "sstatus restore",
        )?;
        if floating {
            for (i, value) in context.fpr.iter().enumerate() {
                ensure(
                    *value == 0x3ff0_0000_0000_0000 + i as u64,
                    "floating register restore",
                )?;
            }
            ensure(context.fcsr == 0x61, "fcsr restore")?;
        } else {
            ensure(context.status & 0x6000 == 0, "FS off")?;
        }
        hal::console_write_fmt(format_args!(
            "FOCOS_TIMER_CONTEXT fp={} count={} pc={:#x}\n",
            usize::from(floating),
            count + 1,
            pc
        ));
    }
    hal::console_write("FOCOS_TIMER_OK\n");
    for initial in [true, false] {
        // SAFETY: 没有活动预约、STIE=0；设置闭包外初态。
        unsafe {
            timer.set_boot_irqs_enabled(initial)?;
        }
        let count = timer.snapshot().completed;
        with_local_irqs_disabled(|| -> Result<(), TimerCheckError> {
            ensure(!local_irqs_enabled(), "outer SIE")?;
            // SAFETY: SIE=0。只预约/开放 STIE，不改变全局 SIE。
            unsafe { arm_short(timer) }?;
            with_local_irqs_disabled(|| -> Result<(), TimerCheckError> {
                wait(timer, "nested pending", |s| s.pending)?;
                let snapshot = timer.snapshot();
                ensure(
                    !local_irqs_enabled() && snapshot.completed == count,
                    "inner masked",
                )?;
                hal::console_write_fmt(format_args!(
                    "FOCOS_IRQ_TIMER initial={} point=inner count={} pending={} sie=0\n",
                    usize::from(initial),
                    snapshot.completed,
                    usize::from(snapshot.pending)
                ));
                Ok(())
            })?;
            let snapshot = timer.snapshot();
            ensure(
                !local_irqs_enabled() && snapshot.pending && snapshot.completed == count,
                "outer masked",
            )?;
            hal::console_write_fmt(format_args!(
                "FOCOS_IRQ_TIMER initial={} point=outer count={} pending=1 sie=0\n",
                usize::from(initial),
                count
            ));
            Ok(())
        })?;
        if !initial {
            let snapshot = timer.snapshot();
            ensure(
                !local_irqs_enabled() && snapshot.pending && snapshot.completed == count,
                "disabled exit",
            )?;
            hal::console_write_fmt(format_args!(
                "FOCOS_IRQ_TIMER initial=0 point=exit count={count} pending=1 sie=0\n"
            ));
            // SAFETY: 已退出全部临界区，合法预约与入口仍有效；允许 STIP 已挂起。
            unsafe {
                timer.set_boot_irqs_enabled(true)?;
            }
        }
        wait(timer, "unmasked delivery", |s| s.completed > count)?;
        ensure(
            local_irqs_enabled() && timer.snapshot().completed == count + 1,
            "restored SIE delivery",
        )?;
        quiet(timer, count + 1)?;
        hal::console_write_fmt(format_args!(
            "FOCOS_IRQ_TIMER initial={} point=delivered count={} sie=1\n",
            usize::from(initial),
            count + 1
        ));
        // SAFETY: 闭包外关闭，保留句柄供后续分页探针回归。
        unsafe {
            timer.set_boot_irqs_enabled(false)?;
            timer.cancel()?;
        }
    }
    hal::console_write("FOCOS_IRQ_TIMER_OK\n");
    Ok(())
}
