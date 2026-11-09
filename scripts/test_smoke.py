"""检查器自身的负向测试，防止启动标记或失败输出被算成内存验收通过。"""
import pathlib
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import smoke


SUCCESS = "OpenSBI v1.3.1\nFOCOS_MM_OK\nFOCOS_BOOT_OK\n"


class ResultTests(unittest.TestCase):
    """验证成功标记、固件版本和退出状态必须共同满足约定。"""

    def test_accepts_complete_memory_result(self):
        """两个唯一标记按顺序输出且正常退出时通过。"""
        smoke.validate_result(SUCCESS, 0, memory=True)

    def test_boot_only_is_not_memory_success(self):
        """旧内核仍能通过启动检查，但不能通过新增内存检查。"""
        output = "OpenSBI v1.3.1\nFOCOS_BOOT_OK\n"
        smoke.validate_result(output, 0)
        with self.assertRaisesRegex(ValueError, "FOCOS_MM_OK"):
            smoke.validate_result(output, 0, memory=True)

    def test_rejects_missing_duplicate_or_embedded_markers(self):
        """缺失、重复或嵌在其他文字中的标记都不是有效成功记录。"""
        for marker in ("FOCOS_MM_OK", "FOCOS_BOOT_OK"):
            for replacement in ("", marker + "\n" + marker, "prefix " + marker):
                with self.subTest(marker=marker, replacement=replacement):
                    with self.assertRaises(ValueError):
                        smoke.validate_result(
                            SUCCESS.replace(marker, replacement), 0, memory=True,
                        )

    def test_rejects_reversed_success_order(self):
        """内存自检尚未完成时不能提前报告启动成功。"""
        with self.assertRaisesRegex(ValueError, "before boot success"):
            smoke.validate_result(
                "OpenSBI v1.3.1\nFOCOS_BOOT_OK\nFOCOS_MM_OK\n", 0, memory=True,
            )

    def test_rejects_panic_even_with_success_markers(self):
        """日志中出现 panic 时，即使已有成功标记也必须失败。"""
        with self.assertRaisesRegex(ValueError, "FOCOS_PANIC"):
            smoke.validate_result(SUCCESS + "FOCOS_PANIC\n", 0, memory=True)

    def test_rejects_abnormal_exit(self):
        """正常标记不能掩盖非零退出状态或信号终止。"""
        for status in (1, 2, -9):
            with self.subTest(status=status):
                with self.assertRaisesRegex(ValueError, "status"):
                    smoke.validate_result(SUCCESS, status, memory=True)

    def test_rejects_unreviewed_firmware(self):
        """固件缺失或版本变化时不能沿用当前保留区假设。"""
        for version in ("", "OpenSBI v1.3.10", "OpenSBI v1.4"):
            with self.subTest(version=version):
                with self.assertRaisesRegex(ValueError, "OpenSBI"):
                    smoke.validate_result(
                        SUCCESS.replace("OpenSBI v1.3.1", version), 0, memory=True,
                    )


class RunnerTests(unittest.TestCase):
    """验证检查入口对超时和错误 QEMU 版本的处理，不启动真实虚拟机。"""

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
                smoke.run_check(memory=True)
            self.assertEqual(write_log.call_args.args[-2:], (SUCCESS, "TIMEOUT"))

    def test_unreviewed_qemu_does_not_run(self):
        """机器实现版本变化时先报错，不能直接沿用 MM-002 的内存布局。"""
        with patch.object(smoke.subprocess, "check_output", return_value=
                          "QEMU emulator version 9.0.0\n"), \
                patch.object(smoke.subprocess, "run") as run:
            with self.assertRaisesRegex(SystemExit, "QEMU 8.2.2"):
                smoke.run_check(memory=True)
            run.assert_not_called()


if __name__ == "__main__":
    unittest.main()
