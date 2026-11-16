//! 同步页异常的单次匹配记录；真实入口与开发机注入共用该匹配器。
use crate::hal::{ExpectedFault, FaultInfo, FaultKind, ProbeError, ProbeOp, ProbeSite};
use crate::mm::VirtAddr;

/// 一次探针的预期与结果；汇编操作期间不保留指向该记录的引用。
#[derive(Clone, Copy)]
struct Record {
    expected: Option<ExpectedFault>,
    observed: Option<FaultInfo>,
}
impl Record {
    /// 初始没有任何可恢复异常。
    const fn new() -> Self {
        Self {
            expected: None,
            observed: None,
        }
    }
    /// 只登记与操作和后端标签一致的预期；忙碌或参数错误不覆盖旧记录。
    fn arm(
        &mut self,
        op: ProbeOp,
        expected: ExpectedFault,
        site: ProbeSite,
    ) -> Result<(), ProbeError> {
        if self.expected.is_some() || self.observed.is_some() {
            return Err(ProbeError::Busy);
        }
        let (kind, address) = match op {
            ProbeOp::ReadByte { address } => (FaultKind::LoadPage, address),
            ProbeOp::WriteByte { address, .. } => (FaultKind::StorePage, address),
            ProbeOp::Execute { address } => (FaultKind::InstructionPage, address),
        };
        if expected.kind != kind || expected.address != address || expected.site != site {
            return Err(ProbeError::InvalidProbe);
        }
        self.expected = Some(expected);
        Ok(())
    }
    /// 三项完全匹配才消费预期并返回恢复标签；不匹配或重复异常交给入口关机。
    fn observe(&mut self, cause: usize, pc: usize, address: usize) -> Result<usize, ()> {
        let kind = match cause {
            12 => FaultKind::InstructionPage,
            13 => FaultKind::LoadPage,
            15 => FaultKind::StorePage,
            _ => return Err(()),
        };
        let expected = self.expected.ok_or(())?;
        if self.observed.is_some()
            || kind != expected.kind
            || pc != expected.site.instruction.as_usize()
            || address != expected.address.as_usize()
        {
            return Err(());
        }
        self.expected = None;
        self.observed = Some(FaultInfo {
            kind,
            instruction: VirtAddr::from_raw(pc),
            address: VirtAddr::from_raw(address),
        });
        Ok(expected.site.resume.as_usize())
    }
    /// 消费结果并清空状态；正常操作没有异常时返回 NotObserved。
    fn finish(&mut self) -> Result<FaultInfo, ProbeError> {
        self.expected = None;
        self.observed.take().ok_or(ProbeError::NotObserved)
    }
}

/// 拒绝重复安装、无效直接模式地址或读回不一致。
fn validate_install(already: bool, requested: usize, actual: usize) -> Result<(), ProbeError> {
    if already {
        return Err(ProbeError::AlreadyInstalled);
    }
    if requested == 0 || !requested.is_multiple_of(4) || requested != actual {
        return Err(ProbeError::InstallFailed);
    }
    Ok(())
}

#[cfg(target_arch = "riscv64")]
mod native;
#[cfg(target_arch = "riscv64")]
pub(crate) use native::installed;
#[cfg(target_arch = "riscv64")]
pub use native::{NativeFaultProbe, install_fault_probe, probe_return_code};

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> (ProbeOp, ExpectedFault) {
        let address = VirtAddr::from_raw(0xffffffc000001000);
        let site = ProbeSite {
            instruction: VirtAddr::from_raw(0x80200002),
            resume: VirtAddr::from_raw(0x80200006),
        };
        (
            ProbeOp::WriteByte {
                address,
                value: 0x76,
            },
            ExpectedFault {
                kind: FaultKind::StorePage,
                address,
                site,
            },
        )
    }
    #[test]
    fn exact_fault_consumes_record_and_returns_explicit_resume() {
        let (op, expected) = request();
        let mut record = Record::new();
        record.arm(op, expected, expected.site).unwrap();
        assert_eq!(
            record.observe(
                15,
                expected.site.instruction.as_usize(),
                expected.address.as_usize()
            ),
            Ok(expected.site.resume.as_usize())
        );
        assert!(
            record
                .observe(
                    15,
                    expected.site.instruction.as_usize(),
                    expected.address.as_usize()
                )
                .is_err()
        );
        assert_eq!(
            record.finish(),
            Ok(FaultInfo {
                kind: expected.kind,
                instruction: expected.site.instruction,
                address: expected.address
            })
        );
        assert_eq!(record.finish(), Err(ProbeError::NotObserved));
    }
    #[test]
    fn wrong_event_and_no_record_are_never_recoverable() {
        let (op, expected) = request();
        let pc = expected.site.instruction.as_usize();
        let address = expected.address.as_usize();
        for event in [
            (13, pc, address),
            (15, pc + 2, address),
            (15, pc, address + 1),
            (usize::MAX, pc, address),
        ] {
            let mut record = Record::new();
            record.arm(op, expected, expected.site).unwrap();
            assert!(record.observe(event.0, event.1, event.2).is_err());
            assert_eq!(record.finish(), Err(ProbeError::NotObserved));
        }
        assert!(Record::new().observe(15, pc, address).is_err());
    }
    #[test]
    fn probe_validation_does_not_overwrite_busy_record() {
        let (op, expected) = request();
        let mut record = Record::new();
        let mut wrong = expected;
        wrong.kind = FaultKind::LoadPage;
        assert_eq!(
            record.arm(op, wrong, expected.site),
            Err(ProbeError::InvalidProbe)
        );
        wrong = expected;
        wrong.address = VirtAddr::from_raw(7);
        assert_eq!(
            record.arm(op, wrong, expected.site),
            Err(ProbeError::InvalidProbe)
        );
        wrong = expected;
        wrong.site.resume = VirtAddr::from_raw(4);
        assert_eq!(
            record.arm(op, wrong, expected.site),
            Err(ProbeError::InvalidProbe)
        );
        record.arm(op, expected, expected.site).unwrap();
        assert_eq!(record.arm(op, wrong, expected.site), Err(ProbeError::Busy));
        assert_eq!(record.expected, Some(expected));
        assert_eq!(record.finish(), Err(ProbeError::NotObserved));
    }
    #[test]
    fn failed_or_duplicate_installation_is_rejected() {
        assert_eq!(validate_install(false, 0x80200000, 0x80200000), Ok(()));
        assert_eq!(
            validate_install(true, 0, 0),
            Err(ProbeError::AlreadyInstalled)
        );
        assert_eq!(
            validate_install(false, 0x80200000, 0),
            Err(ProbeError::InstallFailed)
        );
        assert_eq!(
            validate_install(false, 2, 2),
            Err(ProbeError::InstallFailed)
        );
    }
}
