/// 为支持的裸机目标选择 QEMU 板型，并跟踪其链接布局；开发机库测试不选择板型。
fn main() {
    println!("cargo::rerun-if-changed=src/platform/qemu_riscv64/linker.ld");
    println!("cargo::rustc-check-cfg=cfg(focos_platform, values(\"qemu_riscv64\"))");

    let target = std::env::var("TARGET").expect("Cargo must provide TARGET");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").expect("Cargo must provide target OS");
    if target_os == "none" {
        assert_eq!(
            target, "riscv64gc-unknown-none-elf",
            "unsupported bare-metal target: {target}; expected riscv64gc-unknown-none-elf"
        );
        println!("cargo::rustc-cfg=focos_platform=\"qemu_riscv64\"");
        println!("cargo::warning=FocOS platform: qemu_riscv64");
    }
}
