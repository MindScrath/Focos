# 第一次让 VisionFive2 打印 FocOS

目标：从 U-Boot 手动装入一个 ELF 文件，看到 `FOCOS_VF2_BOOT_OK`。2026-11-05 已在用户的 8 GiB VisionFive2 上完成首次实测，见 [日志节选](evidence/BOARD-001-20261105.txt)；独立队友复现和代码审阅待完成。第一次只验证启动和固件提供的 SBI 控制台，入口约定见 [BOARD-001](../specs/BOARD-001.md)。

## 1. 先确认串口与固件

使用 3.3V 信号电平的 USB 转 TTL 模块，板子断电时接线：模块 GND → 物理脚 6；模块 RXD → 物理脚 8（板 TX）；模块 TXD → 物理脚 10（板 RX）。模块 VCC 不接，板子单独用 Type-C 供电；按板上第 1 脚标记确认物理脚编号。

在开发机运行，设备名以实际枚举为准：

```sh
ls -l /dev/serial/by-id/
mkdir -p target/evidence
picocom --baud 115200 --databits 8 --parity n --stopbits 1 --flow n \
  --logfile target/evidence/vf2-board-session.log /dev/ttyACM0
```

先打开串口，再上电。看到 SPL 时就反复按空格，停在 `StarFive #`；本次固件只等 2 秒。Debian 已启动时，登录后执行 `sudo reboot`，保持串口连接，再中断启动。退出 picocom 用 `Ctrl+A`、`Ctrl+X`。

在 U-Boot 逐条执行并记录：

```text
version
bdinfo
printenv bootdelay bootcmd autostart
help bootelf
help fatload
mmc dev 1
mmc part
```

2026-11-05 已确认：8 GiB 板的 DRAM 从 `0x40000000` 开始，OpenSBI 为 `VF2_515_v3.6.1`，U-Boot 为 `2021.10`，支持上述加载命令。`autostart` 若为 `no`，bootelf 只加载不执行；下面会临时设为 `yes`，不执行 `saveenv`。MMC1 是本次日志中的 SD；换固件以现场枚举为准。

## 2. 在开发机生成 ELF

队友首次复现可直接取已记录的源版本，无须等待合入 main：

```sh
git clone https://github.com/ApolloMonasa/Focos-contrib.git Focos-vf2
cd Focos-vf2
git switch --detach c363b1cb3ba121c2c07f0266f52a12f0a4e29227
```

这是基于已合入 main 整理后的首次上板实现提交，代码与首次实测版本一致；本指南后续的文档提交不改变该次实现。该版本尚待人工审阅。工具准备见 [上手指南](getting-started.md)。

在 FocOS 仓库根目录执行：

```sh
make check
make vf2-check
```

`make vf2-check` 除现有工具外需要 GNU RISC-V binutils 的 `riscv64-linux-gnu-as`、`riscv64-linux-gnu-ld`。Ubuntu/Debian 安装 `binutils-riscv64-linux-gnu`，本次 Fedora 使用同名软件包；只想构建时执行 `make vf2`。CI 同时执行默认与板级检查，并保存 QEMU 日志。

应看到 BOOT-001 通过、ELF 兼容检查通过、两个 `simulated bootelf argc=...` 通过。ELF 检查防止旧版加载器把非装载程序头当作内存段；后两项在 QEMU 中模拟调用参数，不能替代 U-Boot 加载或真板测试。

待复制文件为 `target/vf2/focos-vf2.elf`。构建会打印它的 SHA256；复核可执行：

```sh
sha256sum target/vf2/focos-vf2.elf
git rev-parse HEAD
git status --short
```

记录完整源 SHA。工作区非干净时同时保留 diff 和新增源码，称为未提交实验；正式复现用已提交版本。ELF 和日志位于被 Git 忽略的 `target/`。

## 3. 将一个文件放到现有 SD 卡

本次读卡器识别 SD 为 `/dev/mmcblk0`，第三分区为 100 MiB FAT，第四分区为 Debian ext4；真板已用 `fatls mmc 1:3` 和 `fatload` 验证第三分区。其他镜像先用 `mmc part` 及 `fatls mmc 1:分区号` 确认可读的 FAT 分区。`分区号` 是说明占位，执行前替换成实际数字。

如果走读卡器：停在 U-Boot 提示符后断电，再拔卡；如果已进 Debian，先正常关机。插到开发机后运行：

```sh
lsblk -o NAME,SIZE,FSTYPE,LABEL,MOUNTPOINTS,MODEL
```

根据容量和插卡前后差异确认 SD 卡，找到对应 FAT 分区及其挂载点。**开发机自己的 `/boot/efi` 也可能是 FAT，不能当成 SD 分区。** 不格式化，不覆盖固件、Debian 内核或启动配置。

把下面变量换成已核实的 SD FAT 分区挂载点，再复制到分区根目录：

```sh
VF2_SD_MOUNT='/替换为实际SD卡FAT分区挂载点'
cp -i target/vf2/focos-vf2.elf "$VF2_SD_MOUNT/focos-vf2.elf"
sha256sum target/vf2/focos-vf2.elf "$VF2_SD_MOUNT/focos-vf2.elf"
sync
```

两个哈希应一致。通过桌面安全卸载该卡的全部挂载分区后拔卡，装回断电的板子。也可通过网络将同一 ELF 传到 Debian，再复制到已确认的 FAT 挂载点；镜像布局和登录方式按实际情况确认。

## 4. 从 U-Boot 手动执行

保持串口记录，启动并按空格进入 U-Boot。以下**仅在已确认 SD 为 MMC1、FAT 为分区 3 时**可直接运行；其他布局将 `1:3` 换成实际值。

```text
fatls mmc 1:3
fatload mmc 1:3 0x60000000 focos-vf2.elf
printenv filesize
setenv autostart yes
bootelf -p 0x60000000
```

每条单独执行：`fatls` 必须能看到该文件；`fatload` 成功读取的字节数应等于开发机 `wc -c < target/vf2/focos-vf2.elf` 的结果。失败就停下，不运行 bootelf。文件暂存在 `0x60000000`；bootelf 根据 ELF program headers 将代码装入 `0x80200000`，因此这两个地址不同。

本次已观察到的输出（版本行随项目版本变化）：

```text
## Starting application at 0x80200000 ...
FocOS 0.1.0: VisionFive2 bootelf / SBI console
FOCOS_VF2_BOOT_OK
```

之后保持等待，不自动返回 U-Boot。没有标记、出现 `FOCOS_PANIC` 或异常都算失败，保留完整日志定位；不通过打印额外成功标记绕过失败。恢复 Debian 可重新上电并允许默认启动继续。

## 5. 怎样向队友演示

讲清三个问题：原来 QEMU 的入口假设是什么；为什么 bootelf 的 argc 不能当成 hart ID；日志怎样证明执行了这份 FocOS。提供 spec、改动、实际命令、源 SHA、ELF 哈希和日志，邀请队友冷启动复现。

第一次示范可控制在 10–15 分钟：

1. 打开 #2 和 BOARD-001，说明这次只交付“手动加载、打印一行”，指给队友看成功判据。
2. 展示 `make check` 与 `make vf2-check` 的结果，解释 QEMU 只能覆盖部分入口行为。
3. 打开 `entry.S`，比较默认入口与 bootelf 入口；再看 `main.rs` 的版本和标记输出。不要求队友先读懂全部汇编。
4. 真板从断电开始，串口记录固件、进入 U-Boot、执行第 4 节的命令。让队友指出哪一行开始来自 FocOS。
5. 展示 ELF 哈希和实测节选，说明这份证据对应哪次构建、还没证明哪些功能。请队友复述操作，并在下一次独立执行；卡住的步骤就是指南需要补充的地方。

本次仅是 #2 的第一小步。后续独立实现 UART 收发、定时中断，再按 Issue 的完整条件验收。普通启动 PR 按协作规则审阅，涉及 HAL、ABI 或内存安全的改动请另外两名成员审阅。

队友复现时在 Issue 留下构建提交、ELF 哈希、固件版本和新日志，记录成功或卡住的具体步骤。首次实测时源码尚未提交，冻结快照及哈希在证据中；复现时使用 Git 中记录的准确提交。一次成功不等于稳定性通过：原始会话更早出现过 SD 响应超时，本次成功前已重新启动，原因尚未确认。

参考：[官方串口与供电资料](https://doc-en.rvspace.org/VisionFive2/PDF/VisionFive2_SDK_QSG.pdf)、[bootelf v2021.10 源码](https://github.com/u-boot/u-boot/blob/v2021.10/cmd/elf.c)、[fatload 文档](https://docs.u-boot.org/en/latest/usage/cmd/fatload.html)。本次依据调用规范编写入口，没有复制第三方内核实现。
