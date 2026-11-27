//! 固定记录不借用测试对象；异常入口与探针按单核、关中断协议交接。
use super::*;
use crate::hal::{self, FaultProbe, UserAccess};
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
    fn focos_copy_from_user(user: usize, kernel: *mut u8, len: usize) -> usize;
    fn focos_copy_to_user(user: usize, kernel: *const u8, len: usize) -> usize;
    static focos_copy_from_user_load_pc: u8;
    static focos_copy_from_user_fault_resume: u8;
    static focos_copy_to_user_store_pc: u8;
    static focos_copy_to_user_fault_resume: u8;
}

fn supervisor_status() -> usize {
    let status: usize;
    // SAFETY: 只读本核状态；不得用 nomem 让用户访存越过观测点。
    unsafe {
        asm!("csrr {}, sstatus", out(reg) status, options(nostack));
    }
    status
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
    let requested = focos_vm_trap_entry as *const () as usize;
    install_entry(&INSTALLED, hal::local_irqs_enabled(), requested, |entry| {
        let actual: usize;
        // SAFETY: 调用者保证单核、关中断和有效栈；共用状态机已拒绝重复安装。
        unsafe {
            asm!(
                "csrw stvec, {entry}",
                "csrr {actual}, stvec",
                entry = in(reg) entry,
                actual = out(reg) actual,
                options(nostack),
            );
        }
        actual
    })?;
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
        // SAFETY: 继承 FaultProbe::probe 的单核、目标和返回路径前提。
        unsafe { self.run_probe(op, expected, Channel::Kernel) }
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

impl NativeFaultProbe {
    unsafe fn run_probe(
        &mut self,
        op: ProbeOp,
        expected: ExpectedFault,
        channel: Channel,
    ) -> Result<FaultInfo, ProbeError> {
        // SAFETY: 单核且关中断；复制固定存储，不跨操作持有记录引用。
        let mut record = unsafe { RECORD.0.get().read_volatile() };
        record.arm(op, expected, self.site(&op))?;
        record.channel = channel;
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
}

enum KernelBuffer<'a> {
    Destination(&'a mut [u8]),
    Source(&'a [u8]),
}

impl NativeFaultProbe {
    /// 调用者已验证完整用户区间及内核缓冲的别名；两者只在汇编的一次调用中交接。
    unsafe fn copy(
        &mut self,
        address: VirtAddr,
        mut buffer: KernelBuffer<'_>,
    ) -> Result<(), UserCopyError> {
        let (kind, pc, resume, len) = match &buffer {
            KernelBuffer::Destination(dst) => (
                FaultKind::LoadPage,
                &raw const focos_copy_from_user_load_pc,
                &raw const focos_copy_from_user_fault_resume,
                dst.len(),
            ),
            KernelBuffer::Source(src) => (
                FaultKind::StorePage,
                &raw const focos_copy_to_user_store_pc,
                &raw const focos_copy_to_user_fault_resume,
                src.len(),
            ),
        };
        let site = ProbeSite {
            instruction: VirtAddr::from_raw(pc.addr()),
            resume: VirtAddr::from_raw(resume.addr()),
        };
        // SAFETY: 单核运行。复制记录，不跨潜在异常保留原记录的引用。
        let mut record = unsafe { RECORD.0.get().read_volatile() };
        record.arm_copy(
            ExpectedFault {
                kind,
                address,
                site,
            },
            len,
            user_access_ready(supervisor_status(), installed()),
        )?;
        let start = address.as_usize();
        // SAFETY: 单核关中断，记录空闲且已完整设置；缓冲区按方向借用，无用户别名。
        // 汇编只在一条用户访存指令期间置 SUM，外部调用保留内存副作用。
        let copied = unsafe {
            RECORD.0.get().write_volatile(record);
            compiler_fence(Ordering::SeqCst);
            match &mut buffer {
                KernelBuffer::Destination(dst) => {
                    focos_copy_from_user(start, dst.as_mut_ptr(), len)
                }
                KernelBuffer::Source(src) => focos_copy_to_user(start, src.as_ptr(), len),
            }
        };
        compiler_fence(Ordering::SeqCst);
        assert_eq!(
            supervisor_status() & USER_ACCESS_BITS,
            0,
            "user permissions after copy"
        );
        // SAFETY: 汇编已返回，异常处理已完成；独占消费结果并使接下来的调用可登记。
        let observed = unsafe {
            let mut record = RECORD.0.get().read_volatile();
            let fault = record.finish().ok();
            RECORD.0.get().write_volatile(record);
            fault
        };
        let dst = match buffer {
            KernelBuffer::Destination(dst) => Some(dst),
            KernelBuffer::Source(_) => None,
        };
        complete_copy(start, len, copied, observed, dst)
    }
}

// SAFETY: 使用与内核探针相同的唯一同步入口，明确区分日志、故障指令和恢复标签。
unsafe impl UserAccess for NativeFaultProbe {
    unsafe fn probe_user_isolation(
        &mut self,
        op: ProbeOp,
        expected: ExpectedFault,
    ) -> Result<FaultInfo, ProbeError> {
        if !user_access_ready(supervisor_status(), installed()) {
            return Err(ProbeError::InvalidProbe);
        }
        // SAFETY: 目标由调用者授权，已检查 CPU 状态，使用独立的精确匹配通道。
        unsafe { self.run_probe(op, expected, Channel::Isolation) }
    }
    unsafe fn copy_from_user(
        &mut self,
        address: VirtAddr,
        dst: &mut [u8],
    ) -> Result<(), UserCopyError> {
        // SAFETY: 活动用户区和内核缓冲由调用方验证；失败后清零未读后缀。
        unsafe { self.copy(address, KernelBuffer::Destination(dst)) }
    }
    unsafe fn copy_to_user(&mut self, address: VirtAddr, src: &[u8]) -> Result<(), UserCopyError> {
        // SAFETY: 源只读、与用户目标不别名，硬件权限按字节检查。
        unsafe { self.copy(address, KernelBuffer::Source(src)) }
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
            assert_eq!(supervisor_status() & (1 << 18), 0, "SUM in trap handler");
            match record.channel {
                Channel::Kernel | Channel::Isolation => {
                    let tag = if matches!(record.channel, Channel::Kernel) {
                        "FOCOS_VM_FAULT"
                    } else {
                        "FOCOS_USER_ISOLATION_FAULT"
                    };
                    hal::console_write_fmt(format_args!(
                        "{tag} scause={cause:#x} sepc={pc:#x} stval={address:#x}\n"
                    ));
                }
                Channel::Copy { start, .. } => hal::console_write_fmt(format_args!(
                    "FOCOS_USER_COPY_FAULT scause={cause:#x} sepc={pc:#x} stval={address:#x} copied={}\n",
                    address - start
                )),
            }
            resume
        }
        Err(()) => {
            hal::console_write_fmt(format_args!(
                "FOCOS_VM_UNEXPECTED scause={cause:#x} sepc={pc:#x} stval={address:#x}\n"
            ));
            hal::console_write("FOCOS_PANIC\n");
            hal::shutdown(true)
        }
    }
}
