"""编译真实 RISC-V 库的调用方，验证启动自检与 DTB 读取的 unsafe 边界；不运行裸机代码。"""
import json
import pathlib
import subprocess
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
TARGET = "riscv64gc-unknown-none-elf"


class IrqApiTests(unittest.TestCase):
    """HAL 和架构层的重导出都必须保留启动自检的 unsafe 要求。"""

    @classmethod
    def setUpClass(cls):
        """从 Cargo 本次构建产物中取库路径，避免选中旧的 rlib。"""
        result = subprocess.run(
            ["cargo", "build", "--locked", "--release", "-p", "focos-kernel",
             "--lib", "--target", TARGET, "--message-format=json"],
            cwd=ROOT, capture_output=True, text=True, check=True,
        )
        artifacts = [json.loads(line) for line in result.stdout.splitlines()]
        cls.library = next(
            pathlib.Path(filename)
            for artifact in artifacts
            if artifact.get("reason") == "compiler-artifact"
            and artifact["target"]["name"] == "focos_kernel"
            for filename in artifact["filenames"]
            if filename.endswith(".rlib")
        )

    def compile_caller(self, source):
        """只生成元数据，无需裸机入口、链接脚本或 QEMU。"""
        with tempfile.TemporaryDirectory() as directory:
            caller = pathlib.Path(directory) / "caller.rs"
            caller.write_text("#![no_std]\n" + source, encoding="utf-8")
            return subprocess.run(
                ["rustc", "--edition=2024", "--crate-type=lib", "--emit=metadata",
                 "--target", TARGET, "--extern", f"focos_kernel={self.library}",
                 "-L", f"dependency={self.library.parent}",
                 "--out-dir", directory, str(caller)],
                cwd=ROOT, capture_output=True, text=True, check=False,
            )

    def test_startup_check_requires_unsafe(self):
        """安全调用方必须因 E0133 被拒绝，而不是因为链接或依赖缺失失败。"""
        for module in ("hal", "arch"):
            with self.subTest(module=module):
                result = self.compile_caller(
                    f"pub fn check() {{ let _ = focos_kernel::{module}::"
                    "check_local_irq_state(); }"
                )
                self.assertNotEqual(result.returncode, 0, result.stderr)
                self.assertIn("E0133", result.stderr)

    def test_explicit_unsafe_and_normal_critical_section_compile(self):
        """显式承担启动前提可调用自检；普通临界区仍是安全接口。"""
        result = self.compile_caller("""
            pub unsafe fn startup_check() {
                let _ = unsafe { focos_kernel::hal::check_local_irq_state() };
                let _ = unsafe { focos_kernel::arch::check_local_irq_state() };
            }
            pub fn critical_section() -> (bool, u32) {
                focos_kernel::hal::with_local_irqs_disabled(|| (
                    focos_kernel::hal::local_irqs_enabled(), 0x35,
                ))
            }
        """)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_timebase_reader_requires_unsafe(self):
        """板级入口需承担真实固件存储的前提，不能由安全代码任意解引用。"""
        result = self.compile_caller(
            "pub fn read() { let _ = focos_kernel::platform::boot_timebase_frequency(0); }"
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("E0133", result.stderr)
        result = self.compile_caller(
            "pub unsafe fn read() { let _ = unsafe { "
            "focos_kernel::platform::boot_timebase_frequency(0) }; }"
        )
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_timer_operations_and_self_check_require_unsafe(self):
        """初始化、预约、撤销、显式开关和启动自检均不能由安全调用方执行。"""
        for body in [
            "let _ = focos_kernel::hal::init_boot_timer(core::num::NonZeroU32::new(1).unwrap());",
            "let _ = timer.arm_at(focos_kernel::hal::MonotonicNs(1));",
            "let _ = timer.cancel();",
            "let _ = timer.set_boot_irqs_enabled(true);",
            "let _ = focos_kernel::hal::check_boot_timer(timer);",
        ]:
            result = self.compile_caller("pub fn call(timer: &mut focos_kernel::hal::NativeTimer) {" + body + "}")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("E0133", result.stderr)
            result = self.compile_caller("pub unsafe fn call(timer: &mut focos_kernel::hal::NativeTimer) { unsafe {" + body + "}}")
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_timer_handle_cannot_be_copied_sent_or_shared(self):
        for bound in ("Copy", "Clone", "Send", "Sync"):
            result = self.compile_caller("fn require<T: " + bound + ">() {} pub fn check() { require::<focos_kernel::hal::NativeTimer>(); }")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("E0277", result.stderr)


if __name__ == "__main__":
    unittest.main()
