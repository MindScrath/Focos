fn main() {
    // 链接脚本通过 rustflags 传入，需显式通知 Cargo 在其变化后重新链接。
    println!("cargo::rerun-if-changed=src/arch/riscv64/linker.ld");
}
