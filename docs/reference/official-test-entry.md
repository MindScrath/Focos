# 官方测试怎样接入

本页记录 2026-11-23 的核查。宿主入口提供构建、启动、原始日志和进程清理；官方判分的版本问题单独处理。

## 采用哪些来源

- [2025 测试源码及运行说明](https://github.com/oscomp/testsuits-for-oskernel/tree/8b58dd16d26d30f7c74d48d5832d870d3051b703)：`pre-2025` 固定提交。README 定义 12 组测试、Ext4 无分区表镜像、内核主动扫描并串行执行脚本、完成后主动关机。
- [官方本地评测框架](https://github.com/oscomp/autotest-for-oskernel/tree/d1bb3a3c4b27274e196a2648518525c1a304e339)：README 指定 `pre-20250615` 镜像，但容器标签是 `20260510`。这是可核查的公开搭配，不等于已证明与 2025 赛场环境完全相同。
- [官方判分接口说明](https://github.com/oscomp/oskernel-testsuits-cooperation/tree/44fa7ac65f292580c3974a8252f533c4484ebe6b/judge)：按组开始/结束标记，将组内输出交给对应 judge 的标准输入。保留官方工具的输出，不自行统一成通过率。
- [RISC-V 镜像](https://github.com/oscomp/testsuits-for-oskernel/releases/download/pre-20250615/sdcard-rv.img.xz)：解压后 SHA-256 为 `95973543db6b84a9a5e70f30da466ce292867aff5b689fb14c88dc9406e378b8`，本次重新计算确认。不要用当前源码重新构建的镜像冒充该发布物。

官方 RISC-V 命令采用 `virt`、SBI、VirtIO 块设备和用户态网络、UTC RTC、`-no-reboot`；公开框架配置为 1 CPU、1G 内存、3600 秒。拟定入口沿用这些参数，用临时写入层保护基础镜像。实际 QEMU/固件版本和命令必须记录；本地不同版本的结果不冒充赛场结果。官方提交还要求 `make all` 产生 `kernel-rv/kernel-la`，本轮本地 RISC-V 入口不意味着双架构提交规范已经完成。

## 目前不能跳过的差异

1. 源码 `scripts/test_all.sh` 注释掉了 LTP，并有与现行组脚本不一致的名称。完整执行必须以固定镜像中的两套测试脚本为准，不能直接运行这个总脚本就称为全量。
2. 镜像 BusyBox 命令表有 55 行、54 个不同命令；`echo "bbbbbbb" >> test.txt` 重复。官方 autotest 的 musl judge 仍要求 `kill 10`，镜像却使用 `sh -c 'sleep 5' & ./busybox kill $!`，另有 `printf "abc\n"`。这不是 FocOS 的失败，不能靠修改官方 judge 掩盖；#48 继续保留，尚不能宣布判分版本对应完成。
3. 镜像 LTP 脚本对每个程序无条件打印 `FAIL LTP CASE 名称 : 返回值`，包括返回值为 0 的情况。官方 LTP judge 正是读取该行收尾，不能仅凭单词 FAIL 判定失败。本轮只核查脚本和 judge，未实际运行全量 LTP。
4. 组结束标记、脚本退出 0 和 QEMU 退出 0 都不能证明组内测试已执行成功，下面的实测直接重现了这一点。

## 全部组入口复核（静态检查）

本次从固定镜像重新提取两套 libc 的全部 24 个 `*_testcode.sh`，逐个查看；对应脚本除组名的 libc 后缀外内容相同。下表只说明脚本实际调用和环境依赖，不代表这些组已实测通过，也未完成所有子程序的运行审计。

| 组 | 脚本调用和必须保留的行为 |
| --- | --- |
| basic | 进入 `basic` 执行 `run-all.sh`，然后回到原目录 |
| busybox | 按命令表逐行执行，保留重复命令、转义处理及原始 success/fail 文本 |
| lua | 通过 `test.sh` 调用 9 个 Lua 文件，子脚本需要 `/bin/busybox` |
| libctest | 依次执行 `run-static.sh`、`run-dynamic.sh`；动态加载环境必须另行验证 |
| libcbench | 执行 `libc-bench`，保留程序原始输出 |
| iozone | 多种读写模式与多进程测试，需要可写文件系统 |
| unixbench | 经管道提取程序性能输出；部分命令每次运行 10 或 20 秒，外层退出码不代表每段管道成功 |
| lmbench | 多种延迟、带宽测试，使用 `/var/tmp`、`/tmp` 和子进程 |
| iperf | 在客体 `127.0.0.1:5001` 启动后台服务器；组脚本没有停止服务器 |
| netperf | 在客体 `127.0.0.1:12865` 启动服务器，末尾执行 kill |
| cyclictest | 实时调度测试并启动 hackbench，末尾发 SIGINT；不能只看最后的 echo |
| ltp | 遍历 `ltp/testcases/bin`，逐个执行并打印返回值，需要 `/bin/bash` |

这些脚本使用相对路径，必须在各自 `/musl`、`/glibc` 工作目录执行。两组 iperf 共用端口且留下服务器，完整运行设计还需验证组间后台进程的生命周期；当前宿主入口只能保证结束 QEMU 时一并销毁客体进程，不能声称解决了组间干扰。不能以宿主网络参数替代客体回环网络支持。

## 当前 FocOS 的启动阻塞

补做实际启动：使用固定镜像、临时写入层及官方 1G 参数，当前内核打印 `FOCOS_PANIC` 和 `memory initialization failed: InvalidLayout`，QEMU 仍返回 0。代码 `kernel/src/platform/qemu_riscv64/memory.rs` 将 RAM 末尾固定为 `0x88000000`，拒绝范围外的设备树地址。

仅把内存改为 128M 做对照，输出 `FOCOS_BOOT_OK` 后退出，未运行官方测试。正式入口保留官方 1G 配置；内存支持作为内核依赖处理，不能悄悄缩小配置后称为官方环境。这项发现也要求运行状态只记录进程退出码，不能表述成“内核正常完成”。

## 本地小范围实验

从上述镜像重新提取 musl/glibc 的 BusyBox、Lua、脚本和 Lua 输入文件。分别放入临时 initramfs，在参考 Linux 上串行执行原始 `lua_testcode.sh`、`busybox_testcode.sh`。没有改动测试脚本和命令表；环境提供 `/proc`、`/sys`、`/dev`、BusyBox applet 和解释器路径。这是内存文件系统上的小范围环境实验，不是原始 Ext4 镜像全套运行。

参考内核 SHA-256：`7745b858c72bbd6e1b368e6057f5ec166319da6319ccf2cd0f40e8b609b6e638`；QEMU 10.2.2，virt、1 CPU、512M。它不同于官方参数，不能作为性能比较或 FocOS/VisionFive2 兼容成绩。

初次实验漏配 `/bin/busybox`，Lua 的 `test.sh` 无法执行，日志显示 `not found`，但 Lua 外层脚本和 QEMU 都退出 0。保留这份失败证据后，仅补上解释器路径重新运行，测试文件未改。补齐后两套 Lua 分别输出 9 条 success，BusyBox 两组均到达结束标记。将 musl BusyBox 开始/结束标记之间的原始字节交给上述官方 judge，输出唯一未通过条目为 `busybox kill 10`。这是实际官方工具的结果，不是自行计算的通过率；不把这次环境修正包装成 FocOS 修复。

实验命令、环境哈希和两轮完整串口日志保存在本地 `target/official-entry-research/`；该目录不入库。宿主入口的实际运行方法如下。

## 怎样独立核对 BusyBox 的差异

在 Linux 上准备 Bash、debugfs、Python 3 和 gh。在新的临时目录运行下面的命令，先将 `IMAGE` 设为上文发布镜像解压后的绝对路径。debugfs 默认只读。即使手头没有运行日志，也能用这个步骤比较镜像命令和判分工具要求的命令。

```bash
set -eu
: "${IMAGE:?请设置 IMAGE 为解压镜像的绝对路径}"
cd "$(mktemp -d)"
ref=d1bb3a3c4b27274e196a2648518525c1a304e339
echo "95973543db6b84a9a5e70f30da466ce292867aff5b689fb14c88dc9406e378b8  $IMAGE" | sha256sum -c -
for libc in musl glibc; do
    debugfs -R "dump /${libc}/busybox_cmd.txt ${libc}.commands" "$IMAGE"
    gh api -H 'Accept: application/vnd.github.raw+json' \
        "repos/oscomp/autotest-for-oskernel/contents/kernel/judge/judge_busybox-${libc}.py?ref=$ref" > "${libc}.py"
    echo "307c6d0325abe22e35c3c3cc27566b41f26598e1e690d7e2a504aadab678d03d  ${libc}.py" | sha256sum -c -
    grep -nF 'kill' "${libc}.commands"
    grep -nxF 'kill 10' "${libc}.py"
done
```

复核运行结果时，选取一次串口日志中对应 BusyBox 组开始与结束标记之间的完整字节，分别保存为 `musl.group`、`glibc.group`。在上一步的临时目录执行：

```bash
python3 musl.py < musl.group > musl.result.json
python3 glibc.py < glibc.group > glibc.result.json
```

本次用已有的两份参考 Linux 串口日志重放。两份官方判分脚本的哈希相同，各自产生的唯一零分条目都是 `{"name":"busybox kill 10","pass":0,"all":1,"score":0}`。对照两套镜像命令表，实际执行的是结束后台进程的 `kill $!`。这个差异已在两套 libc 上复现。2025 赛场采用的具体判分版本仍待 #48 核实。

## 在仓库运行

在 Linux 安装仓库 Rust 工具链、make、Git、Python 3.11 或更新版本、QEMU RISC-V 和 OpenSBI。当前入口读取系统固件 `/usr/share/qemu/opensbi-riscv64-generic-fw_dynamic.bin`，将只读副本放入本次输出目录，记录副本的 SHA-256 并用它启动 QEMU。从上面的发布链接下载并解压比赛镜像，在仓库根目录执行：

```bash
python3 scripts/official_test.py --image /绝对路径/sdcard-rv.img --out target/official/run-001
```

选择尚未存在的输出目录。每次运行使用输出目录下独立的 `cargo-target/` 构建、计算内核哈希并启动 QEMU。入口优先采用环境中的 `RUSTC`，其次采用 `CARGO_BUILD_RUSTC`，默认使用 PATH 中的 rustc。官方入口禁用 Cargo 的两层 rustc 包装器。传给 Cargo 的编译器和输出路径记录在 `run.json` 的 `build_env` 中。构建输出见 `build.log`，串口原文见 `serial.log`，QEMU 宿主诊断见 `qemu.stderr`。运行结束后查看 `run.json` 中的内核、固件和镜像哈希、QEMU 实际命令、版本和结束原因。当前内核在 1G 配置下输出内存初始化失败，尚未到达客体脚本。测试超时默认为 3600 秒。

1 GiB 内存布局修复由 [#65](https://github.com/MindScrath/Focos/issues/65) 跟踪。修复合入后，保留原运行目录，用相同镜像和 1 GiB 参数选择一个新输出目录复跑，对照修复前后的串口和内核哈希。

输出目录保留只读的镜像副本（逻辑大小 4 GiB），QEMU 从已校验的副本启动。运行记录的 `build_env` 保留传入的 Rust 编译标志，以及通过 `CARGO_PROFILE_RELEASE_*` 环境变量设置的 release 构建参数（例如优化级别和链接时优化）。Git 查询和构建保留工具所需的 `LD_LIBRARY_PATH`、`NIX_LD` 和 `NIX_LD_LIBRARY_PATH`，已设置的值记录在 `build_env`。宿主收到 SIGTERM 时先清理本次进程组，再记录退出码 143。

## 宿主进程验收

从仓库根目录执行。第一条命令单独检查入口，第二条运行仓库的完整检查。

```bash
python3 -B -m unittest discover -v -s scripts -p 'test_official_test.py'
make check
```

| 入口启动的外部命令 | 共用的启动与回收路径 | 可以核对的行为 |
| --- | --- | --- |
| 构建前后的 Git 查询 | `source_state → command_output → child` | Git 包装器启动后台进程后向入口发送 SIGTERM，辅助进程被回收。 |
| rustc、Cargo、QEMU 版本查询 | `probe_version → command_output → child` | 测试中断版本包装器并检查辅助 PID。版本标准输出写临时文件，后代持有写端不影响等待主命令退出。 |
| `make build` 及其编译器后代 | `main → child` | 等待 make 退出，再清理后台后代；构建失败或无法验证清理结果时停止。 |
| 运行 QEMU | `run_serial → child` | 替身退出 7，脱组辅助程序保持串口写端打开。测试核对串口原文、退出码 7 和辅助 PID 消失。挂起的替身另测超时。 |

`child` 是唯一启动外部命令的位置。入口要求 Linux 允许 `PR_SET_CHILD_SUBREAPER`，使孤儿后代重新归入口管理。查找子进程时优先读取 `/proc/thread-self/children`，缺少时枚举同一 PID 命名空间的 `/proc`，用 `waitid` 核对亲子关系。主命令退出后，只有内核通过 `waitid(P_ALL)` 报告已经没有子进程，才确认清理成功。进程列表偶尔漏掉某个 PID 时会重查；TERM 和 KILL 的等待期各最多 5 秒，期满仍无法确认则报错。

检查测试名中的 `without_proc_children`、`empty_proc_listing` 和 `existing_child`，可分别验证缺少可选文件时强制回收脱组进程、一次空枚举不会误判成功，以及启动前已有进程时停止并保留该进程。整个入口被 SIGKILL 后的清理由外部作业管理器负责。
