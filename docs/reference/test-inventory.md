# TEST-001：输入冻结与子用例清单

关联：[Issue #8](https://github.com/MindScrath/Focos/issues/8) · 状态：草案（待审）· 冻结日期：2026-11-08

本文记录区域赛测试的**输入版本、子用例清单、判据来源和已知差异**。它是 `tests/inventory.json` 的说明页；执行结果不写在这里，写在 `tests/results/` 或 PR 证据中。

## 一句话结论

11 组非 LTP 共展开 **427 个子用例**。其中 5 组有可作 stdin 判分的官方 judge，6 组使用团队判据（lmbench 的官方脚本是运行器）。按当前发布镜像：**glibc 上限 423/427**（busybox 1 项、libctest 3 项在镜像中不存在）；**musl 上限 ≤391/427**（再扣 basic 的 32 项，因为 musl basic 实际是 glibc 构建且缺 loader）。

## 1. 冻结输入

| 输入 | 值 | 校验状态 |
| --- | --- | --- |
| 测例源码 | `oscomp/testsuits-for-oskernel`，分支 `pre-2025`，commit `8b58dd16d26d30f7c74d48d5832d870d3051b703` | 与 `tests/regional-plan.json` 一致 |
| 初赛镜像 | release `pre-20250615`，`sdcard-rv.img.xz`，284,247,048 字节 | 本地 SHA256 校验通过 |
| 镜像压缩包 SHA256 | `2b59aee4d26681b2d78a8faa615ac216f723a8ea157544e7867bbc8c77aac087` | 与 release 元数据一致 |
| 镜像解压后 | 4,294,967,296 字节（4 GiB） | 本地 SHA256 `95973543db6b84a9a5e70f30da466ce292867aff5b689fb14c88dc9406e378b8` |
| 官方 judge | `oscomp/oskernel-testsuits-cooperation`，commit `44fa7ac65f292580c3974a8252f533c4484ebe6b` | 已拉取到 `third_party/`（不入库） |
| 参考 QEMU | 9.2.4（xPack，参考运行）；平台 README 记录 9.2.1 | 参考运行已完成，见 [reference-run.md](reference-run.md) |

## 2. 11 组清单

| 组 | 子用例数 | 判据来源 | judge 文件 | 说明 |
| --- | ---: | --- | --- | --- |
| basic | 32 | 官方 | `judge_basic.py` | 解析 `========== START/END`，逐用例 assert；内部类名 `dup2_test`，输出名 `test_dup2` |
| busybox | 53 | 官方 | `judge_busybox.py` | 镜像有 55 条命令；judge 去重后 53 项 |
| lua | 9 | 官方 | `judge_lua.py` | 固定 9 个 `.lua` |
| libctest | 220 | 官方 | `judge_libctest.py` | static 109 + dynamic 111；镜像实际 107 + 110 |
| iozone | 20 | 官方 | `judge_iozone.py` | 20 个吞吐指标；无 `pass` 字段，按“结果为正”判完成 |
| unixbench | 27 | 团队 | `judge_team/judge_unixbench.py` | 官方未公开 judge；按输出行是否有正数判完成 |
| iperf | 6 | 团队 | `judge_team/judge_iperf.py` | 按 `end: success/fail` 标记 |
| libcbench | 27 | 团队 | `judge_team/judge_libcbench.py` | 27 个内部 benchmark；标签由 `main.c` 的 RUN 行还原 |
| lmbench | 24 | 团队 | `judge_team/judge_lmbench.py` | 官方 `lmbench.sh` 是运行器而非 stdin 判分器；团队按关键字匹配，置信度中 |
| netperf | 5 | 团队 | `judge_team/judge_netperf.py` | 按 `end: success/fail` 标记 |
| cyclictest | 4 | 团队 | `judge_team/judge_cyclictest.py` | 4 个 cyclictest 场景；hackbench 只是负载 |

合计 **427** 项：32 + 53 + 9 + 220 + 20 + 27 + 6 + 27 + 24 + 5 + 4。

## 3. 镜像与源码的已核实差异

逐脚本对比（去掉 `-musl/-glibc` 后缀、统一换行）后：

1. **组名后缀**：镜像里的脚本输出 `basic-musl`、`basic-glibc` 等；源码和 judge 文件名不带后缀。平台如何映射尚未公开，runner 统一归一化后再找 `judge_<group>.py`。
2. **11 组 `*_testcode.sh` 与源码一致**，包括 `basic/run-all.sh`。差异集中在两个数据文件：
3. **busybox 命令清单**：镜像 55 条，源码 55 条，但两处不同——镜像用 `rm test.txt` / `rm busybox_cmd.bak`，源码用 `rm test.txt -f` / `rm busybox_cmd.bak -f`。judge 内嵌清单与镜像一致（无 `-f`），但仍有 3 个不一致：
   - judge 有 `kill 10`，镜像没有 → 该项必然 0 分；
   - 镜像有 `printf "abc\n"`，judge 没有 → 不计分；
   - 镜像有 `sh -c 'sleep 5' & ./busybox kill $!`，judge 没有 → 不计分。
   因此 busybox 最高只能拿 **52/53**。
4. **libctest 名单**：镜像 `run-static.sh` 107 项、`run-dynamic.sh` 110 项；judge baseline 是 109 + 111。缺失项为 static 的 `crypt`、`pleval` 和 dynamic 的 `crypt`。在 `entry-static.exe` 中搜索不到这三个字符串，说明镜像二进制本身不支持，不能靠补脚本解决。因此 libctest 最高只能拿 **217/220**。
5. **LTP**：镜像包含 LTP，本计划明确排除；任何结果表都不得把 LTP 计入分母。
6. 镜像中包含 musl/glibc 两套完整二进制（busybox、lua、iozone、iperf3、netperf、netserver、cyclictest、hackbench、lmbench_all、libc-bench、entry-static/dynamic.exe、runtest.exe），每组约 118 个文件。
7. **basic 的 musl 不是独立构建**：`/musl/basic/{brk,chdir,getpid}` 与 `/glibc/basic/...` 的 SHA256 完全相同；`/musl/lib` 只有 `libc.so`，没有 `ld-linux-riscv64-lp64d.so.1`。参考运行中 musl basic 0/32，不能把 musl basic 当成独立的一套。详见 [reference-run.md](reference-run.md)。

## 4. 判分协议与已知坑

平台契约（来自 judge 仓库 `README.md`）：

- 捕获 `#### OS COMP TEST GROUP START xxxxx ####` 与 `END xxxxx ####` 之间的文本，经 **stdin** 传给 `judge_xxxxx.py`；
- judge 输出 JSON 列表：`[{"name": ..., "all": ..., "pass": ..., "score": ...}]`；
- 缺失测试点必须补 0；即使输入是 EOF，也要输出全部测试点且全 0。

已核实的 judge 行为：

| 现象 | 影响 | 处理 |
| --- | --- | --- |
| `judge_basic.py` 的类名是 `dup2_test`，但 `TestBase` 输出名为 `test_dup2` | 容易误读 | 清单从 judge 空输入的 `name` 字段生成，使用 `test_dup2`；空输入和真实日志均已核对 |
| `judge_basic.py` 向 stderr 打印 `dict_keys([...])` | 不影响 stdout JSON | runner 只在非零退出/JSON 解析失败时使用 stderr |
| `judge_libctest.py` 输出 `total` 而不是 `all` | 通用 runner 会误判 | runner 兼容 `all`/`total` |
| `judge_iozone.py` 没有 `pass` 字段；`res` 笔误只影响 latency 键 | 通用 runner 会记 0 分；iozone baseline 全是 kb/sec，不触发笔误 | 清单标 `performance`；不设 fallback，judge 出错即 `JUDGE_ERROR` |
| `lmbench.sh` 会实际执行命令，不读 stdin | 不能当 judge 用 | 使用团队解析器；结果标 `judge_source: team` |
| `lmbench.sh` 第 22/23 项重名 | 官方脚本自身缺陷 | 清单按命令修正为 `mmaponly` / `open2close` |
| 6 组没有可用的官方 stdin judge（含 lmbench） | 无法声称“官方判分” | 团队判据明确标注，不冒充官方 |

## 5. 分母政策

- **评分分母用 judge 清单（427）**，因为它决定了平台能给你多少分；
- **执行能力用镜像清单**（busybox 52 可执行、libctest 217 可执行），用于解释为什么有些项永远是 0；
- 在输入未冻结、失败未复现前，不发布“总通过率”；
- 任何对镜像或脚本的修改都必须单独标记为“适配镜像”，不能与“原版镜像兼容”混在一起。

**当前理论上限（按 libc 分列）**：

- glibc：423/427（98.6%），扣 busybox 1 项 + libctest 3 项；
- musl：≤391/427（91.6%），再扣 basic 的 32 项；libctest 动态项还会更低。

如果团队决定修补 busybox 命令清单、libctest 脚本或 basic 的 libc 构建，必须新建适配镜像并单独报告；不能直接改原版镜像后声称原版通过。

## 6. 复现入口

```bash
# 1. 拉取官方 judge（固定 commit，不入库）
python scripts/fetch_judges.py

# 2. 用日志回放跑全部 11 组
python scripts/run_tests.py \
  --log path/to/serial.log \
  --out tests/results/2026-11-08/qemu/musl \
  --judge-dir third_party/oskernel-testsuits-cooperation/judge \
  --libc musl

# 3. runner 自身契约测试（空输入/缺组/超时/未知用例/judge 崩溃）
TEST001_JUDGE_DIR=third_party/oskernel-testsuits-cooperation/judge \
  python -m unittest discover -s scripts/tests -v
```

参考运行 harness 见 [scripts/ref/README.md](../../scripts/ref/README.md)；2026-11-08 的 11 组摘要见 [summary](../../tests/results/reference-2026-11-08/summary.md)。

## 7. 参考运行与尚未完成

- [x] 参考 Linux/QEMU 跑同一镜像，确认 judge 在真实输出上的行为（2026-11-08 完成，11 组均有真实结果；见 [reference-run.md](reference-run.md) 与 [summary](../../tests/results/reference-2026-11-08/summary.md)）。
- [ ] 真实 FOCOS QEMU 串口采集（P5；依赖 #2/#4/#5/#7/#22/#23/#24）。
- [ ] VF2 单独结果（P6.3；依赖板卡）。
- [ ] 与赛方/往届队伍确认平台 judge 是否就是本仓库的 5 个 stdin 脚本 + lmbench 运行器（公开仓库可能不是最终版本）。
- [ ] 决定是否制作“适配镜像”修补 busybox/libctest 的 4 个不可达项；若做，单独列结果。

参考运行发现：basic 的 musl 实际是 glibc 构建；busybox judge 多 `kill 10`；libctest 镜像缺 `crypt`/`pleval`；lmbench 的 `lat_proc fork` 在最小参考环境挂住；unixbench 的 `SHELL1/8/16` 无判分行。这些都不改变“FOCOS 结果另行采集”的边界。
