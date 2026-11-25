//! 公共时钟单位、错误与有限轮询预算；不包含 CSR 或 SBI 编码。

/// 从硬件时间原点计起的纳秒数。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct MonotonicNs(pub u64);
/// 关中断后一次读取的完成计数与硬件挂起状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimerSnapshot {
    pub completed: u64,
    pub pending: bool,
}
/// 预约失败不隐式替换已有期限；固件失败后服务不可重新预约。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerError {
    InvalidState,
    Unsupported,
    Overflow,
    DeadlineElapsed,
    Busy,
    Firmware(isize),
}

/// 每次轮询同时消耗次数和硬件时间预算；时间不动也会终止。
pub(crate) struct PollBudget {
    end: u64,
    last: u64,
    remaining: u32,
}
impl PollBudget {
    pub fn new(now: MonotonicNs, duration: u64, polls: u32) -> Result<Self, TimerError> {
        Ok(Self {
            end: now.0.checked_add(duration).ok_or(TimerError::Overflow)?,
            last: now.0,
            remaining: polls,
        })
    }
    pub fn advance(&mut self, now: MonotonicNs) -> bool {
        if self.remaining == 0 || now.0 < self.last || now.0 >= self.end {
            return false;
        }
        self.remaining -= 1;
        self.last = now.0;
        true
    }
}
