"""检查器自身的负向测试，防止启动标记或失败输出被算成专项验收通过。"""
import pathlib
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import smoke


VM_SYMBOLS = {
    "__code_start": 0x80200000, "__code_end": 0x80208000,
    "__rodata_start": 0x80208000, "__rodata_end": 0x8020a000,
    "__writable_start": 0x8020a000, "__kernel_end": 0x80224000,
    "focos_probe_read_pc": 0x80202000, "focos_probe_write_pc": 0x80201000,
}

SUCCESS = """OpenSBI v1.3.1
FocOS memory: [0x80224000, 0x87e00000)
FOCOS_MM_OK
FOCOS_IRQ_OK
FocOS VM layout: KernelLayout { code: PhysAddr(2149580800)..PhysAddr(2149613568), rodata: PhysAddr(2149613568)..PhysAddr(2149621760), writable: PhysAddr(2149621760)..PhysAddr(2149728256), allocatable: PhysAddr(2149728256)..PhysAddr(2279604224), test_window: VirtAddr(18446743798831644672)..VirtAddr(18446743798831661056) }
FocOS VM CPU: satp=0x8000000000080226 mode=Sv39 asid=0 root=0x80226000
FOCOS_VM_ENABLED
FOCOS_VM_ALIAS_OK
FOCOS_VM_FAULT scause=0xf sepc=0x80201000 stval=0xffffffc000001000
FOCOS_VM_READONLY_OK
FOCOS_VM_FAULT scause=0xc sepc=0xffffffc000002000 stval=0xffffffc000002000
FOCOS_VM_NOEXEC_OK
FOCOS_VM_FAULT scause=0xd sepc=0x80202000 stval=0xffffffc000000000
FOCOS_VM_UNMAP_OK
FOCOS_VM_REMAP_OK
FOCOS_VM_CLEAN_OK
FocOS VM CPU: satp=0x8000000000080224 mode=Sv39 asid=0 root=0x80224000
FOCOS_VM_LIFECYCLE_OK
FocOS VM retained table pages: 64
FOCOS_VM_OK
FOCOS_BOOT_OK
"""

USER_SYMBOLS = dict(VM_SYMBOLS, focos_copy_from_user_load_pc=0x80203400,
                    focos_copy_to_user_store_pc=0x80203450,
                    focos_copy_from_user_fault_resume=0x80203420,
                    focos_copy_to_user_fault_resume=0x80203470)
USER_SUCCESS = SUCCESS.replace("FOCOS_BOOT_OK", """FocOS USER VM CPU: satp=0x8000000000080268 mode=Sv39 asid=0 root=0x80268000
FOCOS_USER_ISOLATION_FAULT scause=0xd sepc=0x80202000 stval=0x10000
FOCOS_USER_ISOLATION_FAULT scause=0xf sepc=0x80201000 stval=0x10000
FOCOS_USER_ISOLATION_FAULT scause=0xc sepc=0x12000 stval=0x12000
FOCOS_USER_VM_ISOLATION_OK
FOCOS_USER_COPY_FAULT scause=0xd sepc=0x80203400 stval=0x13000 copied=0
FOCOS_USER_COPY_FAULT scause=0xf sepc=0x80203450 stval=0x11000 copied=0
FOCOS_USER_COPY_FAULT scause=0xd sepc=0x80203400 stval=0x15000 copied=8
FOCOS_USER_COPY_FAULT scause=0xf sepc=0x80203450 stval=0x15000 copied=8
FOCOS_USER_VM_COPY_OK
FocOS USER VM CPU: satp=0x8000000000080269 mode=Sv39 asid=0 root=0x80269000
FocOS USER VM CPU: satp=0x8000000000080268 mode=Sv39 asid=0 root=0x80268000
FOCOS_USER_VM_SWITCH_OK
FocOS USER VM CPU: satp=0x8000000000080224 mode=Sv39 asid=0 root=0x80224000
FocOS USER VM retained table pages: 64
FOCOS_USER_VM_CLEAN_OK
FOCOS_USER_VM_OK
FOCOS_BOOT_OK""")


class ResultTests(unittest.TestCase):
    """验证各合同的成功标记、顺序和退出状态；固件版本不作为通过条件。"""

    def setUp(self):
        """结果判定使用独立 ELF 符号夹具；真实读取另由符号解析测试覆盖。"""
        patcher = patch.object(smoke, "load_vm_symbols", return_value=VM_SYMBOLS)
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_accepts_complete_results(self):
        """启动、内存、中断和分页合同都能从完整输出中取到自己的标记。"""
        for contract in smoke.CONTRACTS:
            with self.subTest(contract=contract):
                smoke.validate_result(USER_SUCCESS, 0, contract, USER_SYMBOLS)

    def test_boot_only_is_not_memory_or_irq_success(self):
        """旧内核仍能通过启动检查，但不能通过新增专项检查。"""
        output = "OpenSBI v1.3.1\nFOCOS_BOOT_OK\n"
        smoke.validate_result(output, 0, "boot")
        for contract in ("mm", "irq", "vm"):
            with self.subTest(contract=contract):
                with self.assertRaisesRegex(ValueError, "FOCOS_"):
                    smoke.validate_result(output, 0, contract)

    def test_rejects_missing_duplicate_or_embedded_markers(self):
        """缺失、重复或嵌在其他文字中的标记都不是有效成功记录。"""
        for contract, (_, markers) in smoke.CONTRACTS.items():
            for marker in markers:
                for replacement in ("", marker + "\n" + marker, "prefix " + marker):
                    with self.subTest(contract=contract, marker=marker,
                                      replacement=replacement):
                        with self.assertRaises(ValueError):
                            smoke.validate_result(
                                USER_SUCCESS.replace(marker, replacement), 0, contract, USER_SYMBOLS,
                            )

    def test_rejects_reversed_success_order(self):
        """专项自检尚未完成时不能提前报告启动成功。"""
        for contract in ("mm", "irq", "vm"):
            with self.subTest(contract=contract):
                _, markers = smoke.CONTRACTS[contract]
                reversed_output = (
                    "OpenSBI v1.3.1\n" + "\n".join(reversed(markers)) + "\n"
                )
                with self.assertRaisesRegex(ValueError, "out of order"):
                    smoke.validate_result(reversed_output, 0, contract)

    def test_rejects_panic_even_with_success_markers(self):
        """日志中出现 panic 时，即使已有成功标记也必须失败。"""
        with self.assertRaisesRegex(ValueError, "FOCOS_PANIC"):
            smoke.validate_result(SUCCESS + "FOCOS_PANIC\n", 0, "mm")

    def test_rejects_abnormal_exit(self):
        """正常标记不能掩盖非零退出状态或信号终止。"""
        for status in (1, 2, -9):
            with self.subTest(status=status):
                with self.assertRaisesRegex(ValueError, "status"):
                    smoke.validate_result(SUCCESS, status, "mm")

    def test_accepts_different_firmware_versions(self):
        """固件版本只写入日志，不代替内核布局、读写或中断状态验收。"""
        for banner in ("", "OpenSBI v1.3.1\n", "OpenSBI v1.6\n"):
            with self.subTest(banner=banner):
                smoke.validate_result(
                    banner + "FOCOS_MM_OK\nFOCOS_IRQ_OK\nFOCOS_BOOT_OK\n",
                    0,
                    "mm",
                )

    def test_vm_rejects_bad_fault_evidence(self):
        """标记不能代替真实异常记录，错误原因、地址及重复异常均失败。"""
        record = next(line for line in SUCCESS.splitlines()
                      if line.startswith("FOCOS_VM_FAULT"))
        for output in (
                SUCCESS.replace(record + "\n", ""),
                SUCCESS + record + "\n", SUCCESS.replace("scause=0xf", "scause=0x7"),
                SUCCESS.replace("stval=0xffffffc000001000", "stval=0x0"),
                SUCCESS.replace("sepc=0xffffffc000002000", "sepc=0x80201000"),
                SUCCESS.replace(record + "\n", "") + record + "\n",
                SUCCESS.replace("sepc=0x80201000", "sepc=0x0"),
                SUCCESS + "FOCOS_VM_UNEXPECTED scause=0x2\n"):
            with self.subTest(output=output), self.assertRaises(ValueError):
                smoke.validate_result(output, 0, "vm")

    def test_old_kernel_cannot_pass_vm(self):
        """MM-002 与 ARCH-001 已通过的旧内核仍缺少分页行为。"""
        with self.assertRaisesRegex(ValueError, "FOCOS_VM"):
            smoke.validate_result(
                "FOCOS_MM_OK\nFOCOS_IRQ_OK\nFOCOS_BOOT_OK\n", 0, "vm",
            )

    def test_vm_rejects_wrong_nonzero_instruction_addresses(self):
        """非零不等于实际探针位置，读写异常都须核对本次 ELF。"""
        for pc in ("0x80201000", "0x80202000"):
            with self.subTest(pc=pc), self.assertRaises(ValueError):
                smoke.validate_result(SUCCESS.replace(f"sepc={pc}", "sepc=0x1"), 0, "vm")

    def test_vm_requires_cpu_and_layout_records(self):
        """只有成功标记及异常事件，不构成完整分页证据。"""
        for prefix in ("FocOS VM CPU:", "FocOS VM layout:", "FocOS memory:",
                       "FocOS VM retained table pages:"):
            output = "\n".join(line for line in SUCCESS.splitlines() if not line.startswith(prefix))
            with self.subTest(prefix=prefix), self.assertRaises(ValueError):
                smoke.validate_result(output, 0, "vm")

    def test_vm_rejects_invalid_or_reordered_state_evidence(self):
        """状态记录须来自同一布局，且分别位于首次启用和最终重建阶段。"""
        cpu = [line for line in SUCCESS.splitlines() if line.startswith("FocOS VM CPU:")]
        layout = next(line for line in SUCCESS.splitlines() if line.startswith("FocOS VM layout:"))
        outputs = [
            SUCCESS.replace("satp=0x8000000000080226", "satp=0x9000000000080226"),
            SUCCESS.replace("satp=0x8000000000080226", "satp=0x8000100000080226"),
            SUCCESS.replace("root=0x80226000", "root=0x80227000"),
            SUCCESS.replace("root=0x80226000", "root=0x80226001"),
            SUCCESS.replace("mode=Sv39", "mode=Bare", 1),
            SUCCESS.replace("asid=0", "asid=1", 1),
            SUCCESS.replace("2149613568", "2149609472"),
            SUCCESS.replace("2149728256", "2149732352"),
            SUCCESS.replace("18446743798831661056", "18446743798831665152"),
            SUCCESS.replace("[0x80224000,", "[0x80225000,"),
            SUCCESS.replace("retained table pages: 64", "retained table pages: 63"),
            SUCCESS + cpu[0] + "\n", SUCCESS + layout + "\n",
            SUCCESS.replace(cpu[0] + "\n", ""), SUCCESS.replace(cpu[1] + "\n", ""),
            SUCCESS.replace(cpu[0] + "\n", "") + cpu[0] + "\n",
            cpu[1] + "\n" + SUCCESS.replace(cpu[1] + "\n", ""),
        ]
        for output in outputs:
            with self.subTest(output=output), self.assertRaises(ValueError):
                smoke.validate_result(output, 0, "vm")

    def test_vm_uses_current_elf_probe_locations(self):
        """另一份 ELF 的合法探针地址不能复用旧日志中的指令地址。"""
        symbols = dict(VM_SYMBOLS, focos_probe_write_pc=0x80203000)
        with self.assertRaises(ValueError):
            smoke.validate_result(SUCCESS, 0, "vm", symbols)
        smoke.validate_result(SUCCESS.replace("sepc=0x80201000", "sepc=0x80203000"),
                              0, "vm", symbols)

    def test_vm_exit_status_is_required(self):
        """全部事件已输出也不能掩盖退出失败。"""
        with self.assertRaisesRegex(ValueError, "status"):
            smoke.validate_result(SUCCESS, 1, "vm")

    def test_user_vm_rejects_old_elf_output_even_after_vm_ok(self):
        smoke.validate_result(SUCCESS, 0, "vm", VM_SYMBOLS)
        with self.assertRaisesRegex(ValueError, "FOCOS_USER_VM"):
            smoke.validate_result(SUCCESS, 0, "user-vm", USER_SYMBOLS)

    def test_user_vm_checks_each_fault_and_its_stage(self):
        lines = USER_SUCCESS.splitlines()
        for i, line in enumerate(lines):
            if not line.startswith(("FOCOS_USER_COPY_FAULT", "FOCOS_USER_ISOLATION_FAULT")):
                continue
            bad = [line.replace("scause=0x", "scause=0x1"), line.replace("sepc=0x", "sepc=0x1"),
                   line.replace("stval=0x", "stval=0x1"), "", line + "\n" + line]
            if "copied=" in line:
                bad.append(line.replace("copied=", "copied=1"))
            for replacement in bad:
                with self.subTest(line=line, replacement=replacement), self.assertRaises(ValueError):
                    smoke.validate_result("\n".join([*lines[:i], replacement, *lines[i+1:]]),
                                          0, "user-vm", USER_SYMBOLS)
            with self.assertRaises(ValueError):
                smoke.validate_result(USER_SUCCESS.replace(line + "\n", "") + line + "\n",
                                      0, "user-vm", USER_SYMBOLS)

    def test_user_vm_checks_all_transitions_and_reclamation(self):
        cpus = [line for line in USER_SUCCESS.splitlines() if line.startswith("FocOS USER VM CPU")]
        for changed in [USER_SUCCESS.replace(cpus[0], cpus[1]),
                        USER_SUCCESS.replace(cpus[3], cpus[1]),
                        USER_SUCCESS.replace(cpus[1], cpus[0]), USER_SUCCESS.replace(cpus[1], ""),
                        USER_SUCCESS.replace(cpus[1], cpus[1].replace("asid=0", "asid=1")),
                        USER_SUCCESS.replace(cpus[1], cpus[1].replace("80269000", "8026a000")),
                        USER_SUCCESS.replace(cpus[1]+"\n", "") + cpus[1] + "\n",
                        USER_SUCCESS.replace("USER VM retained table pages: 64", "USER VM retained table pages: 63"),
                        USER_SUCCESS + "FOCOS_VM_UNEXPECTED", USER_SUCCESS + "FOCOS_PANIC"]:
            with self.subTest(changed=changed), self.assertRaises(ValueError):
                smoke.validate_result(changed, 0, "user-vm", USER_SYMBOLS)
        with self.assertRaises(ValueError):
            smoke.validate_result(USER_SUCCESS, 1, "user-vm", USER_SYMBOLS)


class SymbolTests(unittest.TestCase):
    """核对 readelf 输出解析和错误传播，不把缺少符号当成跳过验收。"""

    def setUp(self):
        self.table = "\n".join(
            f"  {index}: {value:016x} 0 NOTYPE GLOBAL DEFAULT 1 {name}"
            for index, (name, value) in enumerate(VM_SYMBOLS.items(), 1))

    def test_symbols_are_loaded_from_the_selected_elf(self):
        with patch.object(smoke.subprocess, "check_output", return_value=self.table) as read:
            self.assertEqual(smoke.load_vm_symbols(pathlib.Path("selected.elf")), VM_SYMBOLS)
            self.assertEqual(read.call_args.args[0],
                             ["readelf", "--wide", "--symbols", "selected.elf"])

    def test_missing_duplicate_undefined_and_unreadable_symbols_fail(self):
        lines = self.table.splitlines()
        for table in ("\n".join(lines[1:]), self.table + "\n" + lines[0],
                      self.table.replace("DEFAULT 1", "DEFAULT UND", 1)):
            with patch.object(smoke.subprocess, "check_output", return_value=table), \
                    self.assertRaises(ValueError):
                smoke.load_vm_symbols(pathlib.Path("kernel"))
        for error in (FileNotFoundError("readelf"), subprocess.CalledProcessError(1, "readelf")):
            with patch.object(smoke.subprocess, "check_output", side_effect=error), \
                    self.assertRaises(ValueError):
                smoke.load_vm_symbols(pathlib.Path("kernel"))


class RunnerTests(unittest.TestCase):
    """验证检查入口对超时和不同 QEMU 版本的处理，不启动真实虚拟机。"""

    def setUp(self):
        """提供临时 ELF 占位文件，使各用例进入运行与判定路径。"""
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        elf = pathlib.Path(directory.name) / "kernel"
        elf.write_bytes(b"test placeholder")
        self.elf = patch.object(smoke, "ELF", elf)
        self.elf.start()
        self.addCleanup(self.elf.stop)

    def test_timeout_fails_even_after_both_markers(self):
        """输出成功标记后仍不退出，也必须记录为超时失败。"""
        error = subprocess.TimeoutExpired("qemu", 20, output=SUCCESS.encode())
        with patch.object(smoke.subprocess, "check_output", return_value=
                          "QEMU emulator version 8.2.2\n"), \
                patch.object(smoke.subprocess, "run", side_effect=error), \
                patch.object(smoke, "write_log") as write_log:
            with self.assertRaisesRegex(SystemExit, "timed out"):
                smoke.run_check("mm")
            self.assertEqual(write_log.call_args.args[-2:], (SUCCESS, "TIMEOUT"))

    def test_newer_qemu_runs_and_is_logged(self):
        """新版 QEMU 仍执行行为验收，并把实际版本交给日志记录。"""
        output = "OpenSBI v1.6\nFOCOS_MM_OK\nFOCOS_BOOT_OK\n"
        completed = subprocess.CompletedProcess("qemu", 0, stdout=output)
        with patch.object(smoke.subprocess, "check_output", return_value=
                          "QEMU emulator version 9.0.0\n"), \
                patch.object(smoke.subprocess, "run", return_value=completed) as run, \
                patch.object(smoke, "write_log") as write_log, \
                patch("builtins.print") as report:
            smoke.run_check("mm")
            run.assert_called_once()
            self.assertEqual(write_log.call_args.args[1], "QEMU emulator version 9.0.0")
            self.assertEqual(write_log.call_args.args[-2:], (output, "PASS"))
            report.assert_called_once_with(
                "PASS: MM-002 (QEMU emulator version 9.0.0); "
                f"log: {smoke.MM_LOG}"
            )

    def test_vm_timeout_after_success_fails(self):
        """两个分页专项收到完整标记后若超时，仍各自保存失败日志。"""
        for contract, log, output in [("vm", smoke.VM_LOG, SUCCESS),
                                      ("user-vm", smoke.USER_VM_LOG, USER_SUCCESS)]:
            error = subprocess.TimeoutExpired("qemu", 20, output=output.encode())
            with self.subTest(contract=contract), \
                    patch.object(smoke.subprocess, "check_output", return_value="QEMU 8.2.2\n"), \
                    patch.object(smoke.subprocess, "run", side_effect=error), \
                    patch.object(smoke, "write_log") as write_log:
                with self.assertRaisesRegex(SystemExit, "timed out"):
                    smoke.run_check(contract)
                self.assertEqual(write_log.call_args.args[0], log)
                self.assertEqual(write_log.call_args.args[-1], "TIMEOUT")

    def test_irq_contract_uses_its_own_log(self):
        """中断专项通过时报告 ARCH-001，并写入独立日志路径。"""
        output = "OpenSBI v1.6\nFOCOS_IRQ_OK\nFOCOS_BOOT_OK\n"
        completed = subprocess.CompletedProcess("qemu", 0, stdout=output)
        with patch.object(smoke.subprocess, "check_output", return_value=
                          "QEMU emulator version 9.0.0\n"), \
                patch.object(smoke.subprocess, "run", return_value=completed), \
                patch.object(smoke, "write_log") as write_log, \
                patch("builtins.print") as report:
            smoke.run_check("irq")
            self.assertEqual(write_log.call_args.args[0], smoke.IRQ_LOG)
            self.assertEqual(write_log.call_args.args[-2:], (output, "PASS"))
            report.assert_called_once_with(
                "PASS: ARCH-001 (QEMU emulator version 9.0.0); "
                f"log: {smoke.IRQ_LOG}"
            )


if __name__ == "__main__":
    unittest.main()
