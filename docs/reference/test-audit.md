# 选哪年的测例，为什么

2026-11-06 复核官方仓库的 19 个可见分支，**未发现 `pre-2026` 分支**。这不表示不存在其他发布渠道。固定提交见 [sources.lock.json](../../tests/sources.lock.json)；本次只审源码与发布信息，未运行测例。

## 区域赛必须完成什么

采用 **2025 初赛测试 + 2025 现场开发工具**，另加团队自己的 make 构建检查。下表是完成标准，当前尚未通过。

| 采用的资料 | 要做什么 | 在哪里验收 |
| --- | --- | --- |
| `pre-2025`：2025 年全国赛初赛 | 原 11 组与比赛 LTP 的所有纳入子用例都要通过；musl、glibc 两种版本分别运行 | QEMU RISC-V |
| `on-site-final-2025`：2025 年全国赛现场题 | vim 能编辑、保存并重新打开文件；gcc 能在 FocOS 内编译、链接并运行新程序 | QEMU RISC-V 和 VisionFive2 |
| 团队新增的 make 检查 | 构建一个含两个源文件的小工程；修改一个文件后只重编受影响目标；`make clean` 能清理产物 | QEMU RISC-V 和 VisionFive2 |

LTP 是 Linux 内核测试套件，按 2026-11-18 的 #8 扩围纳入本阶段；以比赛冻结版本与镜像为准，具体调用列表仍待审计。musl 和 glibc 是两种 C 标准库；官方提供了基于它们构建的两套程序，不能只测其中一套。

这里的“通过”包括检查程序结果和实际文件操作，不是脚本执行到了末尾。子用例清单仍需随镜像核对后冻结。LoongArch 留到全国赛；真板文件闭环和缓存实验见 [完整交付标准](../../specs/REGION-001.md)。

## 其他年份的资料用来做什么

这些资料不增加区域赛必过项，也不能抵扣上面的任务。

| 资料 | 具体用途 | 什么时候用 |
| --- | --- | --- |
| `pre-2024`：2024 年初赛 | 用 32 个较小的基础程序按需调试系统调用和 FAT32；区域赛仍按 2025 全套测试验收 | 开发早期 |
| `final-2023`：2023 年决赛 | 参考其中的 FAT32 文件读写负载，准备存储实验；结果单独记录 | 做 FAT32 实验时按需选用 |
| `final-2025`：2025 年线上决赛 | 补测文件复制（`copy_file_range`）、管道与文件间传输（`splice`）、中断统计 | 区域赛必达完成后选做 |
| `final-2026`：2026 年线上决赛 | CAgent 检查命令、文件等操作；BuildStorm 要求在内核上运行 Rust 工具链并编译复杂项目 | 全国赛扩展参考 |

其余分支：`main/master/testscripts/pre-2023` 是早期基础参考；`final/final-comp/libc-test` 是旧综合测试；`pre-2024-riscv64/aarch64/x86_64` 不能仅按名字认定平台；`final-2024-rv/la` 是分架构 Ext4 测试；`2025_multiarch` 供构建差异核对；`on-site-final-2023/2024` 留作历史参考。不要把 main 当成最新赛题。

## 测试组带来哪些内核能力

| 测试组 | 重点依赖 |
| --- | --- |
| basic、busybox、lua | 文件、目录、进程、shell、时间 |
| libctest、libcbench | libc 语义、动态加载、TLS、mmap 与线程等，逐子项核对 |
| iozone、unixbench、lmbench | 文件 I/O、进程/管道、映射、信号和计时 |
| iperf、netperf | TCP/UDP、loopback、并发连接、阻塞与退出 |
| ltp | 按冻结调用逐项审计系统调用、权限、设备及环境前提；缺能力不豁免 |
| cyclictest | 线程、定时、调度/亲和性接口、压力子进程与信号 |

组名和脚本固定在 [regional-plan.json](../../tests/regional-plan.json)。**全部**指脚本展开后的子用例全部满足判据；不是只运行组脚本或只选 musl。性能程序应完成操作、生成有效结果，不要求追平 Linux 性能或宣称历史满分。

## 镜像与工具：已知道什么，尚缺什么

- [2025 源码与说明](https://github.com/oscomp/testsuits-for-oskernel/tree/8b58dd16d26d30f7c74d48d5832d870d3051b703)：原环境 Ext4，构建同时生成 musl/glibc，README 要求 RV/LA 两架构；本队区域赛仅认领 RISC-V，LoongArch 后续补。
- [2025 初赛发布镜像](https://github.com/oscomp/testsuits-for-oskernel/releases/tag/pre-20250615)：可作候选；其发布日期与源码 Makefile 的 `20250624` 容器标签不同。#8 必须核对脚本/二进制对应关系，不能直接视为同版本。
- [2025 现场说明](https://github.com/oscomp/testsuits-for-oskernel/blob/9ea291d8535774680e88f4e6bf725d7ba156e752/README.md)包含 vim 编辑保存、gcc 编译运行；**make 不是其中的独立题**。
- [现场 RV 镜像](https://github.com/oscomp/testsuits-for-oskernel/releases/tag/on-site-final-2025-rv64-fs)可作工具来源；[soft-info](https://github.com/oscomp/testsuits-for-oskernel/blob/9ea291d8535774680e88f4e6bf725d7ba156e752/soft-info.txt)列 GCC 14.2.0、Vim 9.1 等，但有 LoongArch 输出，RISC-V 二进制版本仍需从实际镜像核实。
- 当前只取得 GitHub 发布元数据中的压缩包 digest，未下载校验、挂载镜像或冻结 make 版本。候选 URL/digest 与待办项保存在计划 JSON，不能当作已验收锁文件。

## runner 必须处理的坑

1. `pre-2025/scripts/test_all.sh` 有旧入口，不能直接当完整清单。以组脚本及其实际子程序展开；libctest 包含 static/dynamic 两轮。
2. iperf/netperf 使用 `127.0.0.1`。先跑真实回环协议，不要求凭空增加物理网卡验收，也不能伪造返回。
3. 旧 basic runner 漏 `sleep` 并误用 `yield.sh`；`exit.c` 依赖 fork/wait。起步清单保留这些历史记录。
4. mount 测试请求 vfat，并涉及设备/挂载点夹具。准备真实设备与目录；argv/打包适配留补丁，不吞错误。
5. 原 judge、容器、镜像版本要对应。失败、超时、缺失输出都保留；文件操作检查真实副作用。

[2026 决赛](https://github.com/oscomp/testsuits-for-oskernel/blob/b5ec6ef8497e1818cbdec3b54bb722f036e57972/README.md)的 BuildStorm 包含 glibc、Rust/cargo 复杂构建与多核条件，不能当作 vim/gcc 的等价替代。2026 参考任务和下一届正式要求分别记录。
