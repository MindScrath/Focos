# TEST-001 A0：BusyBox/Lua 调用输入审计

本次交付可重复的只读审计工具和 128 条候选记录，供审查真实调用、顺序和 judge 差异。对应 [TEST-001](../../specs/TEST-001.md) 的 A0 第一批；**不是可执行 PROFILE，不代表 A0/A 完成，也没有测例通过成绩**。其余组（含 LTP）、环境及资源策略仍待审计。

## 复现

仓库根目录执行（仅 Python 标准库；整盘复核另需 debugfs 和约 4 GiB 临时空间）：

```bash
python3 -B scripts/test001_audit.py > target/a0-report.json
python3 -B scripts/test001_audit.py --verify-image /path/to/sdcard-rv.img > target/a0-image-report.json
python3 -B -m unittest discover -s scripts -p test_test001_audit.py
make check
```

先创建 `target/`。默认只核对仓库 28 个原样文本输入，image_verification=NOT_RUN；加 `--verify-image` 后以非阻塞方式打开源文件，从同一句柄的 fstat 拒绝非普通文件或异长输入，再私有复制整盘。复制最多写入冻结大小，另读一字节检查增长，提前 EOF 拒绝为输入错误；不按原路径重开。随后以固定 1 MiB 块核对大小/SHA256（不依赖 Python 3.11 的 file_digest），再用只读 `debugfs -R 'cat /固定路径'` 核对 26 份文本与 4 个 ELF 的长度及哈希。任何成员错误都不输出 JSON，不因 debugfs 返回 0 就视为成功。工具不执行 shell 命令表、Lua、judge 或内核。只读审计不使用 build/capture/analyze 的运行包格式；stdout 是完整计算结束后的候选 JSON，没有验收 PASS 字段。输入集合或整盘身份不符退出 2；整盘校验通过后，成员短读/哈希不符属于 debugfs 执行失败，即使退出 0 也退出 1，并保留成员名和 stderr。其他工具执行失败同样退出 1，完成审计退出 0（不表示运行就绪）。

`--inputs DIR` 只能提供与仓库锁完全相同的文本集合和字节，不能覆盖锁、注入 judge 或扩大组范围；拒绝缺失、额外、符号链接与特殊文件。全部文本校验后在内存中消费，不在审计后重新读取原路径。信任仓库源码/锁和主机 Python/debugfs，不提供第三方配置或对抗同账户进程的承诺。

输入锁：[audit-inputs.lock.json](../../tests/test001/audit-inputs.lock.json)。逐行表：[audit-calls.jsonl](../../tests/test001/audit-calls.jsonl)。表中 ID 是绑定输入行的 candidate_id，尚未成为运行协议的 invocation_id。每条记录保留 libc、工作目录、文件与行号、原始文本、展开后命令及顺序；不把 judge 去重键当作调用。

逐行表可从工具输出重建（与完整 JSON 的 groups 按键排序一致）：

```python
import json
from pathlib import Path
report = json.loads(Path('target/a0-report.json').read_text())
rows = [dict(group_libc=key, **call)
        for key, group in sorted(report['groups'].items()) for call in group['calls']]
text = ''.join(json.dumps(row, ensure_ascii=False, sort_keys=True) + '\n' for row in rows)
assert text == Path('tests/test001/audit-calls.jsonl').read_text()
```

## 来源和实际差异

镜像为 [pre-20250615/sdcard-rv.img.xz](https://github.com/oscomp/testsuits-for-oskernel/releases/download/pre-20250615/sdcard-rv.img.xz)，解压后 4294967296 字节，SHA256 `95973543db6b84a9a5e70f30da466ce292867aff5b689fb14c88dc9406e378b8`。本次只验证解压镜像，不继承旧分支的压缩包验证或全组审计声明。

judge 原样取自 [oskernel-testsuits-cooperation 的固定提交](https://github.com/oscomp/oskernel-testsuits-cooperation/tree/44fa7ac65f292580c3974a8252f533c4484ebe6b/judge)，用本地 gh 下载；锁记录逐文件 SHA256。它们是来源证据，只通过 AST 读取字符串常量 cmds，不 import/exec，不用其输出判分。镜像内文本作为原始样本保存；第三方内容保留原样，来源与固定版本见锁，未声明由本项目原创或重新许可。

| 每套 libc | 镜像调用次数 | 原始命令唯一数 | judge 基线行数 / 唯一键数 | 差异 |
| --- | --- | --- | --- | --- |
| BusyBox | 55 | 54 | 54 / 53 | 镜像多 printf、后台 kill；judge 多 kill 10 |
| Lua | 9 | 9 | 9 / 9 | 命令集合和顺序相符 |

BusyBox 镜像多出的原始行为是 `printf "abc\n"` 和 `sh -c 'sleep 5' & ./busybox kill $!`；judge 中 `kill 10` 无对应镜像调用，不纳入候选分母。`echo "bbbbbbb" >> test.txt` 出现两次，分别保留行号和独立 ID，不放宽重复次数，也不折成一个键。差异按原始命令（去掉默认 IFS 首尾空白）比较，实际 eval 文本另列；这是集合/次数审计，不是历史日志结果对照。

决定：后续 PROFILE 按实际 55 次表行调用建立身份；旧 judge 的 53 键仅作对照，不能修改镜像以迁就 judge。#48 的清单差异已有事实与处理方向，但 PROFILE/CRITERIA、历史对照尚未实现，因此保持开放。

## 运行语义与待冻结项

- BusyBox 先在 `/{libc}` 通过 `./busybox cat ./busybox_cmd.txt | while read line` 读取，再 `eval "./busybox $line"`。没有 `read -r`：默认 IFS 下首尾 SP/HT 去掉，反斜杠转义先被消费，例如 printf 原始行变成 `printf "abcn"`。工具保留 raw 与 command/eval_source；没有用 shlex 编造 argv，因为 `&`、管道、重定向、`$!` 都属于 shell 语义。辅助解析器只接受本锁使用的单物理行语法，续行/转义 IFS 空白/空记录/控制字符直接拒绝。
- 调用计数是 55 次 **eval 表行**，不是子进程数。后台命令与管道会启动多个进程；启动/结束标记必须包围整个表行。shell/IFS、权限、时区、环境继承与可执行依赖仍需运行配置审查。
- BusyBox 把退出非零且命令不为 `false` 记 fail；`false` 非零是原脚本明确接受的行为。后续规范判据须保留这一例外，不能统一要求所有内部程序退出 0。`test.txt` 的连续写入/排序/删除、`test_dir`→`test` 的重命名和后台进程依赖前文；不能把表行单独无状态重跑。整个组脚本退出与单行结果是不同事实。
- Lua 顺序调用 `./test.sh NAME.lua`，包装脚本执行 `./lua $1` 并按其进程退出状态打印结果。九份 Lua 输入及包装脚本均锁定；`file_io.lua` 创建 test.txt，`remove.lua` 后续读取并删除，必须保留冻结顺序。Lua 源码的 `return -1` 不能直接当作宿主进程非零退出，判据以包装器实际协议为准。
- 四个 busybox/lua ELF 的镜像字节身份已核对，但 ELF 动态库/解释器依赖尚未展开；绝不把 ELF 哈希等同于完整运行环境。原脚本的 shell 路径、前置条件、清理失败处理、CPU/内存/超时和额外数据依赖是下一批 A0 输入。

## A1 前必须审查的语法表

以下是从冻结脚本得到的语法候选，仍待真实参考日志与新调用标记验证；本次不实现日志解析器。

| 来源/格式 | 身份与次数 | 判据/边界 | 必须拒绝的反例 |
| --- | --- | --- | --- |
| `*_testcode.sh` 的 `#### OS COMP TEST GROUP START/END GROUP-LIBC ####` | 对应一个选中组/libc，一对边界 | 匹配组、libc、完整结束；保留原行 | 嵌套、错组、错 libc、缺结束 |
| `test.sh` 的 `testcase lua NAME.lua success\|fail` | 只能映射到九个已选择参数，各一次，顺序按脚本 | success/fail 是结果字段；整条匹配，LF/CRLF 按解析契约处理 | 未知参数、重复、跨 LF、组外结果、消息引用产生假结果 |
| BusyBox 的 `testcase busybox COMMAND success\|fail` | COMMAND 是 read 后文本；原日志对重复命令没有独立 ID | 作为原始观察保留，不能按出现顺序把两次 bbbbbbb 猜为计划 ID | 两次同名导致 AMBIGUOUS_INVOCATION；失败后成功不得覆盖 |
| 新 BusyBox 调用边界（待设计） | 每条 eval 的独立 ID/开始/结束/退出与冻结顺序 | 必须新 PROFILE revision，旧日志不补造 ID | 跳过、重试、范围外、重排、清理失败仍继续 |

因此本批所有 BusyBox 条目标记 NEEDS_RUNTIME_MARKER；Lua 的 UNIQUE_ARGUMENT 仅表示参数可区分，不等于执行有效。后续先完成真实日志/环境/标记审计，才发布对应可执行配置。保留旧参考日志的结果不代表必须继承旧 judge 的错误。

## 本轮验证与自审

8 个首批失败测试在 `1b75d11` 的未实现接口上失败；实现后增加坏镜像启动前拒绝、CLI 无部分 JSON 和逐行表再生核对三项。测试验证重复/顺序、read 转义、与主机 `/bin/sh` 的只读 read 对照、AST 不执行 judge、已知差异、额外/变更/链接输入和确定性输出。主机 shell 测试不冒充镜像 BusyBox 或 QEMU 执行。 后续补充缺少 file_digest API、debugfs 零/非零退出及版本查询失败的回归，以及源文件类型、复制增长/缩短、路径替换与 FIFO 拒绝回归，共 18 项审计测试；工具读取失败不能被误归为输入错误。

本地实际运行与提交对应的日志在 `target/a0-evidence/`，不提交大日志或镜像。提交前核对：新增路径已入 README；候选数据不写成已冻结；工具错误不生成成功 JSON；原始字段和命令执行语义分开；未做的运行条件审计明确列为待办。审计实现需一名非作者批准，#8/#47/#48 均不因本批关闭。

原样输入包含镜像本身的行尾空格/空行，整仓 `git diff --check` 会报告这些来源字节；不修改证据来消除提示。代码与文档单独做空白检查，原样输入逐字节核对锁和镜像。
