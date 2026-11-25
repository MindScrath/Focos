//! RISC-V 单次定时事件状态机；真实后端与 host 故障注入共用。
use crate::time::{MonotonicNs, TimerError};
use core::num::NonZeroU32;
pub(crate) const STIE: usize = 1 << 5;
const TIMER_CAUSE: usize = (1usize << (usize::BITS - 1)) | 5;
/// 由架构层读取的启动状态；公共逻辑不读取 CSR。
#[derive(Clone, Copy)]
pub(crate) struct CpuState {
    pub enabled: bool,
    pub sources: usize,
    pub entry_valid: bool,
    pub in_handler: bool,
}
pub(crate) trait Hardware {
    fn ticks(&mut self) -> u64;
    fn supported(&mut self) -> Result<bool, TimerError>;
    fn set_timer(&mut self, ticks: u64) -> Result<(), TimerError>;
    fn source(&mut self, enabled: bool);
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Cold,
    Ready,
    Armed(u64),
    Failed,
}
/// 原生后端将其复制到栈上，仅在 SIE=0 时读写固定共享记录。
#[derive(Clone, Copy)]
pub(crate) struct TimerCore {
    phase: Phase,
    frequency: u32,
    last: u64,
    pub completed: u64,
}
impl TimerCore {
    pub const fn new() -> Self {
        Self {
            phase: Phase::Cold,
            frequency: 1,
            last: 0,
            completed: 0,
        }
    }
    pub fn init(
        &mut self,
        frequency: NonZeroU32,
        cpu: CpuState,
        hw: &mut impl Hardware,
    ) -> Result<(), TimerError> {
        if self.phase != Phase::Cold
            || cpu.enabled
            || cpu.sources != 0
            || !cpu.entry_valid
            || cpu.in_handler
        {
            return Err(TimerError::InvalidState);
        }
        self.phase = Phase::Failed;
        hw.source(false);
        if !hw.supported()? {
            return Err(TimerError::Unsupported);
        }
        hw.set_timer(u64::MAX)?;
        self.frequency = frequency.get();
        self.last = hw.ticks();
        ticks_to_ns(self.last, self.frequency)?;
        self.phase = Phase::Ready;
        Ok(())
    }
    fn active(&self) -> Result<(), TimerError> {
        match self.phase {
            Phase::Ready | Phase::Armed(_) => Ok(()),
            _ => Err(TimerError::InvalidState),
        }
    }
    fn read(&mut self, hw: &mut impl Hardware) -> Result<u64, TimerError> {
        self.active()?;
        let current = hw.ticks();
        if current < self.last {
            self.phase = Phase::Failed;
            hw.source(false);
            return Err(TimerError::InvalidState);
        }
        self.last = current;
        Ok(current)
    }
    pub fn now(&mut self, hw: &mut impl Hardware) -> Result<MonotonicNs, TimerError> {
        let ticks = self.read(hw)?;
        ticks_to_ns(ticks, self.frequency).map(MonotonicNs)
    }
    pub fn validate_cpu(&self, cpu: CpuState) -> Result<(), TimerError> {
        self.active()?;
        if cpu.enabled
            || !cpu.entry_valid
            || cpu.in_handler
            || cpu.sources & !STIE != 0
            || (cpu.sources & STIE != 0 && !matches!(self.phase, Phase::Armed(_)))
        {
            return Err(TimerError::InvalidState);
        }
        Ok(())
    }
    pub fn arm(
        &mut self,
        deadline: MonotonicNs,
        cpu: CpuState,
        hw: &mut impl Hardware,
    ) -> Result<(), TimerError> {
        self.validate_cpu(cpu)?;
        if matches!(self.phase, Phase::Armed(_)) {
            return Err(TimerError::Busy);
        }
        let current = self.read(hw)?;
        let ticks = deadline_ticks(deadline, self.frequency, current)?;
        if let Err(error) = hw.set_timer(ticks) {
            self.phase = Phase::Failed;
            hw.source(false);
            return Err(error);
        }
        self.phase = Phase::Armed(ticks);
        hw.source(true);
        Ok(())
    }
    pub fn cancel(&mut self, cpu: CpuState, hw: &mut impl Hardware) -> Result<(), TimerError> {
        self.validate_cpu(cpu)?;
        hw.source(false);
        if let Err(error) = hw.set_timer(u64::MAX) {
            self.phase = Phase::Failed;
            return Err(error);
        }
        self.phase = Phase::Ready;
        Ok(())
    }
    pub fn interrupt(&mut self, cause: usize, hw: &mut impl Hardware) -> Result<(), TimerError> {
        hw.source(false);
        let deadline = match self.phase {
            Phase::Armed(value) if cause == TIMER_CAUSE => value,
            _ => {
                self.phase = Phase::Failed;
                return Err(TimerError::InvalidState);
            }
        };
        if self.read(hw)? < deadline {
            self.phase = Phase::Failed;
            return Err(TimerError::InvalidState);
        }
        if let Err(error) = hw.set_timer(u64::MAX) {
            self.phase = Phase::Failed;
            return Err(error);
        }
        let Some(next) = self.completed.checked_add(1) else {
            self.phase = Phase::Failed;
            return Err(TimerError::Overflow);
        };
        self.completed = next;
        self.phase = Phase::Ready;
        Ok(())
    }
}
fn ticks_to_ns(ticks: u64, frequency: u32) -> Result<u64, TimerError> {
    u64::try_from(u128::from(ticks) * 1_000_000_000 / u128::from(frequency))
        .map_err(|_| TimerError::Overflow)
}
fn deadline_ticks(deadline: MonotonicNs, frequency: u32, current: u64) -> Result<u64, TimerError> {
    let scaled = u128::from(deadline.0) * u128::from(frequency);
    let ticks = u64::try_from(scaled.div_ceil(1_000_000_000)).map_err(|_| TimerError::Overflow)?;
    if ticks == u64::MAX {
        return Err(TimerError::Overflow);
    }
    if ticks <= current {
        return Err(TimerError::DeadlineElapsed);
    }
    Ok(ticks)
}

#[cfg(test)]
mod tests;

/// 定时异常只来自 S 模式；恢复同一条被打断指令，不按指令长度推进。
pub(crate) fn dispatch(
    core: &mut TimerCore,
    hw: &mut impl Hardware,
    cause: usize,
    pc: usize,
    status: usize,
) -> Result<usize, TimerError> {
    if status & (1 << 8) == 0 || status & 2 != 0 {
        return Err(TimerError::InvalidState);
    }
    core.interrupt(cause, hw)?;
    Ok(pc)
}
