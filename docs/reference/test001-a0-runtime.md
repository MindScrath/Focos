# TEST-001 A0：脚本启动路径与共享数据

本批将已有 BusyBox/Lua 输入、LTP 遍历脚本与真实基盘目录交叉核对，暴露解释器缺口及文件生命周期。依据 [TEST-001](../../specs/TEST-001.md) 的 A0；不生成可执行 PROFILE，不替未采集的运行条件作保证。

## 审计工具契约

`test001_runtime_audit.py` 使用仓库三份固定锁：#50 的 BusyBox/Lua 文本、#51 的 LTP 输入以及本批根目录与两套 libc 目录。先验证所有输入、三锁的镜像身份一致性以及脚本目录类型/长度，再从私有内存中的脚本首行提取 shebang；记录来源、实际首行和元数据。没有 shebang 的脚本必须显式报告，不按文件名假定解释器。

目录解析复用 #51 的严格普通文件/目录格式，不接纳符号链接或未知类型。本批路径查询区分 PRESENT_IN_BASE_IMAGE、ABSENT_FROM_BASE_IMAGE、NOT_AUDITED：已审计父目录缺少组件才证明基盘缺失；已知目录的内部没审计则未知，不能当作不存在。报告同时列出 BusyBox/Lua 的工作目录、相对程序以及四个写入目标的初始状态。

`--verify-image IMAGE` 复用同句柄限量私有复制、整盘 SHA256 和 debugfs 逐字节复核，验证本批三份目录；已有脚本文本由其既有锁核验，不新增脚本的本轮提取验证声明。默认 image_verification=NOT_RUN；坏输入退出 2，执行错误退出 1，成功计算后一次输出完整 JSON。固定 runtime_ready=false，不创建路径/链接、不设置环境、不执行脚本。只支持冻结脚本的简单绝对路径 shebang 加可选单个参数，不提供通用 shell 解析。

## 启动路径事实与后续决定

[原始目录](../../tests/test001/runtime-audit-inputs)、[身份锁](../../tests/test001/runtime-audit-inputs.lock.json) 和[可再生报告](../../tests/test001/runtime-audit-report.json)共同记录本次观察。目录取自与 #50 相同的固定 4 GiB 镜像；debugfs 1.47.3，`ls -p /`、`ls -p /musl`、`ls -p /glibc` 的原样 stdout。

| 两套 libc 均相同 | 首行 | 基盘事实 | 后续必须明确 |
| --- | --- | --- | --- |
| busybox_testcode.sh | `#!/busybox sh` | 根目录无 busybox | 提供此绝对路径，或冻结显式解释器启动协议 |
| test.sh（Lua 子调用） | `#!/bin/busybox sh` | 根目录无 bin | 即使外层显式启动，`./test.sh` 仍有独立解释器依赖 |
| lua_testcode.sh | 第一行是 `./busybox echo ...` | 无 shebang，mode=100755 | 不能只凭执行位假定 execve 能直接启动；不能依赖未冻结的 shell 回退 |
| ltp_testcode.sh | `#!/bin/bash` | 根目录无 bin | 需要真实解释器来源与身份；不能无验证地将 bash 路径指向另一 shell |

基盘根目录只有 lost+found、musl、glibc（另有 `.`/`..`）；因此三个绝对解释器路径的缺失有完整父目录证据。这不表示未来 initramfs/挂载视图也缺失它们，也不证明历史启动方如何补齐。报告特意使用 FROM_BASE_IMAGE 限定结论。

后续运行配置必须同时冻结外层启动 argv、子脚本解释器提供方式、各路径对应 ELF 身份和 cwd。不能只把外层改成 `./busybox sh SCRIPT` 就宣布 Lua 子调用依赖已解决。BusyBox 脚本使用 `[[ ... ]]`，Lua 包装器使用 `[ $? == 0 ]`；本批没有用主机 `/bin/sh` 代替来宾 shell 证明兼容。

## PATH 与环境审查清单

脚本没有设置 PATH；目录里没有独立的 `sh`、`ash`、`sleep`、`basename` 或 `ls` 同名入口。BusyBox 表行通常经 `./busybox APPLET` 直接调度，但不能由此推出所有子命令都绕过 PATH：

- 表第 15 行 `which ls` 会观察命令查找环境；它查到什么必须由运行配置说明。
- 第 23 行 `sh -c 'sleep 5' & ./busybox kill $!` 中，外层经过 BusyBox 调度，子 shell 的 `sleep` 仍可能依赖 applet 编译选项或 PATH。`$!`、后台子进程及终止/回收必须独立验证，不能只检查最后一个 kill 返回值。
- LTP 脚本用未限定路径的 `basename`。目录里不存在同名文件不证明运行一定失败，因为 shell/applet 提供方式和最终 PATH 尚未冻结。

后续 PROFILE 不继承任意主机/父进程环境，应明确 PATH、locale、TZ、TERM、umask、uid/gid、标准输入和 cwd；具体值须结合真实来宾样本审查。本批仅列需求，不把建议值伪装为实际运行条件。

BusyBox 的 date/cal/hwclock 涉及时间，ps/free/uptime/df/dmesg 等涉及系统信息，clear/more 涉及终端或输入。它们的具体 `/proc`、设备与权限依赖要由锁定构建与真实样本确认，不按命令名称猜成已完成的设备清单。资源/超时数字本批不编造。

## 文件生命周期与隔离要求

以下按已冻结源行审查，musl/glibc 相同；是静态行为及后续验证要求，不是已执行轨迹。表行号来自 [busybox_cmd.txt](../../tests/test001/audit-inputs/musl/busybox_cmd.txt)。

| 对象 | 源码行为 | 不能丢失的依赖/验证点 |
| --- | --- | --- |
| busybox_cmd.txt | 作为 while/read 输入；第 52/53/55 行再读、复制、搜索 | 不修改原表来迁就判据；结束后仍须核对只读身份 |
| test.txt（BusyBox） | 第 27 行创建，28 行截断写；29–47 行读取/追加/管道；48 行删除 | 顺序和两次相同追加都保留，不能将每行当成无状态独立调用 |
| test_dir → test | 第 49 行 mkdir，50 行 rename，51 行 rmdir | 两个名字都须有基盘初始状态和结束后检查，不能只看最终 rmdir 输出 |
| busybox_cmd.bak | 第 53 行复制，54 行删除 | 前项失败后残留也须记录；清理失败不能被后续成功覆盖 |
| test.txt（Lua） | file_io.lua 第 2 行以 w 打开，remove.lua 第 1 行以 r 打开再删除 | 后者依赖前者，不能按参数随意重排；与 BusyBox 共用相对文件名，组隔离/新副本必须明确 |

三份目录证据确认两套 libc 中上述四个写入路径初始均不存在。但当前目录仍含很多其他测试文件，不能把它等同于空工作目录；第 11/24/55 行的 du/ls/find 会观察这一布局。未来采集使用新可变副本，保留预期初始布局；失败残留和清理结果作为证据记录，不在同一槽位自动重试，也不通过删除输入“修复”结果。

Lua 源码的 `return -1` 不是进程退出码；test.sh 只检查 `./lua` 的进程状态。文件读写、数学/字符串断言是否被包装器充分表达需要判据审计，不能因为打印 success 就认定全部内部断言成立。现有原始输入不在本批改写。

## 复现与剩余门槛

```bash
mkdir -p target
python3 -B scripts/test001_runtime_audit.py > target/runtime-audit.json
cmp target/runtime-audit.json tests/test001/runtime-audit-report.json
python3 -B scripts/test001_runtime_audit.py --verify-image /path/to/sdcard-rv.img > target/runtime-image-audit.json
python3 -B -m unittest discover -s scripts -p 'test_test001_runtime_audit.py'
make check
```

默认报告可逐字节再生；加镜像验证后报告增加本次验证状态，不能与默认报告直接 cmp。三份目录的原样末尾空行是 debugfs 输出的一部分，保留并核对哈希；代码/文档另作空白检查。新测试先在未实现接口失败，再覆盖缺解释器/无 shebang、未审计与缺失的区别、非法路径、初始写入目标、混用镜像锁、报告再生及 CLI 错误传播。

本批不启动来宾、不新增链接、不改变环境或原始脚本。A0 下一步需冻结提供解释器和 applet 的实际配置，并取得真实语法/前置/清理样本；LTP 的逐文件依赖与其余测试组仍未完成。#8 保持开放，需一名非作者审阅。
