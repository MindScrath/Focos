# 任务导航

先看 [开放任务](https://github.com/MindScrath/Focos/issues?q=is%3Aissue%20is%3Aopen) 或 [未认领任务](https://github.com/MindScrath/Focos/issues?q=is%3Aissue%20is%3Aopen%20no%3Aassignee)，再按模块找到对应 Issue。负责人、当前进度、依赖、下一步与 SPEC/PR 记录统一维护在 Issue；认领前先读正文及最新讨论。

## 按模块找 Issue

| 模块 | 任务入口 | 职责与规格 |
| --- | --- | --- |
| 真板启动与设备资源 | [#2 VisionFive2](https://github.com/MindScrath/Focos/issues/2) | 启动、UART、时钟；[PLAT-001](../specs/PLAT-001.md)、[DRV-001](../specs/DRV-001.md) |
| 块设备与存储驱动 | [#3 SD / QEMU 块设备](https://github.com/MindScrath/Focos/issues/3) | 统一块接口、设备读写与同步；[HAL-001](../specs/HAL-001.md)、[TEST-002](../specs/TEST-002.md) |
| 内存 | [#4 内存与用户映射](https://github.com/MindScrath/Focos/issues/4) | 页分配、分页、隔离及 mmap；[MM-001](../specs/MM-001.md)、[MM-002](../specs/MM-002.md)、[MM-003](../specs/MM-003.md) |
| 执行与中断 | [#5 进程、ELF 与管道](https://github.com/MindScrath/Focos/issues/5) · [#35 本核临界区](https://github.com/MindScrath/Focos/issues/35) | 上下文、异常、调度及用户态；[ARCH-001](../specs/ARCH-001.md) |
| 文件系统与文件接口 | [#6 FAT32](https://github.com/MindScrath/Focos/issues/6) · [#22 Ext4](https://github.com/MindScrath/Focos/issues/22) · [#7 VFS / FD](https://github.com/MindScrath/Focos/issues/7) | 格式行为、路径、文件生命周期及持久化 |
| 官方测试接入 | [#8 测试接入](https://github.com/MindScrath/Focos/issues/8) · [#48 BusyBox 官方版本核对](https://github.com/MindScrath/Focos/issues/48) | 原 11 组与比赛冻结版本 LTP；[TEST-001](../specs/TEST-001.md)、[验证指南](verification.md) |
| Linux 运行时与网络 | [#23 动态加载、线程与信号](https://github.com/MindScrath/Focos/issues/23) · [#24 TCP/UDP loopback](https://github.com/MindScrath/Focos/issues/24) | ABI、阻塞唤醒和程序依赖 |
| 开发工具 | [#25 vim/gcc/make](https://github.com/MindScrath/Focos/issues/25) | 编辑保存、编译运行、完整与增量构建 |
| 性能与交付 | [#9 缓存实验](https://github.com/MindScrath/Focos/issues/9) · [#10 材料与提交](https://github.com/MindScrath/Focos/issues/10) | [性能实验](benchmarks.md)、独立复现、证据与许可 |
| 全国赛扩展 | [#11 LoongArch / 星云板](https://github.com/MindScrath/Focos/issues/11) · [#12 扩展负载](https://github.com/MindScrath/Focos/issues/12) | 第二架构、真板及扩展兼容与检索负载 |

## 接口先找谁确认

平台与设备按 [架构交接](architecture.md) 分工：CPU 机制归 arch，板级资源与装配归 platform，控制器操作归 drivers；公共文件系统通过统一接口使用设备。跨层接口由调用方与实现方共同审阅。

内存与执行接口由 #4/#5 共审；FAT32、Ext4 和 VFS 由 #6/#22/#7 共审。测试失败按实际缺失能力分派给相应模块，测试负责人负责复现与证据，不包办全部内核实现。

## 怎样开始一次交付

在 Issue 确认是否可认领，约 reviewer，按 [SPEC 模板](../specs/TEMPLATE.md) 和 [六步 SOP](collaboration.md) 交付一个能验证的行为。规格审定后先写失败测试，再实现、回归并提交 PR；首份交付尽量控制在 3–6 小时。

交付范围见 [REGION-001](../specs/REGION-001.md)，阶段与平台顺序见 [路线](roadmap.md)。新增或替代模块入口时维护本页链接；具体任务变化在 Issue 更新。
