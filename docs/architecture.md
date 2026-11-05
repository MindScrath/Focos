# 从用户程序到磁盘

这张图画的是 FocOS 的目标架构。当前已验证 QEMU 启动、SBI 输出与关机，以及 VisionFive2 上 U-Boot 加载 FocOS 后的 SBI 输出（[实测记录](evidence/BOARD-001-20261105.txt)）；真板 UART/SD 驱动和其余内核功能待实现。区域赛先在 VisionFive2 上接通整条路径，全国赛再移植到星云板并验证同一套文件系统。

```mermaid
flowchart TB
    accTitle: FocOS 目标架构
    accDescr: 用户程序经系统调用访问 FAT32。HAL 和 arch 隔离两块板的差异。已验证 QEMU 启动及关机、VisionFive2 bootelf 启动与 SBI 输出；真板驱动待实现。

    subgraph USER["用户态"]
        DEMO["区域赛<br/>演示程序与历史测例"]
        ANN["全国赛扩展<br/>磁盘向量检索"]
    end

    subgraph KERNEL[" "]
        SYS["共享内核 · Rust no_std<br/>系统调用"]
        VFS["VFS · 路径与文件描述符"]
        FAT["FAT32 · 目录与簇链"]
        CACHE["块缓存 · 可旁路做对照"]
        CORE["内存管理 · 异常与中断<br/>调度 · 用户程序装载"]
        SYS --> VFS --> FAT --> CACHE
        SYS --> CORE
    end

    HAL["HAL / arch<br/>设备接口 · 启动 · CPU 操作"]
    VF2["VisionFive2 · JH7110<br/>已实测 bootelf / SBI 输出<br/>SD / UART 驱动待实现"]
    LA["2K1000LA 星云板<br/>全国赛扩展 · LoongArch64"]
    QEMU["QEMU virt · 已验证<br/>启动、SBI 输出与关机"]

    DEMO --> SYS
    ANN --> SYS
    CACHE -->|扇区读写与同步| HAL
    CORE -->|页表、中断与上下文切换| HAL
    HAL --> VF2
    HAL --> LA
    HAL -. 启动基线 .-> QEMU

    classDef future stroke-dasharray: 5 3;
    classDef verified stroke-width: 3px;
    class ANN,LA future;
    class QEMU verified;
```

## 跟着一次读文件走

用户程序发出系统调用。VFS 是统一的文件接口，负责解析路径并管理文件描述符。FAT32 沿目录和簇链找到文件数据。块缓存可以复用已经读过的扇区，未命中时再通过 `BlockDevice` 接口交给板级驱动读取。

移植星云板时，启动、页表、中断和设备驱动在 HAL 与 arch 层适配。VFS、FAT32 和缓存继续使用同一份源代码，按相同的文件操作用例验收。

## 未来怎样验证优化

1. **先跑通区域赛闭环。** 在 VisionFive2 上用用户程序创建文件、跨簇写入、同步后重启读回。历史测例用于核验它们覆盖的系统调用行为。
2. **再解释缓存的收益。** 在同一板卡上用同一镜像和请求序列比较缓存开关，记录物理 I/O 次数、内存开销和尾延迟。星云板移植完成后，重复同样的对照。
3. **最后换成真实应用负载。** 全国赛用用户程序从 FAT32 文件按需读取图节点和向量。在同一召回率和内存预算下，对比每次查询的 I/O 次数与端到端延迟。

如果当届全国赛要求其他文件系统或更完整的 Linux ABI，分别在 VFS 下增加兼容后端、扩展系统调用层。FAT32 继续作为基础文件系统和优化对象。

按周验收看 [推进路线](roadmap.md)，接口细节看 [详细设计约束](reference/delivery-details.md)，采样与绘图看 [性能实验](benchmarks.md)。
