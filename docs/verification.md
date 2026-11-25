# 怎样证明功能可用

**当前没有历史测例通过成绩。** 启动检查和真板首次输出分别有证据，不能替代下面的验收。

## 本次选哪套

区域赛主线为 `pre-2025` 的全部测试（原 11 组及比赛 LTP），RISC-V 的 musl/glibc 都纳入。`pre-2024` 32 个 basic 程序只用于早期定位，不再是区域赛交付上限。

| 内容 | 入口 |
| --- | --- |
| 交付范围与平台 | [REGION-001](../specs/REGION-001.md) |
| 测试组与来源参考 | [regional-plan.json](../tests/regional-plan.json) |
| 历史分支固定提交 | [sources.lock.json](../tests/sources.lock.json) |
| 起步用 basic 清单 | [catalog.json](../tests/catalog.json) |
| 版本选择、工具来源和已知坑 | [测例审计](reference/test-audit.md) |

## 接入顺序

1. **核实官方流程。** 对照比赛版本确认测试源码、镜像、脚本、运行环境及官方判分方式，已知 BusyBox 版本差异由 [#48](https://github.com/MindScrath/Focos/issues/48) 跟踪。
2. **提供运行入口。** 在仓库根目录执行 `python3 scripts/official_test.py --image /绝对路径/sdcard-rv.img --out target/official/run-001`，选择新的输出目录。串口原文见 `serial.log`，运行条件及结束原因见 `run.json`。依赖与镜像下载见[运行说明](reference/official-test-entry.md#在仓库运行)。
3. **验证内核兼容。** 在 FocOS 上运行规定范围的原版测试，musl/glibc 分开记录。沿用对应版本的官方判分工具；没有工具时按官方标准核对，不自行生成通过率。参考 Linux 仅用于检查环境和运行方式。
4. **接工具演示。** 2025 原版 vim/gcc，加已锁定来源的 make；编辑、构建、执行并检查磁盘副作用。版本输出只是加载检查。

全部纳入的比赛测试通过才完成区域赛测试目标；脚本结束或退出 0 不代表全套通过。旧分母、槽位、PROFILE/CRITERIA 和自研判分要求已停用，见 [TEST-001](../specs/TEST-001.md)。

## 每次留下什么

保存内核版本、测试与工具版本、镜像身份、环境、完整命令、原始输出和进程退出情况。若运行官方判分工具，同时保存其版本、命令与原始报告；不修改输出来适配旧解析器，不用后来的成功覆盖失败记录。

QEMU、VF2、LoongArch 分开报告；原版 Ext4 镜像与适配镜像分开报告。改 argv、打包或脚本须留补丁，不能改被测语义。写入和工具构建同时核对文件内容与新产物。

额外自测覆盖坏指针、OOM、异常隔离、簇链环、跨簇写、失败同步及资源释放。官方测试只证明它实际覆盖的行为。
