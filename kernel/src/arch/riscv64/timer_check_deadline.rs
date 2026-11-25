//! 启动自检的短期限预约，开发机可注入时钟和固件结果。
use crate::time::{MonotonicNs, TimerError};

const DELAY_NS: u64 = 2_000_000;
const MAX_ATTEMPTS: usize = 3;

pub(super) fn arm_short<T>(
    timer: &mut T,
    now: impl Fn(&mut T) -> Result<MonotonicNs, TimerError>,
    mut arm_at: impl FnMut(&mut T, MonotonicNs) -> Result<(), TimerError>,
) -> Result<MonotonicNs, TimerError> {
    for _ in 0..MAX_ATTEMPTS {
        let target = MonotonicNs(
            now(timer)?
                .0
                .checked_add(DELAY_NS)
                .ok_or(TimerError::Overflow)?,
        );
        match arm_at(timer, target) {
            Ok(()) => return Ok(target),
            // 宿主机调度可能使刚计算的目标过期，此时固件尚未收到预约。
            Err(TimerError::DeadlineElapsed) => continue,
            Err(error) => return Err(error),
        }
    }
    Err(TimerError::DeadlineElapsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elapsed_deadline_uses_fresh_time_and_returns_armed_target() {
        let mut time = 10_000_000;
        let mut attempted = 0;
        let target = arm_short(
            &mut time,
            |time| Ok(MonotonicNs(*time)),
            |time, target| {
                attempted += 1;
                assert_eq!(target.0, *time + DELAY_NS);
                if attempted == 1 {
                    *time += 3_000_000;
                    Err(TimerError::DeadlineElapsed)
                } else {
                    Ok(())
                }
            },
        );
        assert_eq!(target, Ok(MonotonicNs(15_000_000)));
        assert_eq!(attempted, 2);
    }

    #[test]
    fn repeated_elapsed_deadlines_stop_at_attempt_limit() {
        let mut attempts = 0;
        let result = arm_short(
            &mut (),
            |_| Ok(MonotonicNs(10_000_000)),
            |_, _| {
                attempts += 1;
                Err(TimerError::DeadlineElapsed)
            },
        );
        assert_eq!(result, Err(TimerError::DeadlineElapsed));
        assert_eq!(attempts, MAX_ATTEMPTS);
    }

    #[test]
    fn firmware_error_is_returned_without_retry() {
        let mut attempts = 0;
        let result = arm_short(
            &mut (),
            |_| Ok(MonotonicNs(10_000_000)),
            |_, _| {
                attempts += 1;
                Err(TimerError::Firmware(-2))
            },
        );
        assert_eq!(result, Err(TimerError::Firmware(-2)));
        assert_eq!(attempts, 1);
    }
}
