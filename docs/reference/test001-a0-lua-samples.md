# TEST-001 A0：真实 Lua 语法样本

本批用比赛镜像已锁定的两套 BusyBox/Lua 和原样脚本，在参考 Linux/QEMU 中取得真实输出，供 A1 单组解析使用。样本不是 TEST-001 C 阶段运行包，trust 固定为 external_log/unknown；没有 FocOS/VF2 成绩，也不能据此发布完整 PROFILE。

## 做了什么

[样本目录](../../tests/test001/fixtures/lua-a0) 包含实验 init、复现归档配方、两份未改写的串口片段和 [index.json](../../tests/test001/fixtures/lua-a0/index.json)。每套 libc 单独启动，原始组脚本执行一次；随后在组外做三个语义探针。最终提交的两个样本来自相同最终配方。此前开发过程的探针日志保留在本地 target/lua-samples/initial-probe，不把试验迭代称为正式 campaign 重试或验收。

运行根文件系统是最小 initramfs，不挂载比赛整盘。`/{libc}` 内只放原样九份 Lua、test.sh、lua_testcode.sh 及本 libc 的 busybox/lua；逐成员 mode、内容哈希和设备号均在索引中。`/bin/busybox` 链接到本 libc BusyBox，`/bin/sh` 链接到 busybox；外层明确执行 `./busybox sh ./lua_testcode.sh`。由此满足 #53 发现的 Lua 子脚本绝对解释器路径；没有给 BusyBox/LTP 组补齐启动环境的声明。

init 设置 PATH=/bin、LC_ALL=C、TZ=UTC、TERM=dumb、umask=022，uid/gid=0；内核通过 LIBC 参数选择工作目录。QEMU 使用 virt、单核、256 MiB、无网卡、显式 OpenSBI、60 秒主机超时；RTC 使用默认主机墙钟，未冻结时间。未挂载 proc/sys，也没有 Bash 或独立 applet 文件集合。这是本次 Lua 样本环境，不是所有组可复用的环境保证。

## 实测观察与判据边界

| 实际操作 | musl / glibc 观察 | A1 必须保留的含义 |
| --- | --- | --- |
| 原样 lua_testcode.sh | 九条 success，顺序与输入相同；组 START/END 完整 | 原包装协议观察，不能扩大为所有 Lua 内部断言通过 |
| 组脚本结束 | shell 状态 0；test.txt 不存在 | 组退出、文件观察和逐调用结果分开；不能替代逐条判定 |
| `./lua -e 'return -1'` | 退出 0 | Lua 返回值不是进程退出码 |
| `./lua -e 'error("A0_EXPECTED_ERROR")'` | 错误与 traceback，退出 1 | 错误原文保留，不从单词 error 猜测归属或覆盖结果 |
| `./test.sh __a0_missing.lua` | 原包装器打印 fail，但包装器最终退出 0 | 它检查 Lua 进程后执行 echo；包装器自身退出 0 不能推导用例成功 |

不存在的 `__a0_missing.lua` 仅用于组外故障注入，不加入比赛调用清单。原九份 Lua 和包装器未改写；这份失败输出不能伪装成正式日期用例失败或完整组负例。

串口片段保留原生 CRLF、制表符及错误文本。索引记录完整 stdout 的长度/哈希、片段半开字节区间 `[begin,end)`、片段中的组区间；只截取 A0_SAMPLE_BEGIN 到 A0_SAMPLE_END，不逐行修补。完整启动日志本地保留，不提交其中的构建信息。仅凭索引不能独立恢复未提交的完整 stdout，完整原文核查需要本地日志；片段本身的字节与来源输入可在仓库检查。

## 进入 A1 的 Lua 语法表

| 片段/上下文 | 必需字段与次数 | 待实现的反例 |
| --- | --- | --- |
| `#### OS COMP TEST GROUP START lua-LIBC ####` | 所选 libc 的唯一组开始；单行完整匹配 | 错 libc、重复/嵌套开始、组外结果 |
| `testcase lua NAME.lua success\|fail` | 九个已选择参数的每次结果，原顺序；结果字段属于同一记录 | 缺项、重复、失败后成功、未知/范围外参数、调换顺序、消息引用伪造结果 |
| `#### OS COMP TEST GROUP END lua-LIBC ####` | 与开始对应的一次结束 | 缺结束、错组、结束后新增结果 |
| A0_*、Lua 错误与 traceback | 本实验组外探针/诊断，原字节位置保留 | 不能整体吞成已消费组内容，不能为当前 selection 补调用 |

A1 可用索引中的组区间作为独立正例；若输入整个片段，组外缺失参数探针不能被悄悄纳入或忽略。字节→记录→归属→规范结果必须唯一，不能把本批测试中对静态样本的精确断言复制为另一套判分器。所有外部样本验收仍 BLOCKED。

## 复现

先按 [ELF 审计配方](test001-a0-elf.md#复现) 从固定镜像提取四份二进制，置于一个目录，命名 musl-busybox、musl-lua、glibc-busybox、glibc-lua。以下配方会按已有锁限量复制并核对全部四份，再生成归档：

```bash
python3 -B tests/test001/fixtures/lua-a0/pack.py /path/to/binaries target/lua-reproduction-new
```

输出目录必须尚不存在。pack.py 是本次实验配方：只打包 initramfs 与成员清单，不调用 QEMU，不实现 build/capture、完成清单、缓存或运行包发布协议。newc 的 uid/gid/mtime 固定为 0；/dev/console 为 5:1，/dev/null 为 1:3。输入错误在创建输出目录前失败；写入中失败可能留下不完整实验目录，不能作为正式构建包使用。配方及 init 字节由索引锁定，变化后应另留样本记录。

参考内核取固定 Alpine APK（来源与 SHA256 见索引），先核验 APK，再通过 Python tarfile 读取单个 `boot/vmlinuz-lts` 成员，以 gzip 解压为 Image；不执行 APK 内容，也不解包任意路径。解压 Image 必须匹配索引哈希。OpenSBI 用本机已记录版本对应文件，运行前比对索引哈希；换内核/固件/QEMU 时记录新身份，不能冒充同一环境。

```bash
# musl；glibc 使用另一份归档，并将 LIBC 改为 glibc
 timeout 60 qemu-system-riscv64 -machine virt -m 256M -smp 1 \
  -nographic -no-reboot -nic none -bios /path/to/firmware.bin \
  -kernel /path/to/Image -initrd target/lua-reproduction-new/musl.cpio \
  -append 'console=ttyS0 rdinit=/init panic=-1 LIBC=musl' \
  </dev/null >target/musl-full-serial.log 2>target/musl-qemu-stderr.log
```

完整 argv 模板、工具/构件身份和 QEMU 退出码见索引。clock/启动耗时会变化，不要求重跑完整串口与旧哈希相同；本次两份 initramfs 则已按提交配方重建并逐字节匹配，GNU cpio 独立列目录成功，再用重建归档实际启动并保存最终样本。

## 验证与下一步

样本一致性测试先因缺失样本/索引失败，补入真实证据后检查完整成员集合、输入锁对应、片段哈希/区间、固定包装器观察和九项顺序。它们保护样本，不是 A1 解析实现。实际执行日志在 target/lua-samples/；工程回归和 Python 版本检查见 PR。

本批为 A1 Lua 单组纯解析提供真实正例及失败语法。下一步按已合并解析契约先写反例，再实现记录归属与规范判分；BusyBox 的团队标记、LTP 原生语法和其他组 A0 门槛仍需分别完成。需一名非作者审阅，不关闭 #8/#47/#48。
