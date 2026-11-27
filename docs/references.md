# 来源：需要时再查

## 区域赛规则

工作区 `contest_info/` 的 2026 年 10 月原件优先于旧摘要：

| 原件 | 查什么 | SHA256 |
| --- | --- | --- |
| 2026年操作系统设计赛华东区域赛章程-1010.pdf | 赛程、组队、来源与 AI 要求 | `ec70009584de03dd1a48a4b7e9ef7d25abdce33c8edcf2280eee8d71f54da16b` |
| 2026年操作系统设计赛华东区域赛技术方案-1010.pdf | 评分、交付和 GitLab | `87aa31339caf043bc3d6a740c0aa735a2bd6411cfd6cedb370ef98555d47571d` |

计划按 12/24 提交，12/20 准备候选版。技术方案参考权重：代码 30%、文档 30%、教学 10%、工程规范 10%、演示答辩 20%。新加的 2025 测试目标是团队要求，不是官方最低门槛。

要求公开代码/文档、保留开发与来源记录，并通过指定 GitLab 提交。代码需选开源许可，文档按 CC-BY-SA 4.0；当前仓库尚待补齐许可文件，由 #10 跟进。GitHub 公有仓库不等于赛方提交已完成。

入口：[官网](https://os.xtnl.org.cn/#/index?TYPE=26OS_K)。本次网页正文未能直接读取；最新通知仍需提交前核对。个人报名材料不发布。

## 测评来源

- [2025 初赛](https://github.com/oscomp/testsuits-for-oskernel/tree/8b58dd16d26d30f7c74d48d5832d870d3051b703)：区域赛兼容基线。
- [2025 现场](https://github.com/oscomp/testsuits-for-oskernel/tree/9ea291d8535774680e88f4e6bf725d7ba156e752)：vim/gcc 来源与任务。
- [2026 决赛](https://github.com/oscomp/testsuits-for-oskernel/tree/b5ec6ef8497e1818cbdec3b54bb722f036e57972)：CAgent/BuildStorm 进阶参考。
- [官方运行框架](https://github.com/oscomp/autotest-for-oskernel/tree/d1bb3a3c4b27274e196a2648518525c1a304e339)、[测例与 judge 说明](https://github.com/oscomp/oskernel-testsuits-cooperation/tree/44fa7ac65f292580c3974a8252f533c4484ebe6b)。

完整分支比较、镜像差异与未确认项见 [审计](reference/test-audit.md)。不要把历史适配结果写成当届官方成绩。

## 实现与研究参考

- [JH7110 官方资料](https://doc-en.rvspace.org/JH7110/Product_Brief/JH7110_DS/feature_pb.html)、[2K1000LA](https://loongson.cn/EN/news/show?id=636)：设备资料入口。
- [SBI System Reset](https://github.com/riscv-non-isa/riscv-sbi-doc/blob/master/src/ext-sys-reset.adoc)：启动关机规范。
- [获奖作品索引](https://github.com/oscomp/os-competition-info/blob/main/os-kernel-winners.md)、[Starry Mix](https://github.com/Starry-Mix-THU/starry-mix)：针对具体问题阅读，记录版本、借鉴范围与本队增量。
- [HNSW](https://arxiv.org/abs/1603.09320)、[DiskANN](https://papers.neurips.cc/paper/2019/hash/09853c7fb1d3f8ee67a61b6bf4a7f8e6-Abstract.html)、[MemGPT](https://arxiv.org/abs/2310.08560)：算法与应用动机；论文性能不是 FocOS 数据。

引入第三方代码前核对许可证，在源码和设计/答辩材料注明来源。AI 辅助与人工复核见 [记录](ai-usage.md)。

## 内核分页参考

- [RISC-V 特权规范 20240411：Supervisor](https://docs.riscv.org/reference/isa/v20240411/priv/supervisor.html)：MM-003 的 Sv39、分页切换、地址转换缓存同步及异常语义依据；亦为 MM-004 的 `sstatus.SUM`/`MXR`、Sv39 `U` 位与 S 模式取指禁令依据（详见 [MM-004 参考页](reference/user-mapping.md#规范依据)）。
- [OpenSBI v1.3.1](https://github.com/riscv-software-src/opensbi/tree/v1.3.1)：核对已有 QEMU 环境中的异常转交；具体源码及借鉴范围见 [MM-003 参考页](reference/sv39-paging.md#规范依据)。

- [Rust 内联汇编参考](https://doc.rust-lang.org/reference/inline-assembly.html)：核对寄存器和编译器访存约束；`satp`、`sfence.vma`、`fence.i` 汇编不使用 `nomem`。
- [Rust 未定义行为参考](https://doc.rust-lang.org/reference/behavior-considered-undefined.html)：核对页访问闭包的有效存储、对齐、引用独占及有效期前提，用于补全 MM-003 的安全注释。
- [Rust MaybeUninit](https://doc.rust-lang.org/std/mem/union.MaybeUninit.html#method.as_mut_ptr)：核对先写入初始化、再形成字节引用的顺序，避免依赖空闲 RAM 已初始化。

以上资料用于 MM-003 的设计约定，未引入外部实现代码。运行证据由实现 PR 提供。

## 匿名映射与堆顶

- [MM-005](../specs/MM-005.md) 的匿名页初始清零、内核选址、按页撤销及空洞处理参照 [Linux man-pages 5.13 的 mmap(2) 与 munmap(2)](https://raw.githubusercontent.com/mkerrisk/man-pages/man-pages-5.13/man2/mmap.2)。
- 堆顶按字节记录、映射按页增减。Linux 原始 `brk` 在堆增长因空间不足被拒时返回旧堆顶，FocOS 内存层保留具体错误供 [#23](https://github.com/MindScrath/Focos/issues/23) 对接。参见 [Linux v6.12 的 brk 系统调用](https://github.com/torvalds/linux/blob/v6.12/mm/mmap.c#L103-L204)。

## 多平台设计参考

[KernelX 固定快照](https://gitlab.eduxiji.net/educg-group-48535-3229427/T2026103369910030-2889/-/tree/12ac84b6401cbddf75ca937ad485f8437cb3dc1d)（2026-11-11 源码检查）：

- [架构入口与统一类型](https://gitlab.eduxiji.net/educg-group-48535-3229427/T2026103369910030-2889/-/blob/12ac84b6401cbddf75ca937ad485f8437cb3dc1d/kernelx/src/arch/mod.rs)、[CPU/页表/上下文接口](https://gitlab.eduxiji.net/educg-group-48535-3229427/T2026103369910030-2889/-/blob/12ac84b6401cbddf75ca937ad485f8437cb3dc1d/kernelx/src/arch/arch.rs)：参考编译时架构选择与 ABI 隔离。
- [设备匹配](https://gitlab.eduxiji.net/educg-group-48535-3229427/T2026103369910030-2889/-/blob/12ac84b6401cbddf75ca937ad485f8437cb3dc1d/kernelx/src/driver/matcher.rs)、[VirtIO 传输适配](https://gitlab.eduxiji.net/educg-group-48535-3229427/T2026103369910030-2889/-/blob/12ac84b6401cbddf75ca937ad485f8437cb3dc1d/kernelx/src/driver/virtio/matcher.rs)、[DMA 适配](https://gitlab.eduxiji.net/educg-group-48535-3229427/T2026103369910030-2889/-/blob/12ac84b6401cbddf75ca937ad485f8437cb3dc1d/kernelx/src/driver/virtio/hal.rs)：参考统一设备接口与底层资源适配。

- [VirtIO 驱动内缓存](https://gitlab.eduxiji.net/educg-group-48535-3229427/T2026103369910030-2889/-/blob/12ac84b6401cbddf75ca937ad485f8437cb3dc1d/kernelx/src/driver/block/virtio.rs)、[双架构构建及启动配置](https://gitlab.eduxiji.net/educg-group-48535-3229427/T2026103369910030-2889/-/blob/12ac84b6401cbddf75ca937ad485f8437cb3dc1d/build-kernel.sh)：据此补充 FocOS 的公共缓存边界与平台运行配置要求。

本次借鉴设计思路，未引入代码。FocOS 另行明确板级层，保留 HAL-001 的错误及同步语义；早期静态装配，按需扩展。上述源码路径存在不代表两块真板均已验证，本次未运行 KernelX。

## QEMU 异常与时钟规格

- [SBI v2.0 TIME](https://github.com/riscv-non-isa/riscv-sbi-doc/blob/v2.0/src/ext-time.adoc)：绝对期限、清除 pending 与撤销方式。
- [Devicetree Specification v0.4：DTB 格式](https://github.com/devicetree-org/devicetree-specification/blob/v0.4/source/chapter5-flattened-format.rst)：板级最小频率读取器的编码与边界依据。

以上用于 [ARCH-002](../specs/ARCH-002.md) 设计；沿用 RISC-V 特权规范 20240411，未导入外部实现，运行验收待后续实现。
