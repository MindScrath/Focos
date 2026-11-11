/// 链接脚本变化时重新链接内核，避免 `__kernel_end` 沿用旧地址。
fn main() {
    println!("cargo::rerun-if-changed=src/arch/riscv64/linker.ld");
}
