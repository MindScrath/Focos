"""异常负例检查器不能把任意崩溃、超时或普通启动当成预期失败。"""
import unittest
import trap_negative


class NegativeTrapTests(unittest.TestCase):
    def test_expected_paths(self):
        trap_negative.validate_failure("FOCOS_TRAP_REENTRY\n", 0, "reentry")
        trap_negative.validate_failure("FOCOS_TRAP_NEGATIVE_UNKNOWN\nFOCOS_VM_UNEXPECTED scause=0x3 sepc=0x80200000 stval=0x0\nFOCOS_PANIC\n", 1, "unknown")

    def test_rejects_missing_duplicate_success_or_crash(self):
        for output, status in [("", 0), ("FOCOS_TRAP_REENTRY\n" * 2, 0),
                               ("FOCOS_TRAP_REENTRY\nFOCOS_BOOT_OK\n", 0),
                               ("FOCOS_TRAP_REENTRY\n", -9),
                               ("FOCOS_TRAP_REENTRY\nFOCOS_PANIC\n", 0)]:
            with self.subTest(output=output), self.assertRaises(ValueError):
                trap_negative.validate_failure(output, status, "reentry")

    def test_unknown_requires_breakpoint_and_failure_order(self):
        good = "FOCOS_TRAP_NEGATIVE_UNKNOWN\nFOCOS_VM_UNEXPECTED scause=0x3 sepc=0x80200000 stval=0x0\nFOCOS_PANIC\n"
        for output in [good.replace("0x3", "0xd"), good.replace("FOCOS_PANIC", ""),
                       "FOCOS_PANIC\n" + good.replace("FOCOS_PANIC\n", "")]:
            with self.assertRaises(ValueError):
                trap_negative.validate_failure(output, 0, "unknown")
