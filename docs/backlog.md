# 选一个 3–6 小时的第一步

在 Issue 认领、约 reviewer，再按 [六步 SOP](collaboration.md) 先审 SPEC、写失败测试、实现。负责人、阻塞和完成状态以 Issue 为准；本页只提供任务入口。

## 现在可认领的三个小任务

PR #32 的架构约定已经合入，先把以下边界落实到实现与测试。

| 子任务 | 第一份交付 | 对应主任务 |
| --- | --- | --- |
| [#33 抽离 QEMU 板级配置与启动内存](https://github.com/MindScrath/Focos/issues/33) | 列迁移清单、审 SPEC；保持现有布局和错误行为，回归 BOOT-001/MM-002 | #4 内存，与 #2 对齐板级入口 |
| [#34 抽取共享块设备契约测试](https://github.com/MindScrath/Focos/issues/34) | 共享检查已入 `kernel/src/block/contract.rs`（含 VirtIO/SD 适配入口说明）；后端提供设备与专用测试盘后接入 | #3 设备，复用已完成的 #1 |
| [#35 实现可嵌套本核临界区](https://github.com/MindScrath/Focos/issues/35) | 先写 QEMU RISC-V 开/关中断与嵌套状态测试，再实现接口 | #5 进程，与 #2 联调时钟 |

#33/#34 是重构，#35 是新增能力。前两项可用现有基线独立推进；#35 状态测试可先做，关闭任务前还须完成真实定时中断验收。三个任务都不代表整个父任务完成。

已完成的基础：[#1 块设备接口与内存盘](https://github.com/MindScrath/Focos/issues/1) 已关闭；MM-001 页分配与 [PR #31 的 QEMU 真实内存接入](https://github.com/MindScrath/Focos/pull/31) 已合入。继续复用，不重新认领已有实现；真板内存和用户映射仍在 #4 推进。

## 基础模块继续往哪里走

| 主任务 | 下一步入口 |
| --- | --- |
| [#2 真板启动、UART 与时钟](https://github.com/MindScrath/Focos/issues/2) | 固定平台运行配置并独立冷启动；按板级/驱动边界接自有设备 |
| [#3 SD 与 QEMU 块设备](https://github.com/MindScrath/Focos/issues/3) | 先读 QEMU 已知扇区；SD 资源调查可并行，共用 #34 的测试逻辑 |
| [#4 内存与用户映射](https://github.com/MindScrath/Focos/issues/4) | #33 板级迁移；随后按专项 SPEC 推进页表、隔离与 mmap |
| [#5 进程、ELF 与管道](https://github.com/MindScrath/Focos/issues/5) | #35 临界区及上下文/异常事件 SPEC，再接两任务与用户 ELF |
| [#6 FAT32](https://github.com/MindScrath/Focos/issues/6) | 内存盘 BPB/簇链测试；真实驱动接入前可先推进格式行为 |
| [#7 VFS 与文件接口](https://github.com/MindScrath/Focos/issues/7) | 路径、FD 生命周期与共享偏移，不依赖具体板型或控制器 |

平台任务先读 [架构与接口交接](architecture.md)：#2/#3 对齐 PLAT-001、DRV-001，#4/#5 对齐 ARCH-001。跨层接口由调用方与实现方共同审阅，认领时说明向谁要资源、向谁交接口。

## 区域赛兼容工作

| 任务 | 第一份交付 |
| --- | --- |
| [#8 完整测例接入](https://github.com/MindScrath/Focos/issues/8) | 冻结镜像、子用例和判据；按 Issue 当前认领记录推进 |
| [#22 Ext4](https://github.com/MindScrath/Focos/issues/22) | 镜像特性调查与 host 只读测试 |
| [#23 动态加载、线程、信号](https://github.com/MindScrath/Focos/issues/23) | 动态 ELF 的解释器/库依赖；架构 ABI 与公共运行时分工 |
| [#24 TCP/UDP loopback](https://github.com/MindScrath/Focos/issues/24) | 一对客户端/服务端收发及超时测试 |
| [#25 vim/gcc/make](https://github.com/MindScrath/Focos/issues/25) | 工具版本、依赖与终端需求 |

基础运行依赖 #2/#4/#5；文件依赖已完成的 #1 与 #3/#6/#7。Ext4 和运行时调查可提前开始，运行验收要等相关能力接通。#8 的失败结果分派给能力 owner，不由测试负责人包办所有内核实现。

## 实验与全国赛

- [#9 缓存实验](https://github.com/MindScrath/Focos/issues/9)：公共缓存策略、统一预算与统计位置明确后，比较 B0/B1。
- [#10 提交与教学材料](https://github.com/MindScrath/Focos/issues/10)：独立复现、许可与赛方提交项。
- [#11 LoongArch 与星云板](https://github.com/MindScrath/Focos/issues/11)：资料可先调查；实现先验 QEMU 架构，再验真板资源与设备。
- [#12 全国赛扩展](https://github.com/MindScrath/Focos/issues/12)：双平台、LTP、2026 参考负载和真实磁盘检索。

交付边界见 [REGION-001](../specs/REGION-001.md)，检查点见 [路线](roadmap.md)。
