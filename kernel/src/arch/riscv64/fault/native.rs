//! 固定记录不借用测试对象；异常入口与探针按单核、关中断协议交接。
use super::*;
use crate::hal::{self, FaultProbe};
use core::{
    arch::{asm, global_asm},
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, Ordering, compiler_fence},
};

global_asm!(include_str!("../fault.S"));
struct Shared(UnsafeCell<Record>);
// SAFETY: 只有启动 hart，所有入口要求 SIE=0；探针和异常只通过 raw 复制记录，
// 不跨汇编操作持有引用。第二个异常在单次记录消费后只能失败关机。
unsafe impl Sync for Shared {}
static RECORD: Shared = Shared(UnsafeCell::new(Record::new()));
static INSTALLED: AtomicBool = AtomicBool::new(false);
/// 唯一同步异常入口的使用句柄，安装后持有至关机。
pub struct NativeFaultProbe {
    _private: (),
}

unsafe extern "C" {
    fn focos_vm_trap_entry();
    fn focos_probe_read(address: usize);
    fn focos_probe_write(address: usize, value: usize);
    fn focos_probe_execute(address: usize);
    static focos_probe_read_pc: u8;
    static focos_probe_read_resume: u8;
    static focos_probe_write_pc: u8;
    static focos_probe_write_resume: u8;
    static focos_probe_execute_resume: u8;
}

/// 可复制到专用测试页的返回代码；指令编码只由架构层解释。
pub fn probe_return_code() -> &'static [u8] {
    // jalr x0, 0(ra)：取指保护失效时仍能安全返回，由自检将 NotObserved 判为失败。
    &[0x67, 0x80, 0x00, 0x00]
}

/// 同时核对安装记录和实际 stvec，避免覆盖入口后继续启用分页。
pub(crate) fn installed() -> bool {
    let value: usize;
    // SAFETY: S 模式读取异常入口，未改变状态。
    unsafe {
        asm!("csrr {}, stvec", out(reg) value, options(nostack));
    }
    INSTALLED.load(Ordering::Relaxed) && value == focos_vm_trap_entry as *const () as usize
}

/// 安装并读回直接模式的同步异常入口。
///
/// # Safety
/// 唯一启动 hart、关中断且有有效内核栈；句柄持有至关机，不另行修改 stvec。
pub unsafe fn install_fault_probe() -> Result<NativeFaultProbe, ProbeError> {
    if INSTALLED.load(Ordering::Relaxed) {
        return Err(ProbeError::AlreadyInstalled);
    }
    if hal::local_irqs_enabled() {
        return Err(ProbeError::InstallFailed);
    }
    let requested = focos_vm_trap_entry as *const () as usize;
    let actual: usize;
    // SAFETY: 调用者保证合法启动栈和单核；先安装，再允许启用分页。
    unsafe {
        asm!(
            "csrw stvec, {entry}",
            "csrr {actual}, stvec",
            entry = in(reg) requested,
            actual = out(reg) actual,
            options(nostack),
        );
    }
    validate_install(false, requested, actual)?;
    INSTALLED.store(true, Ordering::Relaxed);
    Ok(NativeFaultProbe { _private: () })
}

// SAFETY: 标签来自固定汇编，操作恰好一次；匹配器三项核对后才恢复，其他情况关机。
unsafe impl FaultProbe for NativeFaultProbe {
    fn site(&self, op: &ProbeOp) -> ProbeSite {
        let (pc, resume) = match op {
            ProbeOp::ReadByte { .. } => (
                (&raw const focos_probe_read_pc).addr(),
                (&raw const focos_probe_read_resume).addr(),
            ),
            ProbeOp::WriteByte { .. } => (
                (&raw const focos_probe_write_pc).addr(),
                (&raw const focos_probe_write_resume).addr(),
            ),
            ProbeOp::Execute { address } => (
                address.as_usize(),
                (&raw const focos_probe_execute_resume).addr(),
            ),
        };
        ProbeSite {
            instruction: VirtAddr::from_raw(pc),
            resume: VirtAddr::from_raw(resume),
        }
    }
    unsafe fn probe(
        &mut self,
        op: ProbeOp,
        expected: ExpectedFault,
    ) -> Result<FaultInfo, ProbeError> {
        // SAFETY: 单核且关中断；复制固定存储，不跨操作持有记录引用。
        let mut record = unsafe { RECORD.0.get().read_volatile() };
        record.arm(op, expected, self.site(&op))?;
        // SAFETY: 已通过 Busy/参数检查；独占记录写回后，汇编才可能触发异常。
        unsafe {
            RECORD.0.get().write_volatile(record);
        }
        compiler_fence(Ordering::SeqCst);
        // SAFETY: 调用者保证目标和意外成功路径安全；extern 调用允许读写内存。
        unsafe {
            match op {
                ProbeOp::ReadByte { address } => focos_probe_read(address.as_usize()),
                ProbeOp::WriteByte { address, value } => {
                    focos_probe_write(address.as_usize(), usize::from(value))
                }
                ProbeOp::Execute { address } => focos_probe_execute(address.as_usize()),
            }
        }
        compiler_fence(Ordering::SeqCst);
        // SAFETY: 操作已返回，异常不再访问记录；读回并清除本次状态。
        let mut record = unsafe { RECORD.0.get().read_volatile() };
        let result = record.finish();
        // SAFETY: 同上；不保留探针的可变引用供异常使用。
        unsafe {
            RECORD.0.get().write_volatile(record);
        }
        result
    }
    unsafe fn sync_instructions(&mut self) {
        compiler_fence(Ordering::SeqCst);
        // SAFETY: 调用者已完成指令写入；不标 nomem，约束编译器与本核取指。
        unsafe {
            asm!("fence.i", options(nostack));
        }
        compiler_fence(Ordering::SeqCst);
    }
}

/// 汇编已保存寄存器和对齐栈，只返回经过匹配的恢复标签。
#[unsafe(no_mangle)]
extern "C" fn focos_vm_trap(cause: usize, pc: usize, address: usize) -> usize {
    // SAFETY: 同步异常打断探针，探针无活跃记录借用；单核关中断。
    let mut record = unsafe { RECORD.0.get().read_volatile() };
    match record.observe(cause, pc, address) {
        Ok(resume) => {
            // SAFETY: 匹配成功，先消费单次记录再恢复；重复异常不会再次匹配。
            unsafe {
                RECORD.0.get().write_volatile(record);
            }
            // 原始 CSR 三元组在匹配后记录，不用 panic 表示预期异常。
            hal::console_write_fmt(format_args!(
                "FOCOS_VM_FAULT scause={cause:#x} sepc={pc:#x} stval={address:#x}\n"
            ));
            resume
        }
        Err(()) => {
            hal::console_write_fmt(format_args!(
                "FOCOS_VM_UNEXPECTED scause={cause:#x} sepc={pc:#x} stval={address:#x}\n"
            ));
            hal::shutdown(true)
        }
    }
}
