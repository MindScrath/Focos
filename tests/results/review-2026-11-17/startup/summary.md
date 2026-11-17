# PR29 Python 启动与 strict 参数补充修复（2026-11-17）

继续跟进 335f5d1 的 Codex 评审，发现两处有效遗漏。此前的 -B 只禁止写入 pyc，不能禁止读取，因此工作树已有缓存且源文件等长、时间戳仍有效时，源码哈希可能与实际执行模块不一致。另一方面，strict capture 已检查必填参数，strict replay 却到运行后才报告缺失，错误地退出 1 而非参数错误 2。

五个 CLI 入口在任何项目导入前建立每次运行私有的空 pycache，并禁止写缓存，退出时显式清理；不删除共享 __pycache__。所有 Python judge 子进程使用独立空缓存前缀和 -B，覆盖 common/inventory 及自定义 helper。Makefile 的两类 Python 测试、测试中的 judge 子进程和受控构建进程也隔离缓存；构建测试直接编译源文件，避免验证本身读到旧代码。

strict 的 manifest、kernel/initramfs 与 image 三组参数统一在读取日志、QEMU 版本探测、采集和判分前检查，任一缺失退出 2。原先期待缺参数却产生 ENV_ERROR 的测试已纠正为“参数齐全而协议错误”，不再固化错误分类。

新测试在修复前产生 18 个失败断言。先制造旧 pyc，再写入同长度新源文件并恢复原时间戳；同时覆盖未校验的 hash pyc。五个入口 --help 均必须触发新源码；四个 runner 依赖在真实成功判分时，其实际执行标记与 source_sha256 必须都属于新源码；自定义 judge helper 也不能执行旧缓存。strict 必填参数的 7 个非空缺失子集分别在采集/回放验证，均不得执行 judge 或 QEMU。

`make check mm-smoke irq-smoke` 通过 114 项专项、52 Rust、15 原有 Python、BOOT-001/MM-002/ARCH-001。15 份历史状态、逐项判分和数量不变，13 份完全一致、2 份仍只含已登记诊断增补。真实冻结输入新构建后，最终 QEMU 采集 Lua glibc 9/9；同日志回放 musl 9/9，镜像仍各完整哈希 2 次。

原始串口、manifest 和采集/回放摘要位于 lua/，哈希见 [validation.json](validation.json)。本轮跟进另留存这一次参考启动，旧 ownership/preflight/历史证据不覆盖，未重跑 22 次测量。常规支持入口为文档中的 CPython 文件 CLI；嵌入式 Python 调用须在首次项目导入前自行建立相同隔离条件。

Python 官方语义：[命令行 -B](https://docs.python.org/3/using/cmdline.html#cmdoption-B)、[sys.pycache_prefix](https://docs.python.org/3/library/sys.html#sys.pycache_prefix)。缓存隔离补齐了已复现的启动条件，不构成任意解释器环境或源码并发修改的密码学证明。

原始 serial.log 已转存外部固定哈希归档；本目录摘要与 validation 的原始哈希保留。下载位置、逐文件路径与校验值统一见 [原始证据索引](../../raw-evidence-2026-11-17.json)。库内不再保存含上游版权邮箱的日志，旧判分和原始字节不修改。
