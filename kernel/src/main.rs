#![no_std]
#![no_main]

mod arch;
mod hal;

#[unsafe(no_mangle)]
extern "C" fn kernel_main() -> ! {
    #[cfg(feature = "vf2-bootelf")]
    {
        hal::console_write(concat!(
            "FocOS ",
            env!("CARGO_PKG_VERSION"),
            ": VisionFive2 bootelf / SBI console\r\nFOCOS_VF2_BOOT_OK\r\n",
        ));
        hal::halt()
    }
    #[cfg(not(feature = "vf2-bootelf"))]
    {
        hal::console_write("FocOS: early S-mode boot\nFOCOS_BOOT_OK\n");
        hal::shutdown(false)
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    hal::console_write("FOCOS_PANIC\n");
    hal::shutdown(true)
}
