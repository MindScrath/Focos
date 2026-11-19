# TEST-001 A0：BusyBox/Lua ELF 链接证据

承接 #50/#51，检查四个已锁定 ELF 的程序头与动态段，保留可重复的 readelf 原始输出。依据 [TEST-001](../../specs/TEST-001.md) 的 A0 依赖审计；本批不完成脚本、数据、环境依赖闭合，不发布 PROFILE 或执行成绩。

## 工具契约

`python3 -B scripts/test001_elf_audit.py --binaries DIR` 读取 DIR 下 `musl-busybox`、`musl-lua`、`glibc-busybox`、`glibc-lua`。文件名及长度/SHA256 来自仓库已有 [输入锁](../../tests/test001/audit-inputs.lock.json)，不接受自定义锁。其他文件不参与审计。

先复用 #50 的非阻塞打开、fstat 普通文件检查、同一句柄限量复制，将每个文件复制到临时私有目录并校验长度/哈希；四份全部通过后才运行 readelf。工具只消费这些私有副本，不执行来宾程序，不调用 ldd；不会按可变原路径再次打开。readelf 使用主机 PATH 中的工具，LC_ALL=C，单次超时 30 秒，保留版本和 `-h -l -d -W` 完整 stdout。非零退出、超时、启动失败或 stderr 诊断均视为工具失败，避免把部分输出当作有效证据。

stdout 在所有检查成功后一次输出 JSON。输入不匹配退出 2，IO/工具失败退出 1，完成退出 0。报告固定 runtime_ready=false、image_verification=NOT_RUN：只证明消费的四个 ELF 与锁匹配，不证明源目录来自哪次整盘提取，也不证明运行依赖齐全。readelf 输出由人审查，本工具不新增 ELF 解析器或自动判分。

## 已观察到的结果

[完整原始证据](../../tests/test001/elf-audit-evidence.json) 由 GNU readelf 2.46.1-1.fc44 生成。四个文件均为 ELF64、小端、RISC-V、ET_EXEC；程序头无 PT_INTERP/PT_DYNAMIC，readelf 明确报告没有动态段。本批据此确认四个文件没有通过这两类程序头声明启动解释器或 DT_NEEDED 链接依赖；不把“没有动态段”扩大为“无需任何运行资源”。

| 文件 | 字节数 | ELF flags | 程序头中的 TLS |
| --- | ---: | --- | --- |
| musl/busybox | 1387560 | 0x0 | 无 |
| musl/lua | 476144 | 0x0 | 无 |
| glibc/busybox | 1745480 | 0x5，RVC / double-float ABI | 有 |
| glibc/lua | 880992 | 0x5，RVC / double-float ABI | 有 |

这些是二进制声明，不是内核已经支持 TLS、浮点或加载执行的成绩。musl 的 flags=0 也不证明程序不会使用浮点指令。没有 DT_NEEDED 不能排除代码在运行时加载模块或读取配置；本批没有做此类调用路径分析。

原有组脚本的 shebang、`sh`/`bash` 查找、PATH、BusyBox applet 配置、Lua 模块路径、时区以及 `test.txt` 等文件生命周期仍需冻结。LTP 的 5660 个目录候选不在本次四文件检查范围内。

## 复现

先从固定镜像提取四份二进制；下面脚本只运行主机 debugfs，随后审计工具再次按 #50 的锁校验实际消费副本。镜像路径按本机修改，二进制不入库。

```bash
mkdir -p target/elf-inputs
python3 - <<'PY'
from pathlib import Path
import json
import subprocess
lock = json.loads(Path('tests/test001/audit-inputs.lock.json').read_text())
image = '/path/to/sdcard-rv.img'
for name, meta in lock['executables'].items():
    result = subprocess.run(['debugfs', '-R', 'cat ' + meta['path'], image],
                            capture_output=True, check=True, timeout=30)
    Path('target/elf-inputs', name.replace('/', '-')).write_bytes(result.stdout)
PY
python3 -B scripts/test001_elf_audit.py --binaries target/elf-inputs > target/elf-report.json
python3 -B -m unittest discover -s scripts -p 'test_test001_elf_audit.py'
make check
```

提取命令本身不是校验：debugfs 即使返回 0，也可能没有正确读到文件，必须由审计工具验证长度及哈希。同一 readelf 版本/locale 下可直接比较报告；换工具版本保留新报告和版本供审查，不覆盖现有证据冒充原始结果。需要整盘验证时另运行 `test001_audit.py --verify-image IMAGE`；本批没有重做整盘验证，不继承为新工具的能力。

## 验证与自审边界

先写三个测试并在未实现接口上确认失败，实现后补齐全部输入在启动工具前校验、CLI 无部分 JSON、非 UTF-8/空工具输出等反例。合成 `elf` 三字节样本仅用于控制流故障注入；上述四个真实 ELF 的证据单独生成。文件类型、大小变化和路径替换沿用 #50 的已有回归覆盖。

按 AGENTS.md 自审：没有执行未信任的来宾程序；冻结身份绑定实际私有副本；工具诊断不被静默吞掉；原始输出供人工审阅，不生成运行合格结论；README 路径与 AI 记录同步。测试/构建结果见 PR；仍需一名非作者批准。
