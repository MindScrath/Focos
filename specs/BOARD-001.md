# BOARD-001：VisionFive2 的第一行 FocOS 输出

关联 [Issue #2](https://github.com/MindScrath/Focos/issues/2)。本次实现 U-Boot bootelf 的单 hart 启动；2026-11-05 已由用户在真板观察到成功标记，规格和实现待队友审阅，独立冷启动复现待执行。UART 驱动、输入和定时中断留给后续 PR，不能据此关闭整个 Issue。

## 已观察到什么

2026-11-05 的本地串口记录显示：8 GiB DRAM，范围为 `0x40000000..0x240000000`（右端不含）；OpenSBI `VF2_515_v3.6.1`；U-Boot `2021.10`（2023-08-23）；本次 Boot HART ID 为 1。U-Boot 能执行 `version`、`bdinfo`，并提供 `bootelf -p` 与 `fatload`。现有 Debian 能启动到登录提示。

随后在同一串口会话中，从 SD 的 FAT 分区 `1:3` 装入 22720 字节 ELF，执行 `bootelf -p 0x60000000`，观察到 `Starting application at 0x80200000`、版本行和唯一的 `FOCOS_VF2_BOOT_OK`。这是一次真板启动实测，尚无队友复现或稳定性结论。

可分享的 [实测节选](../docs/evidence/BOARD-001-20261105.txt) 已去除设备标识，包含 ELF 哈希、基线提交和未提交源码快照哈希。原始记录已冻结在被忽略的 `target/evidence/vf2-board-first-success-20261105.raw.log`，不随仓库分发。

## 入口和预期行为

- `make vf2` 生成独立的 `target/vf2/focos-vf2.elf`，不覆盖默认 QEMU 产物。
- 继续链接到 `0x80200000`；这在本次板子的 DRAM 内，避开已观察到的 OpenSBI 与 U-Boot 区域。ELF 暂存地址用 `0x60000000`，与装载后的内核、栈及 BSS 分开。换固件或板型先重新核对内存布局。
- 显式生成仅含 PT_LOAD 的程序头。v2021.10 的 `load_elf64_image_phdr` 不过滤程序头类型，不能把默认生成的 attributes 等非装载程序头交给它；构建检查类型、文件长度、段范围与重叠，限制内核全部段在 `0x80200000..0x80300000`。
- 只支持在当前 S-mode U-Boot 提示符执行 `bootelf -p`，由它在当前 hart 调用 ELF 入口。其他 hart 不进入该入口；本实现不启动或接管其他 hart。
- bootelf 的 `a0/a1` 是 `argc/argv`，不能当成 hart ID / DTB。入口忽略它们，不以 `a0 == 0` 为条件，也不固定 hart ID 为 1。
- 建栈前清除 S 模式中断使能及 `sie`，执行 `fence.i`；随后建立私有 64 KiB 栈、清零 BSS，再调用 Rust。本次不安装中断处理或启用分页；要求固件提供无分页的 S-mode 运行环境。
- 通过现有 HAL/SBI 控制台打印版本、启动方式与唯一标记 `FOCOS_VF2_BOOT_OK`，之后保持中断关闭并停机等待；不返回已被替换栈的 U-Boot 调用者。panic 打印 `FOCOS_PANIC` 并请求失败关机。
- 该输出借用固件的 UART 能力，不代表 FocOS 已实现 UART 驱动。SD 文件由 U-Boot 读取，不代表 FocOS 已实现 SD 驱动。

## 怎样验证

1. 先让模拟调用测试以现有内核运行，观察其因缺少板级标记或非零 `argc` 被停住而失败。
2. `make check` 回归 BOOT-001；`make vf2-check` 编译板级选项，并在 QEMU 用跳板分别模拟 `argc=0` 和 `argc=2` 的单 hart 调用。要求板级标记恰好一次、没有 panic，且保持运行至观察超时。这只检查入口和 SBI 输出，不执行真实 U-Boot，也不模拟 JH7110。
3. 真板加载 ELF，看到 U-Boot 的 `Starting application` 后出现版本行及板级标记；保存从固件到标记的完整串口记录。
4. 记录源提交完整 SHA、工作区是否有改动、ELF SHA256、固件版本和实际命令；未提交的实验必须同时保留改动，不能仅用基线 SHA 标识实验代码。
5. 冷启动后由另一名队员按指南复现，再标记本小步骤已验收。尚未实测的步骤必须保留待验状态。

## 依据

- [U-Boot v2021.10 bootelf 实现](https://github.com/u-boot/u-boot/blob/v2021.10/cmd/elf.c)：以 `entry(argc, argv)` 调用程序。
- [赛昉 U-Boot 分支](https://github.com/starfive-tech/u-boot/blob/JH7110_VisionFive2_devel/cmd/elf.c)：本次查阅的分支同样使用上述接口；实物加载路径已验证，尚未把厂商固件二进制对应到精确源码提交。
- [ELF program header 加载](https://github.com/u-boot/u-boot/blob/v2021.10/lib/elf.c)：按 `p_paddr` 装载，故文件暂存地址与运行地址必须分开。

操作步骤见 [上板指南](../docs/visionfive2.md)。
