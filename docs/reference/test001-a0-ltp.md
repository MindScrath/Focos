# TEST-001 A0：LTP 目录与遍历输入

本批审计冻结镜像中的 LTP 目录和两份遍历脚本，生成可重复的候选表；承接 #50，仍不是完整 A0、可执行 PROFILE 或测试成绩。行为依据为 [TEST-001](../../specs/TEST-001.md) 和 [A0 门槛](test001-analysis-contract.md#a0先审计调用再宣布冻结)。

## 本批工具契约

`scripts/test001_ltp_audit.py` 默认校验仓库固定锁中的四份原样输入，再枚举两套 libc 的目录候选。`--inputs DIR` 只能替换为完全同字节、同集合的输入，不能自定义锁。`--verify-image IMAGE` 复用 #50 的限量私有复制和整盘 SHA256 校验，再核对两份脚本和两份 `debugfs ls -p` 输出；不执行镜像程序或 judge。目录输出按本次 debugfs 格式逐字节锁定，工具格式不同导致复核失败时不降级接受。

目录解析只支持本镜像已审计的普通文件和目录、ASCII 安全名称及十进制 inode/uid/gid/size、六位八进制 mode；缺少 `.`/`..`、重复名、非法行、符号链接或特殊文件直接拒绝，不能猜测 `test -f` 的跟随行为。`.`/`..` 与点号开头的名字不匹配默认 glob；目录不满足 `-f`，单列跳过原因。普通文件无论是否可执行都纳入候选，不能替脚本暗中加 `-x` 过滤。空目录保留零候选，不凭空生成一次调用。

候选表按 ASCII 文件名字节排序，仅供确定性审查；每条以 libc+basename 建 candidate_id，记录原始目录行、mode/inode/size、argv、cwd 和脚本调用位置。**没有冻结执行序号**，`expected_candidate_sequence=null`、`runtime_ready=false`；目录物理顺序不冒充 shell glob 顺序，目录排序也不冒充实际执行观察。候选文件大小和 inode 不是二进制版本/依赖审计的替代。

stdout 只在全部成功后一次输出完整 JSON；坏输入退出 2，debugfs/IO 执行失败退出 1，审计完成退出 0。整盘通过后成员读取不符即工具错误，包括 debugfs 返回 0 的空/短输出。CLI 沿用 #50 的错误边界，无报告文件发布协议、无验收 PASS。

测试门槛：冻结输入再生；不同 libc 差异；普通文件/目录/隐藏项；不可执行普通文件仍纳入；无序目录不产生执行序号；重复/坏行/链接/特殊文件拒绝；修改或额外输入拒绝；读取失败不输出 JSON；真实整盘独立复核。合成目录仅测试审计控制流，不作为实测成绩。

## 复现与来源

```bash
mkdir -p target
python3 -B scripts/test001_ltp_audit.py > target/ltp-audit.json
python3 -B scripts/test001_ltp_audit.py --verify-image /path/to/sdcard-rv.img > target/ltp-image-audit.json
python3 -B -m unittest discover -s scripts -p 'test_test001*_audit.py'
make check
```

镜像身份沿用 #50 已核对的 [pre-20250615/sdcard-rv.img.xz](https://github.com/oscomp/testsuits-for-oskernel/releases/download/pre-20250615/sdcard-rv.img.xz)：解压后 4294967296 字节、SHA256 `95973543db6b84a9a5e70f30da466ce292867aff5b689fb14c88dc9406e378b8`；本批再次核对整盘和四个输入。镜像版本不等于已知 LTP 源码构建版本。完整锁、提取操作及长度/哈希见 [ltp-audit-inputs.lock.json](../../tests/test001/ltp-audit-inputs.lock.json)，原样证据见 [ltp-audit-inputs](../../tests/test001/ltp-audit-inputs)。命令分别为 `debugfs -R 'cat /musl/ltp_testcode.sh' IMAGE` 和 `debugfs -R 'ls -p /musl/ltp/testcases/bin' IMAGE`，glibc 同理；stderr 版本横幅不混入 stdout 证据。复核需 debugfs 和约 4 GiB 临时空间。

候选表 [ltp-audit-candidates.jsonl](../../tests/test001/ltp-audit-candidates.jsonl) 由报告生成，测试逐字节检查再生：

```python
import json
from pathlib import Path
report = json.loads(Path('target/ltp-audit.json').read_text())
rows = (dict(group_libc=key, **row)
        for key, group in sorted(report['groups'].items()) for row in group['candidates'])
text = ''.join(json.dumps(row, sort_keys=True) + '\n' for row in rows)
assert text == Path('tests/test001/ltp-audit-candidates.jsonl').read_text()
```

本地 gh 核对固定 [cooperation 提交的 judge 目录](https://github.com/oscomp/oskernel-testsuits-cooperation/tree/44fa7ac65f292580c3974a8252f533c4484ebe6b/judge)，该目录没有 `judge_ltp.py`；不能编造一份官方 LTP judge 或移用旧分支判据。该提交的 [LTP 说明](https://github.com/oscomp/oskernel-testsuits-cooperation/blob/44fa7ac65f292580c3974a8252f533c4484ebe6b/doc/ltp/ltp.md) 是后续语法/构建来源审查入口，本批未将其转成判分实现。

## 枚举结果与决定

| libc | 普通文件候选 | `-f` 跳过的非隐藏子目录 | 默认 glob 不匹配 |
| --- | ---: | --- | --- |
| musl | 2820 | datafiles、ftrace_stress | `.`、`..` |
| glibc | 2840 | datafiles、ftrace_stress | `.`、`..` |

本镜像目录未见符号链接、特殊文件或其他隐藏项。数量由本轮目录证据重新得到，与旧候选数字相同不代表继承旧实现或执行有效性。`datafiles`/`ftrace_stress` 不作为顶层调用，不等于可以从运行包删除；它们可能是依赖，尚未递归审计。目录清单包含 shell 脚本、辅助程序和测试程序，不能把 5660 个文件称为 5660 个可独立验收用例。

musl 集合全部出现在 glibc 中，glibc 额外 20 项（报告同时保留逐项差异）：

```text
cpuset_base_ops_testset.sh  cpuset_cpu_hog
cpuset_exclusive_test.sh   cpuset_funcs.sh
cpuset_hierarchy_test.sh   cpuset_hotplug_test.sh
cpuset_inherit_testset.sh  cpuset_list_compute
cpuset_load_balance_test.sh cpuset_mem_hog
cpuset_memory_pressure    cpuset_memory_pressure_testset.sh
cpuset_memory_spread_testset.sh cpuset_memory_test
cpuset_memory_testset.sh  cpuset_regression_test.sh
cpuset_sched_domains_check cpuset_sched_domains_test.sh
cpuset_syscall_test       cpuset_syscall_testset.sh
```

决定：保留两套 libc 各自的集合，不取交集，不把差异解释成成功或豁免。需要逐文件核对可执行身份、构建来源和调用需求后才能发布 PROFILE。

## 遍历与输出语义

两份脚本以 `#!/bin/bash` 开头，在 `/{libc}` 工作目录中遍历 `"$target_dir"/*`，只判断 `-f`，随后直接执行 `"$file"`，不传附加参数。脚本没有设置 locale、glob 选项、PATH、超时或清理策略；脚本首行也不能证明实际启动方使用 bash。shell/locale、目录在运行中的变化、前项对后项的影响均待冻结，因此本批不推断最终调用顺序或实际启动次数。

| 脚本来源 | 候选语法 | 本批能确认 | 后续必须验证 |
| --- | --- | --- | --- |
| 第 3/24 行 | `#### OS COMP TEST GROUP START/END ltp-LIBC ####` | 两个组边界输出命令 | 真实日志、跨组/缺结束/嵌套反例 |
| 第 13 行 | `RUN LTP CASE NAME` | `basename` 结果作为开始名称 | 外部 basename/PATH、原生输出冒充边界、唯一调用身份 |
| 第 15–19 行 | `FAIL LTP CASE NAME : RET` | RET 来自前一 `"$file"` 的 `$?`；即使 0 也打印 FAIL | 按数值退出状态与原生 LTP 语义判定，不能按 FAIL 字面判失败 |

上述只有静态语法候选；工具不解析运行日志。脚本没有 `set -e`，某一调用非零仍继续；结束 echo 的状态不代表所有子项成功。外部 basename 的失败未被脚本单独记录。原生 TPASS/TFAIL/TBROK/TCONF/TWARN、汇总格式、退出位、无参数辅助程序的行为以及清理失败，仍需冻结源码与真实样本审计。

## 验证与剩余工作

新增测试先在未实现接口上运行失败，再实现目录解析与报告；验证项包含合成目录、冻结真实输入、CLI 错误传播和 debugfs 返回 0 但输出不符。原样脚本和目录输出保留来源空白，不为 `diff --check` 改写证据；代码与文档另作空白检查。实际命令和结果见 PR，本地详细日志置于 `target/a0-ltp-evidence/`，不上传整盘。

提交前按 AGENTS.md 检查来源、错误传播、限量复制、输入快照、README 路径和未完成边界。仍需一名非作者审阅。后续 A0 还包括本组与 BusyBox/Lua 的 ELF/脚本/数据依赖、运行环境、资源/超时/清理策略、真实语法样本及其他组；#8/#47/#48 不因本批关闭。
