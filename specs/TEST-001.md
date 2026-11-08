# TEST-001：串口日志驱动的测试执行与判分

关联：[Issue #8](https://github.com/MindScrath/Focos/issues/8) · 状态：草案（待审）

## 要做到什么

把 2025 初赛 Ext4 镜像在 QEMU 或真板上的串口输出，转换成逐组、逐子用例的可追溯结果：按组切分、调用官方或团队 judge、对缺失项补 0、标记超时/缺组/judge 错误，并输出 JSON 与 Markdown。

本次不包含：LTP、LoongArch、内核能力实现、修改被测脚本语义、伪造通过率。runner 只负责“测量与报告”，不负责让测试通过。

## 行为约定

- **输入**：一份完整串口日志，或一条会被执行并捕获 stdout 的命令（如 QEMU）。镜像、源码、judge 的版本来自 `tests/inventory.json`，runner 不自行推断版本。
- **组切分**：以 `#### OS COMP TEST GROUP START <name> ####` / `END <name> ####` 为界。`<name>` 若以 `-musl`/`-glibc` 结尾，去掉后缀得到组名，并把后缀记为 libc；没有后缀时使用 `--libc`。同一 `(组, libc)` 只保留一段。
- **判分**：官方组调用 `judge_<group>.py`，团队组调用 `scripts/judge_team/judge_<group>.py`；两者都用 stdin 传入组段文本，stdout 必须是 JSON 列表。runner 不改官方 judge 的判分规则。
- **状态**：
  - `PASS`：清单内全部子用例 `pass == all`；
  - `PARTIAL`：部分子用例通过或有分数；
  - `FAIL`：组段存在，但清单内没有任何通过项；
  - `TIMEOUT`：有 START 无 END，或捕获命令超时；
  - `NOT_RUN`：日志中没有该组；
  - `JUDGE_ERROR`：judge 缺失、超时、非零退出或输出不是合法 JSON。
- **缺失与额外**：judge 没返回的清单项记 `MISSING`，分数 0；judge 返回但清单没有的名字放进 `extras`，不计入分母；日志里出现但清单没有的 `========== START <name> ==========` 放进 `unexpected_in_log`，用于发现脚本/镜像漂移。
- **输出**：`summary.json`（元数据、逐组、逐子用例、totals）、`summary.md`（组级表）、`serial.log`（原始日志）、`sections/<group>-<libc>.log`（组段）。每个子用例至少含 `name`、`status`、`all`、`pass`、`score`。
- **退出码**：0 = 所有选中组 PASS；1 = 存在非 PASS；2 = 参数/清单错误。CI 只认退出码，不解析 Markdown。
- **单位**：不同组的 `score` 单位不同（分、lps、KBps、秒），汇总表不得跨组求和或比较。

## 怎样验收

| 场景 | 操作或测试入口 | 应看到什么 |
| --- | --- | --- |
| 正常样本 | `scripts/tests/test_runner.py` 的 basic 样例 | `test_brk`/`test_chdir` PASS，组 PARTIAL（其余缺失项 0） |
| 空输入 | 空日志跑全部组 | 全部 NOT_RUN，退出码 1，不崩溃 |
| 缺组 | 日志只有 basic，选 basic+unixbench | unixbench 全部 NOT_RUN，不补 PASS |
| START 无 END | lmbench 只给 START | 组 TIMEOUT，不把截断段落判成 PASS |
| 未知用例 | basic 段插入 `test_unknown` | `unexpected_in_log` 含 `test_unknown`，不影响清单分母 |
| judge 崩溃 | 用退出码 3 的假 judge | 组 JUDGE_ERROR，子用例全 JUDGE_ERROR，退出码 1 |
| 缺失项补 0 | 官方 judge 空输入 | 输出清单全部子用例，`pass=0`，不崩溃 |
| 性能组 | iozone 给一个正吞吐 | 该项 PASS；单位不同的分数不跨组比较 |
| 11 组烟测 | 合成日志跑全部组 | 427 个子用例、0 个 JUDGE_ERROR、11 组都有结果 |

测试先失败再实现；证据放 PR，不在本文件维护运行状态。

## 依据与依赖

- 输入版本：[test-inventory.md](../docs/reference/test-inventory.md)、[inventory.json](../tests/inventory.json)；
- 官方 judge：`oscomp/oskernel-testsuits-cooperation@44fa7ac`，用 `scripts/fetch_judges.py` 拉取，不入库；
- 真实执行依赖 [#2](https://github.com/MindScrath/Focos/issues/2)、[#4](https://github.com/MindScrath/Focos/issues/4)、[#5](https://github.com/MindScrath/Focos/issues/5)、[#7](https://github.com/MindScrath/Focos/issues/7)、[#22](https://github.com/MindScrath/Focos/issues/22)、[#23](https://github.com/MindScrath/Focos/issues/23)、[#24](https://github.com/MindScrath/Focos/issues/24)；
- 本改动不涉及 HAL/ABI/内存安全/持久化语义，普通 PR 一名非作者批准即可；若后续修改 CI 必过集合，需再一名成员确认。
