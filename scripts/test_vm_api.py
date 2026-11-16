"""编译真实库调用方，检查 MM-003 的 unsafe、独占借用和页句柄边界。"""
import json
import pathlib
import subprocess
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
TARGET = "riscv64gc-unknown-none-elf"
IMPORTS = """
use focos_kernel::mm::{KernelAddressSpace, KernelLayout, VmMemory, VmPage,
    PhysAddr, VirtAddr, MapPerm, IdentityMemory, IdentityPage, PageAllocator};
use focos_kernel::hal::{self, VmArch};
"""


class VmApiTests(unittest.TestCase):
    """只产生调用方元数据，不在开发机执行裸机指令。"""

    @classmethod
    def setUpClass(cls):
        result = subprocess.run(
            ["cargo", "build", "--locked", "--release", "-p", "focos-kernel",
             "--lib", "--target", TARGET, "--message-format=json"],
            cwd=ROOT, capture_output=True, text=True, check=True,
        )
        artifacts = [json.loads(line) for line in result.stdout.splitlines()]
        cls.library = next(pathlib.Path(filename) for artifact in artifacts
                           if artifact.get("reason") == "compiler-artifact"
                           and artifact["target"]["name"] == "focos_kernel"
                           for filename in artifact["filenames"]
                           if filename.endswith(".rlib"))

    def compile(self, source):
        with tempfile.TemporaryDirectory() as directory:
            caller = pathlib.Path(directory) / "caller.rs"
            caller.write_text("#![no_std]\n" + IMPORTS + source, encoding="utf-8")
            return subprocess.run(
                ["rustc", "--edition=2024", "--crate-type=lib", "--emit=metadata",
                 "--target", TARGET, "--extern", f"focos_kernel={self.library}",
                 "-L", f"dependency={self.library.parent}",
                 "--out-dir", directory, str(caller)],
                cwd=ROOT, capture_output=True, text=True, check=False,
            )

    def test_vm_entries_require_explicit_unsafe(self):
        for call in ("hal::init_vm_arch()", "hal::install_fault_probe()",
                     "space.activate()", "space.deactivate()",
                     "space.map_page(VirtAddr::from_raw(0), PhysAddr::from_raw(0), MapPerm::RW)",
                     "space.unmap_page(VirtAddr::from_raw(0))"):
            with self.subTest(call=call):
                result = self.compile("""
                    pub fn call<M: VmMemory, A: VmArch>(
                        space: &mut KernelAddressSpace<'_, M, A>
                    ) { let _ = CALL; }
                """.replace("CALL", call))
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("E0133", result.stderr)

    def test_destroy_keeps_both_exclusive_borrows_until_object_ends(self):
        for access, error in (("memory.alloc_zeroed_page()", "E0499"),
                              ("arch.current_root()", "E0502")):
            with self.subTest(access=access):
                result = self.compile("""
                    pub fn call<M: VmMemory, A: VmArch>(
                        memory: &mut M, arch: &mut A, layout: KernelLayout
                    ) {
                        let mut space = KernelAddressSpace::new(memory, arch, layout).unwrap();
                        space.destroy().unwrap();
                        let _ = ACCESS;
                    }
                """.replace("ACCESS", access))
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(error, result.stderr)

    def test_independent_page_access_and_explicit_drop_compile(self):
        result = self.compile("""
            pub unsafe fn call<M: VmMemory, A: VmArch>(
                memory: &mut M, arch: &mut A, layout: KernelLayout
            ) {
                let mut page = memory.alloc_zeroed_page().unwrap();
                let mut space = KernelAddressSpace::new(memory, arch, layout).unwrap();
                page.with_bytes_mut(|bytes| bytes[0] = 7);
                let _ = space.query(VirtAddr::from_raw(page.phys_addr().as_usize()));
                space.destroy().unwrap();
                drop(space);
                let _ = arch.current_root();
                let _ = unsafe { memory.release_page(page) };
            }
        """)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_page_reference_cannot_escape_access_closure(self):
        result = self.compile("""
            pub fn escape<P: VmPage>(page: &P) -> &[u8; 4096] {
                page.with_bytes(|bytes| bytes)
            }
        """)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("lifetime may not live long enough", result.stderr)

    def test_uncounted_page_cannot_be_released_through_observer(self):
        result = self.compile("""
            use focos_kernel::mm::vm_check::{ObservedMemory, PageCount};
            pub unsafe fn call<M: VmMemory>(mut memory: M) {
                let page = memory.alloc_zeroed_page().unwrap();
                let live = PageCount::default();
                let mut observed = ObservedMemory::new(memory, &live);
                let _ = unsafe { observed.release_page(page) };
            }
        """)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("E0308", result.stderr)

    def test_native_page_is_neither_copy_nor_clone(self):
        for trait in ("Copy", "Clone"):
            with self.subTest(trait=trait):
                result = self.compile(
                    f"fn bound<P: {trait}>() {{}} pub fn call() {{ bound::<IdentityPage>(); }}"
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("E0277", result.stderr)


if __name__ == "__main__":
    unittest.main()
