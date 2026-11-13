# 第一天：运行 → 认领 → 提交

先通过开发机测试和 QEMU 启动检查，再交一个队友能快速审完的小改动。

## 1. 准备环境

需要 Linux、Git、rustup、make、sed、Python 3 和 QEMU RISC-V。开发机测试还需要本机 C 链接工具（`cc`）。Ubuntu / Debian 安装示例：

```sh
sudo apt update
sudo apt install -y git build-essential sed python3 qemu-system-misc binutils-riscv64-linux-gnu
```

按 [rustup 官方说明](https://www.rust-lang.org/tools/install) 安装 Rust 管理器。仓库的 `rust-toolchain.toml` 固定工具链；首次运行会下载。Fedora 使用对应系统包；WSL2 可用于 QEMU 开发，板卡连接需另配。

## 2. 获取并运行

在 [团队仓库](https://github.com/MindScrath/Focos) 点击 Fork，再将下方用户名和 fork 名改成自己的：

```sh
git clone https://github.com/YOUR_GITHUB_LOGIN/Focos.git
cd Focos
git remote add upstream https://github.com/MindScrath/Focos.git
git fetch upstream
git switch -c feat/1-memory-disk upstream/main
make check
```

`origin` 是个人 fork，`upstream` 是团队仓库。分支名中的 `1` 换成认领的 Issue 编号。

`make check` 检查格式和 clippy，运行 Rust 开发机测试与 Python 检查器测试，再构建内核并检查 QEMU 启动。全部通过时，终端会显示 `test result: ok`、检查器测试的 `OK` 和 `PASS: BOOT-001`，命令正常退出。

单独验证页分配、[HAL-001 内存盘](../specs/HAL-001.md) 和检查器用 `make test`，结果和失败断言显示在终端。只编译用 `make build`，只跑启动流程用 `make smoke`，启动日志在 `target/evidence/boot-qemu-rv.log`。

[MM-002 真实内存接入](../specs/MM-002.md) 另跑 `make mm-smoke`，通过时显示 `PASS: MM-002`。已验证 QEMU 8.2.2 / OpenSBI 1.3.1，以及 Fedora 45 容器中的 QEMU 11.1.3 / OpenSBI 1.8.1；版本不作为精确门禁，机器仍须使用 `virt`、128 MiB、单核和默认固件。实际内存范围、版本、命令和退出结果保存在 `target/evidence/mm-qemu-rv.log`；带 `dirty` 的源码版本表示工作区还有未提交改动。

| 遇到问题 | 先检查 |
| --- | --- |
| 找不到 QEMU | `qemu-system-riscv64 --version` |
| Rust 下载失败 | 网络与 rustup 安装 |
| 格式不对 | `cargo fmt --all` 后查看 diff |
| clippy 报错 | 按 `make lint` 输出的文件和行号修复，再复跑 |
| 开发机测试找不到链接器 | 运行 `cc --version`，缺失时安装本机编译工具链。Ubuntu / Debian 用上面的 `build-essential` |
| 开发机目标为空或不匹配 | 检查 `rustc -vV` 是否列出 `host:`，确认 sed 可用且 `HOST_TARGET` 未被覆盖 |
| 开发机测试出现 `FAILED` | 用 `make test` 复跑，按失败用例名和断言定位。内存盘用例见 [`kernel/src/block/memory/tests.rs`](../kernel/src/block/memory/tests.rs)，契约负例见 [`kernel/src/block/contract/tests.rs`](../kernel/src/block/contract/tests.rs) |
| 启动超时 | 日志、完整命令与当前 SHA；不要删断言 |

修复后重跑 `make check`。如果检查在启动前失败，请用本次终端输出定位，已有启动日志可能来自上一次运行。

## 3. 交第一份 PR

1. 从 [任务入口](backlog.md) 选一项，在 Issue 认领并约 reviewer。
2. 复制 [SPEC 模板](../specs/TEMPLATE.md)，先约定行为，再写失败测试和实现。示例见 [BOOT-001](../specs/BOOT-001.md)。
3. 运行相关检查和 `make check`，检查 diff；路径变化时更新 README 文件树。
4. `git add <本次文件>` 后提交并推送：

```sh
git commit -m "feat: add memory disk boundary checks"
git push -u origin HEAD
```

PR 目标选 `MindScrath/Focos:main`，按自动出现的模板填写。完整流程只有 [六步](collaboration.md)。当前启动成功不代表文件系统或历史测例已通过。
