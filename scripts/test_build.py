"""检查 PLAT-002 的构建选择，直接运行构建脚本，不执行裸机代码。"""
import os
import pathlib
import subprocess
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]


class PlatformBuildTests(unittest.TestCase):
    """验证目标拒绝、板型导出和开发机测试之间的边界。"""

    @classmethod
    def setUpClass(cls):
        """用开发机工具链编译一次构建脚本，供各目标环境用例调用。"""
        directory = tempfile.TemporaryDirectory()
        cls.addClassCleanup(directory.cleanup)
        cls.build_script = pathlib.Path(directory.name) / "build-script"
        subprocess.run(
            ["rustc", "--edition=2024", str(ROOT / "kernel/build.rs"),
             "-o", str(cls.build_script)], cwd=ROOT, check=True,
        )

    def run_build_script(self, target, target_os):
        """提供 Cargo 的目标环境变量，返回构建脚本的退出状态及输出。"""
        environment = os.environ.copy()
        environment.update(TARGET=target, CARGO_CFG_TARGET_OS=target_os)
        return subprocess.run(
            [str(self.build_script)], cwd=ROOT / "kernel", env=environment,
            capture_output=True, text=True, check=False,
        )

    def test_supported_target_selects_qemu(self):
        """受支持目标必须声明并启用板型，同时显示构建选择。"""
        result = self.run_build_script("riscv64gc-unknown-none-elf", "none")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('cargo::rustc-cfg=focos_platform="qemu_riscv64"', result.stdout)
        self.assertIn(
            'cargo::rustc-check-cfg=cfg(focos_platform, values("qemu_riscv64"))',
            result.stdout,
        )
        self.assertIn("FocOS platform: qemu_riscv64", result.stdout)

    def test_other_bare_metal_targets_are_rejected(self):
        """同架构的其他目标及其他架构都不能被误当成当前 QEMU 配置。"""
        for target in ("riscv64imac-unknown-none-elf", "loongarch64-unknown-none"):
            with self.subTest(target=target):
                result = self.run_build_script(target, "none")
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("unsupported bare-metal target", result.stderr)
                self.assertIn(target, result.stderr)
                self.assertNotIn("cargo::rustc-cfg=focos_platform=", result.stdout)

    def test_host_build_does_not_select_a_board(self):
        """开发机库测试正常构建，不启用板级静态资源或输出 QEMU 选择。"""
        result = self.run_build_script("x86_64-unknown-linux-gnu", "linux")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("cargo::rustc-cfg=focos_platform=", result.stdout)
        self.assertNotIn("FocOS platform:", result.stdout)


if __name__ == "__main__":
    unittest.main()
