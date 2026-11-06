# 第一次参与 FocOS

目标是先通过开发机测试和 QEMU 启动检查，再提交一个队友能审完的小改动。第一天不需要接板子，也不需要实现一个完整模块。

## 1. 准备工具

建议先用 Linux；Windows 成员可以用 WSL2。需要 Git、rustup、GNU make、sed、Python 3 和 `qemu-system-riscv64`。开发机（host）测试会生成本机可执行文件，还需要本机 C 链接工具链（提供 `cc` 等工具）。

Ubuntu / Debian 的系统工具可这样安装（其他发行版使用对应包管理器）：

```sh
sudo apt-get update
sudo apt-get install -y git build-essential sed python3 qemu-system-misc
```

Rust 请先按 [官方 rustup 安装说明](https://www.rust-lang.org/tools/install) 安装 rustup。项目固定 Rust 1.99.0，进入仓库后 rustup 会按 `rust-toolchain.toml` 获取工具链、组件和目标。

## 2. 获取代码

在 GitHub 打开 [上游仓库](https://github.com/MindScrath/Focos)，点击 Fork，创建自己的公有 fork。然后运行下面命令，先把 `YOUR_GITHUB_LOGIN` 换成自己的用户名：

```sh
git clone https://github.com/YOUR_GITHUB_LOGIN/Focos.git
cd Focos
git remote add upstream https://github.com/MindScrath/Focos.git
git fetch upstream
git switch -c feat/1-memory-disk upstream/main
make check
```

这里的分支名只是 #1 的例子；认领其他任务时换成对应编号和主题。`origin` 是你的 fork，`upstream` 是团队仓库。

## 3. 判断是否跑通

在仓库根目录运行普通的 `make check`，会依次执行以下检查。任一步失败，命令返回非零状态，后续步骤不会继续。

| 阶段 | 检查内容 | 预期结果 |
| --- | --- | --- |
| `fmt` | `cargo fmt --all -- --check` | 没有格式差异，正常退出 |
| `lint` | RISC-V release clippy，以及 host 通用库 clippy | 两次检查都正常退出；警告也按失败处理 |
| `test` | 在开发机运行 `focos-kernel` 的库单元测试 | 出现 `test result: ok`，所有测试通过且没有忽略项；当前为 10 个 HAL-001 测试 |
| `smoke`（先 `build`） | 构建 RISC-V 内核，再用 QEMU 检查启动、输出与关机 | 出现 `PASS: BOOT-001` |

完整成功需要本次 host 测试通过、QEMU 检查通过，且 `make check` 正常退出。单独出现 `PASS: BOOT-001` 只说明启动检查成功；测试数量可随新增用例增加，不以固定数量判断成功。

Makefile 从 `rustc -vV` 的 `host:` 字段自动获取开发机目标（`HOST_TARGET`）。`make test` 使用该目标运行通用库，`make build` 使用 `riscv64gc-unknown-none-elf` 构建裸机内核；开发机测试不需要 QEMU。

| 需要什么 | 命令或路径 |
| --- | --- |
| 只检查格式 | `make fmt` |
| 只执行 RISC-V 与 host clippy | `make lint` |
| 只运行开发机单元测试 | `make test` |
| 只编译 | `make build` |
| 编译并跑启动检查 | `make smoke` |
| host 测试结果与断言失败详情 | `make test` 的终端输出；不会写入启动日志 |
| 串口与 QEMU 版本记录 | `target/evidence/boot-qemu-rv.log` |
| 内核 ELF | `target/riscv64gc-unknown-none-elf/release/focos-kernel` |

先根据失败阶段排查：

| 现象 | 下一步 |
| --- | --- |
| Rust 工具链下载失败或缺少组件/目标 | 检查网络、`rustup show` 和仓库的 `rust-toolchain.toml`；按该文件安装固定工具链、rustfmt、clippy 与 RISC-V 目标后重试 |
| `fmt` 报告格式差异 | 运行 `cargo fmt --all`，查看 `git diff`，再运行 `make fmt` |
| `lint` 失败 | 查看输出中失败的是 RISC-V 还是 host 命令，按文件和行号修复；用 `make lint` 复查，不移除 `-D warnings` |
| host 测试提示找不到 `cc` 或链接失败 | 检查 `cc --version`；安装本机编译/链接工具链，Ubuntu / Debian 对应上面的 `build-essential`。保留链接器的原始错误再定位 |
| host 目标为空或不匹配 | 检查 `rustc -vV` 的 `host:` 字段、sed 是否可用，以及是否误传了 `HOST_TARGET`；恢复本机目标后运行 `make test` |
| host 测试出现 `FAILED` 或 panic | 运行 `make test`，保留失败用例名、断言的实际值与期望值；对照 [HAL-001](../specs/HAL-001.md) 和 `kernel/src/block/tests.rs` 定位，可按下方命令单独复跑 |
| RISC-V 构建失败 | 运行 `make build`，检查第一条编译/链接错误；找不到目标库时检查 RISC-V target 是否已安装，链接脚本问题检查 `.cargo/config.toml` 和 `kernel/src/arch/riscv64/linker.ld` |
| 找不到 QEMU | 运行 `qemu-system-riscv64 --version`，确认工具已安装且在 PATH 中 |
| QEMU 启动超时或报告 `boot contract violated` | 运行 `make smoke`，查看本次 `target/evidence/boot-qemu-rv.log`；脚本要求 20 秒内正常退出、恰好一个 `FOCOS_BOOT_OK` 且没有 `FOCOS_PANIC` |

单独复跑一个 host 测试，例如同步失败用例（其他用例替换完整测试名）：

```sh
focos_host_target=$(rustc -vV | sed -n 's/^host: //p')
cargo test --locked -p focos-kernel --lib --target "$focos_host_target" \
  block::tests::failed_sync_reaches_caller_and_can_be_retried -- --exact --nocapture
```

该命令应显示恰好 1 个测试运行并通过；若显示 0 个，检查测试名，不能算作验证成功。修复后重新运行 `make check`。向队友求助时提供 `git rev-parse HEAD`、本地改动说明、完整命令和本次错误输出；启动失败再附启动日志。前面的检查失败时，QEMU 尚未运行，已有启动日志可能来自上一次，不能用来证明本次通过。不要通过删除断言或忽略失败用例让检查变绿。

当前基线验证 HAL-001 的内存盘行为、测试设备注入的读写/同步错误，以及 QEMU 的启动和关机。host 测试不证明真实 SD 驱动或断电持久化；启动检查也不证明真板、UART 驱动或文件系统已经可用。

## 4. 找到要改的位置

| 路径 | 用途 |
| --- | --- |
| `kernel/src/main.rs` | 内核入口 |
| `kernel/src/lib.rs` | `no_std` 通用库与 host 测试入口 |
| `kernel/src/block.rs`、`kernel/src/block/memory.rs` | 块设备接口、错误类型和内存盘实现 |
| `kernel/src/block/tests.rs` | 内存盘边界与故障注入测试 |
| `kernel/src/hal.rs` | 硬件抽象入口 |
| `kernel/src/arch/riscv64/` | 汇编入口、链接脚本和 RISC-V 操作 |
| `specs/BOOT-001.md` | 第一份功能规格，写新 spec 时可参考它的粒度 |
| `specs/HAL-001.md` | 块设备与内存盘的行为和验收条件 |
| `scripts/smoke.py` | 启动检查脚本 |
| `tests/` | 外部测例版本与清单，目前不是已运行的测试集 |

## 5. 提交第一个小改动

在 [任务入口](backlog.md) 选择任务，到 Issue 说明你准备先做哪一小步、何时交付，请一名队友做 reviewer。先把预期行为写进 spec，经审阅后补一个会失败的测试，再写实现。

提交前运行相关测试和 `make check`，检查 `git diff`，确认没有日志、镜像或凭据。用 `git add <本次文件>` 暂存后执行：

```sh
git commit -m "feat: add memory disk boundary checks"
git push -u origin HEAD
```

在 GitHub 创建 PR：目标选 `MindScrath/Focos:main`，来源选自己的 fork 和分支。写清关联 Issue、spec、做了什么、怎样验证；邀请队友审阅。GitHub 登录或推送认证使用自己的凭据，不要把凭据放进命令示例或 Issue。详细规则见 [协作流程](collaboration.md)。
