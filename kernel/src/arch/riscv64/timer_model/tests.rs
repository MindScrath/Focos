use super::*;
use crate::time::PollBudget;
extern crate std;
use std::vec::Vec;
struct Fake {
    tick: u64,
    supported: bool,
    error: Option<isize>,
    source: bool,
    calls: Vec<u64>,
}
impl Fake {
    fn new() -> Self {
        Self {
            tick: 15,
            supported: true,
            error: None,
            source: false,
            calls: Vec::new(),
        }
    }
}
impl Hardware for Fake {
    fn ticks(&mut self) -> u64 {
        self.tick
    }
    fn supported(&mut self) -> Result<bool, TimerError> {
        Ok(self.supported)
    }
    fn set_timer(&mut self, value: u64) -> Result<(), TimerError> {
        self.calls.push(value);
        self.error.map_or(Ok(()), |e| Err(TimerError::Firmware(e)))
    }
    fn source(&mut self, enabled: bool) {
        self.source = enabled;
    }
}
fn cpu() -> CpuState {
    CpuState {
        enabled: false,
        sources: 0,
        entry_valid: true,
        in_handler: false,
    }
}
fn ready() -> (TimerCore, Fake) {
    let mut state = TimerCore::new();
    let mut hw = Fake::new();
    state
        .init(NonZeroU32::new(10_000_000).unwrap(), cpu(), &mut hw)
        .unwrap();
    (state, hw)
}
#[test]
fn rounding_matches_spec_and_rejects_reserved_or_overflowed_deadlines() {
    assert_eq!(ticks_to_ns(15, 10_000_000), Ok(1500));
    assert_eq!(deadline_ticks(MonotonicNs(1501), 10_000_000, 15), Ok(16));
    assert_eq!(
        deadline_ticks(MonotonicNs(1500), 10_000_000, 15),
        Err(TimerError::DeadlineElapsed)
    );
    assert_eq!(
        deadline_ticks(MonotonicNs(u64::MAX), 1_000_000_000, 0),
        Err(TimerError::Overflow)
    );
    assert_eq!(
        deadline_ticks(MonotonicNs(u64::MAX), u32::MAX, 0),
        Err(TimerError::Overflow)
    );
    assert_eq!(ticks_to_ns(u64::MAX, 1), Err(TimerError::Overflow));
    assert_eq!(ticks_to_ns(1, 3), Ok(333_333_333));
}
#[test]
fn one_shot_disarms_before_count_and_can_be_rearmed() {
    let (mut state, mut hw) = ready();
    for n in 1..=3 {
        state
            .arm(MonotonicNs((hw.tick + 2) * 100), cpu(), &mut hw)
            .unwrap();
        assert!(hw.source);
        hw.tick += 2;
        state
            .interrupt((1usize << (usize::BITS - 1)) | 5, &mut hw)
            .unwrap();
        assert!(!hw.source);
        assert_eq!(state.completed, n);
        assert_eq!(hw.calls.last(), Some(&u64::MAX));
    }
}
#[test]
fn busy_and_bad_deadlines_do_not_replace_existing_event() {
    let (mut state, mut hw) = ready();
    for deadline in [MonotonicNs(0), MonotonicNs(1500)] {
        assert_eq!(
            state.arm(deadline, cpu(), &mut hw),
            Err(TimerError::DeadlineElapsed)
        );
    }
    assert_eq!(hw.calls.len(), 1);
    state.arm(MonotonicNs(2000), cpu(), &mut hw).unwrap();
    assert_eq!(
        state.arm(MonotonicNs(3000), cpu(), &mut hw),
        Err(TimerError::Busy)
    );
    assert_eq!(hw.calls.len(), 2);
    assert!(hw.source);
}
#[test]
fn firmware_failure_never_publishes_completion_or_allows_rearm() {
    for during_interrupt in [false, true] {
        let (mut state, mut hw) = ready();
        if during_interrupt {
            state.arm(MonotonicNs(2000), cpu(), &mut hw).unwrap();
        }
        hw.error = Some(-4);
        hw.tick = 20;
        let result = if during_interrupt {
            state.interrupt((1usize << (usize::BITS - 1)) | 5, &mut hw)
        } else {
            state.arm(MonotonicNs(3000), cpu(), &mut hw)
        };
        assert_eq!(result, Err(TimerError::Firmware(-4)));
        assert!(!hw.source);
        assert_eq!(state.completed, 0);
        assert_eq!(
            state.arm(MonotonicNs(4000), cpu(), &mut hw),
            Err(TimerError::InvalidState)
        );
    }
}
#[test]
fn cpu_prerequisites_and_duplicate_initialization_are_checked() {
    let (mut state, mut hw) = ready();
    assert_eq!(
        state.init(NonZeroU32::new(1).unwrap(), cpu(), &mut hw),
        Err(TimerError::InvalidState)
    );
    for bad in [
        CpuState {
            enabled: true,
            ..cpu()
        },
        CpuState {
            sources: 2,
            ..cpu()
        },
        CpuState {
            entry_valid: false,
            ..cpu()
        },
        CpuState {
            in_handler: true,
            ..cpu()
        },
    ] {
        assert_eq!(
            state.arm(MonotonicNs(3000), bad, &mut hw),
            Err(TimerError::InvalidState)
        );
        let mut fresh = TimerCore::new();
        assert_eq!(
            fresh.init(NonZeroU32::new(1).unwrap(), bad, &mut hw),
            Err(TimerError::InvalidState)
        );
    }
    assert_eq!(hw.calls.len(), 1);
    assert_eq!(
        state.validate_cpu(CpuState {
            sources: STIE,
            ..cpu()
        }),
        Err(TimerError::InvalidState)
    );
    state.arm(MonotonicNs(3000), cpu(), &mut hw).unwrap();
    assert!(
        state
            .validate_cpu(CpuState {
                sources: STIE,
                ..cpu()
            })
            .is_ok()
    );
}
#[test]
fn unsupported_and_init_firmware_errors_are_not_silent_fallbacks() {
    let mut state = TimerCore::new();
    let mut hw = Fake::new();
    hw.supported = false;
    assert_eq!(
        state.init(NonZeroU32::new(1).unwrap(), cpu(), &mut hw),
        Err(TimerError::Unsupported)
    );
    assert!(hw.calls.is_empty());
    assert!(!hw.source);
    let mut state = TimerCore::new();
    hw.supported = true;
    hw.error = Some(-2);
    assert_eq!(
        state.init(NonZeroU32::new(1).unwrap(), cpu(), &mut hw),
        Err(TimerError::Firmware(-2))
    );
}
#[test]
fn cancel_is_idempotent_and_does_not_count() {
    let (mut state, mut hw) = ready();
    for _ in 0..2 {
        state.cancel(cpu(), &mut hw).unwrap();
    }
    state.arm(MonotonicNs(3000), cpu(), &mut hw).unwrap();
    state
        .cancel(
            CpuState {
                sources: STIE,
                ..cpu()
            },
            &mut hw,
        )
        .unwrap();
    assert_eq!(state.completed, 0);
    assert!(!hw.source);
}
#[test]
fn unexpected_early_or_duplicate_interrupt_and_counter_wrap_fail() {
    for cause in [5, (1usize << (usize::BITS - 1)) | 9] {
        let (mut state, mut hw) = ready();
        state.arm(MonotonicNs(2000), cpu(), &mut hw).unwrap();
        hw.tick = 20;
        assert_eq!(
            state.interrupt(cause, &mut hw),
            Err(TimerError::InvalidState)
        );
        assert_eq!(state.completed, 0);
    }
    let (mut state, mut hw) = ready();
    let cause = (1usize << (usize::BITS - 1)) | 5;
    assert_eq!(
        state.interrupt(cause, &mut hw),
        Err(TimerError::InvalidState)
    );
    let (mut state, mut hw) = ready();
    state.arm(MonotonicNs(2000), cpu(), &mut hw).unwrap();
    assert_eq!(
        state.interrupt(cause, &mut hw),
        Err(TimerError::InvalidState)
    );
    let (mut state, mut hw) = ready();
    state.arm(MonotonicNs(2000), cpu(), &mut hw).unwrap();
    hw.tick = 20;
    state.completed = u64::MAX;
    assert_eq!(state.interrupt(cause, &mut hw), Err(TimerError::Overflow));
    assert_eq!(state.completed, u64::MAX);
}
#[test]
fn backward_time_is_rejected() {
    let (mut state, mut hw) = ready();
    assert_eq!(state.now(&mut hw), Ok(MonotonicNs(1500)));
    hw.tick = 14;
    assert_eq!(state.now(&mut hw), Err(TimerError::InvalidState));
    assert!(!hw.source);
}

#[test]
fn polling_terminates_on_stalled_clock_time_deadline_and_backward_time() {
    let mut budget = PollBudget::new(MonotonicNs(10), 100, 2).unwrap();
    assert!(budget.advance(MonotonicNs(10)));
    assert!(budget.advance(MonotonicNs(10)));
    assert!(!budget.advance(MonotonicNs(10)));
    let mut budget = PollBudget::new(MonotonicNs(10), 100, 2).unwrap();
    assert!(!budget.advance(MonotonicNs(110)));
    let mut budget = PollBudget::new(MonotonicNs(10), 100, 2).unwrap();
    assert!(!budget.advance(MonotonicNs(9)));
    assert!(PollBudget::new(MonotonicNs(u64::MAX), 1, 2).is_err());
}

#[test]
fn dispatch_preserves_exact_pc_and_rejects_user_or_enabled_handler() {
    for pc in [0x80200000, 0x80200002, 0x8020fffe] {
        let (mut state, mut hw) = ready();
        state.arm(MonotonicNs(2000), cpu(), &mut hw).unwrap();
        hw.tick = 20;
        let cause = (1usize << (usize::BITS - 1)) | 5;
        assert_eq!(dispatch(&mut state, &mut hw, cause, pc, 1 << 8), Ok(pc));
        assert_eq!(
            state.interrupt(cause, &mut hw),
            Err(TimerError::InvalidState)
        );
    }
    for status in [0, 2, (1 << 8) | 2] {
        let (mut state, mut hw) = ready();
        state.arm(MonotonicNs(2000), cpu(), &mut hw).unwrap();
        hw.tick = 20;
        assert_eq!(
            dispatch(
                &mut state,
                &mut hw,
                (1usize << (usize::BITS - 1)) | 5,
                0x80200000,
                status
            ),
            Err(TimerError::InvalidState)
        );
        assert_eq!(state.completed, 0);
    }
}
#[test]
fn cancel_failure_keeps_service_failed_and_count_unchanged() {
    let (mut state, mut hw) = ready();
    state.arm(MonotonicNs(2000), cpu(), &mut hw).unwrap();
    hw.error = Some(-1);
    assert_eq!(
        state.cancel(
            CpuState {
                sources: STIE,
                ..cpu()
            },
            &mut hw
        ),
        Err(TimerError::Firmware(-1))
    );
    assert!(!hw.source);
    assert_eq!(state.completed, 0);
    hw.error = None;
    assert_eq!(
        state.arm(MonotonicNs(3000), cpu(), &mut hw),
        Err(TimerError::InvalidState)
    );
}
