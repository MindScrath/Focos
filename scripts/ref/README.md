# TEST-001 参考运行与证据回放

这里运行参考 Linux，校准判分与失败处理。结果不计为 FocOS 成绩。构建、runner、团队判据和测试均随本 PR 交付。

## 先验证管线

需要 Linux/POSIX、CPython 3.11+、已配置的 `gh`。缓存完整时无需联网；缓存缺失时通过 `gh` 从固定提交/Release 拉取。

```bash
make test-test001
make reference-replay
```

第一条运行清单一致性、构建故障注入、判分和 runner 契约测试。第二条核对历史归档 SHA256、38 个内部文件哈希，再回放 15 份机器结果，输出 `target/evidence/test001/replay/replay.json`。CI 同时执行这些检查；不依赖本地已有镜像、APK 或第三方 judge。

归档先在私有临时目录解包，校验完成后发布；目标、祖先或树内存在符号链接（含断链）时拒绝。复用 original 树仅在内容完全一致时读取，不覆盖不同内容；需要恢复被修改的证据时改用新 --out。归档不可放进受管 current 树，也不可与结果文件硬链接重叠；每次执行前移除旧摘要，子进程崩溃不能借用旧结果。报告以原子替换写入，避免修改其他硬链接。

历史诊断变化登记在 tests/reference-replay-updates.json，绑定归档 SHA256、具体样本/组及精确前后值。目前仅两份摘要新增 busybox 的未知命令标记；15 份的状态、逐项判分和数量均不变。报告保留 groups_match/all_match 的历史直接对比，另列 expected_groups_match/all_expected_match 与 diagnostic_updates；退出成功要求全部符合审计后对照。登记只允许 unexpected_in_log，不能改状态、分数、通过数；未登记或超出登记的变化仍失败。

官方 judge 不入库。`scripts/fetch_judges.py` 从 `inventory.json` 的固定提交拉取 5 个文件并校验各自 SHA256；已有缓存被修改时明确失败，不覆盖现场。需要重新下载时使用新的 `--dest`，并把对应 `judge/` 传给 runner 的 `--judge-dir`。

## 取得当前冻结输入

唯一输入来源为 [inventory.json](../../tests/inventory.json) 和 [reference.lock.json](../../tests/reference.lock.json)。压缩镜像 SHA256 为 `2b59aee4d26681b2d78a8faa615ac216f723a8ea157544e7867bbc8c77aac087`，解压后为 `95973543db6b84a9a5e70f30da466ce292867aff5b689fb14c88dc9406e378b8`。

```bash
mkdir -p target/ref-inputs
gh release download pre-20250615 -R oscomp/testsuits-for-oskernel \
  -p sdcard-rv.img.xz -D target/ref-inputs
xz -dk target/ref-inputs/sdcard-rv.img.xz
gh release download test-001-pr29-validation-2026-11-16 \
  -R ApolloMonasa/Focos-contrib -p linux-lts-6.18.63-r0.apk -D target/ref-inputs
python3 -m venv target/ref-env
target/ref-env/bin/python -m pip install ext4==1.4.1
```

APK 原始来源为 Alpine v3.23，SHA256 为 `47034305e6ddde9717652ce1da25d29b51ae88b997ab7c69b0beeee00ccc9369`。Release 保留相同字节，避免滚动包站撤下旧包后无法取回；包来源及 aports 提交见 lock。历史 6.18.60 APK 的旧 URL 已返回 404，其哈希保留在 `historical_kernel`，不把新内核结果替代旧测量。

## 构建与实际执行

使用支持 RISC-V virt 的 QEMU；本轮使用 QEMU 10.2.2 / Alpine 6.18.63。下例在同一启动中执行双 libc 的 Lua：

```bash
target/ref-env/bin/python -B scripts/ref/build_initramfs.py \
  --image target/ref-inputs/sdcard-rv.img \
  --kernel-apk target/ref-inputs/linux-lts-6.18.63-r0.apk \
  --out target/ref/lua.cpio.gz --kernel-out target/ref/Image \
  --modules-dir target/ref/modules --libcs glibc musl \
  --scripts lua_testcode.sh > target/ref-build.log

python3 -B scripts/run_tests.py \
  --capture-cmd "$(head -n 1 target/ref-build.log)" \
  --out target/ref/results/glibc --libc glibc --group lua --timeout 180 \
  --reference strict --reference-image target/ref-inputs/sdcard-rv.img \
  --reference-manifest target/ref/lua.cpio.manifest.json \
  --reference-artifacts target/ref/Image target/ref/lua.cpio.gz

python3 -B scripts/run_tests.py \
  --log target/ref/results/glibc/serial.log \
  --out target/ref/results/musl --libc musl --group lua \
  --reference strict --reference-image target/ref-inputs/sdcard-rv.img \
  --reference-manifest target/ref/lua.cpio.manifest.json \
  --reference-artifacts target/ref/Image target/ref/lua.cpio.gz
```

构建器第一行输出经过 shell 引号保护的完整 QEMU 命令，第二行输出构件哈希。`-snapshot` 保留原始镜像；不同组建议分别启动，避免前一组的挂载或文件操作影响后一组。内核/initramfs/manifest 必须保留，严格回放需要核对实际构件。每个输入/选择/源码版本请使用独立的 out、kernel-out、modules-dir 目录；相同内容可复用，不同内容会明确拒绝覆盖。

默认检查镜像、APK、release；缺失或重复模块、输出覆盖输入、非法脚本名均在构建阶段失败。只提取当前 release 的必需模块；`netperf_testcode.sh` 自动启用 dummy 网卡，iperf/netperf 预先配置 loopback。显式 `--network-dummy` 可用于其他实验。

`--allow-unpinned-inputs` 或指定外部 `--busybox` 只能生成标记为未冻结的实验构件；严格判分拒绝这些输入。为排查而放宽构建不等于验收通过。

cpio 的顺序、权限、属主和时间固定；gzip 固定 `mtime=0` 且不记录文件名。相同输入、选择、实现和压缩工具链在不同目录/时间得到相同产物与 manifest。manifest 记录 zlib 版本、输入和实现哈希、选择及输出哈希；内嵌构建 ID 把实际串口与该次构建绑定。跨 zlib 版本不承诺压缩字节相同。

## 失败怎样区分

| 标记/状态 | 含义 |
| --- | --- |
| `REF_PROTOCOL 1`、`REF_BUILD_ID <sha256>` | 新协议和本次构建身份 |
| `REF_MODULES_OK`、`REF_MOUNT_OK` | 必需模块加载和 ext4 挂载成功 |
| `REF_PREPARED <libc>` | 对应目录、链接、配置、权限、脚本存在及 chroot 预检成功 |
| `REF RUN <libc> <script>` | 开始一次脚本执行；组输出须位于对应尝试内 |
| `REF RESULT <libc> <script> <rc>` | 脚本实际退出码；非零不能被成功输出掩盖 |
| `REF_ALL_DONE` | 全部循环结束；不表示测试通过 |
| `REF_FAIL <stage>` / `ENV_ERROR` | 必需前置动作失败，或协议/构件身份不一致；停止后续测试，结果无效 |
| `CAPTURE_ERROR` | 捕获进程非零退出，即使组标记完整也不能通过 |
| `LOG_ERROR` / `TIMEOUT` | 组标记重复、嵌套、错配 / 缺 END 或捕获超时 |
| `JUDGE_ERROR` | 判分器来源、执行或输出不合法；不降级到其他判据 |

原始 `serial.log` 逐字节保留，包括 CRLF 和非 UTF-8 字节；仅解析视图进行解码。环境/捕获错误会清零有效通过计数，原观察值保留于 `observed`。捕获超时会杀死整个子进程组，避免 QEMU 后代继续持有串口管道。

参考标记自动启用严格检查；严格模式还必须提供 manifest 和两份构件。旧日志使用 `--reference legacy`，输出环境 `LEGACY_UNVERIFIED`、`execution_valid: false`，即使子项全部通过也退出 1。历史回放比较逐项结果与数量，明确移除原先没有物理意义的跨单位总分。

## 团队判据与边界

- lmbench 无 trace 的两个表头也会预留一次观测；缺失或不完整尝试不能被后续成功覆盖。trace 与紧随的首个表头合为一次，出现数据后再遇表头则是新尝试。
- 六个团队判据拒绝重复/冲突结果。指标名从同一冻结清单读取，避免独立维护多份名称。
- lmbench 按冻结源码的完整标签、单位和有效正数识别。lat_mmap 与四个读带宽指标的原始输出没有标签；新 harness 对 lmbench 使用 `sh -x` 记录命令上下文，脚本内容不修改。缺少上下文时不按数字出现顺序猜测归属。
- libc-bench 需要标签和完整 time 行；中间/尾随错误、重复 time、NaN/Inf 不构成成功。
- 性能组的“通过”只表示产生约定的有效结果，不表示达到 Linux 性能，更不能跨不同单位求和。
- symlink、chmod、hosts、loopback/dummy 是参考环境适配，必须随结果披露；不能声称原版镜像无需适配即可兼容。

历史结果见 [2026-11-08 测量](../../docs/reference/reference-run.md)，新验证结果及限制见 [2026-11-16 摘要](../../tests/results/reference-2026-11-16/summary.md)。这些均非 FocOS 或 VF2 成绩。

## 后续未解决评论的处理

构建器从当前 APK 的 `modules.dep` 拓扑排序必需模块，缺依赖或环时拒绝构建。锁定的 6.18.63 中 `virtio.ko` 依赖 `virtio_ring.ko`，而非反向；真实 QEMU 已验证该顺序可挂载并运行双 libc 的 Lua。详见结果目录的 `unresolved-fixes.json`。

普通 `--capture-cmd` 现在必须声明实际构件，例如 `--capture-artifact target/kernel --capture-artifact path/to/sdcard.img`，可重复传入；参考模式已有的 `--reference-artifacts` 仍有效。所有声明构件的初始哈希写入 `artifact_sha256`，运行期变化使结果失效；调用方应列全实际依赖文件。

`--inventory` 的团队组仅接受与冻结判据具有相同名称顺序和模式的快照，不兼容快照在执行前明确失败。复用 `--out` 会重新生成受管 `sections/`，不会夹带上次缺组或少选组的日志；不要在该目录存放输入文件。旧 Release 与 SOURCE 校验仍针对旧源码，不覆盖历史构建值。

严格参考模式现须提供 `--reference-image`：实际磁盘 SHA256 必须同时匹配所选清单与构建 manifest，磁盘加入构件哈希和变化监视。strict capture 仅接受构建器打印的直接 QEMU 命令（固定 virt/1 GiB/1 hart/snapshot 布局），核对命令中的 kernel/initrd/drive 路径及选项后，以 argv 直接执行；不接受 shell 包装、额外挂盘或路径错配。普通 capture 仍支持自定义命令与声明构件。

严格日志回放也要提供磁盘；它核对当前磁盘和构件内容，`reference_command_verified` 为 false，不表示重新执行或独立证明历史命令。历史完整证据仍需保留当时的采集记录。缺盘、错盘不能获得严格通过。

回放日志不得与 summary.json/summary.md 重叠，包括硬链接和符号链接别名；受管输出不得指向执行输入。唯一允许的原地模式是普通文件 `--log <out>/serial.log`，成功输出仍保留完全相同的字节。

受管输出别名在启动前和落盘前分别检查；采集命令运行期间新建的输出符号链接同样被拒绝，不能借输出写入覆盖原始输入。

strict capture 执行规范化的真实构件路径，声明路径及其解析后的路径都参与变化检查；实际 argv 写入 capture_argv，避免命令别名在核对后重定向到另一文件。

strict capture 还会解析 QEMU 的实际可执行文件，先取 SHA256，运行同一文件的 --version 并复查未变化，再执行真实路径；元数据 qemu 包含路径、版本探测输出和初始哈希，实际文件及调用别名都加入运行期检查。版本输出用于记录条件，不是供应商签名认证。

构建器在处理镜像/APK 前固定 builder、inventory.py、reference_manifest.py、清单与锁的身份；重操作后、发布 manifest 前复查源码和输入。输出不能覆盖这些源码/配置或其硬链接。清单与锁从同一字节快照解析、计算哈希，避免归因到后来编辑的版本。

原生 --log 输出现在包含 mode=replay、replayed_at 和 replayed_from（路径与原始字节哈希），不写 captured_at。真实 capture 才记录 captured_at；旧包的包装回放仍按其冻结 SOURCE 解释。脚本非零退出仅覆盖可判分状态，judge 缺失、崩溃或非法输出仍为 JUDGE_ERROR。

新构建使用 schema 2 manifest。构建 ID 同时包含实际内核 SHA256，以及 initramfs 解压后整份 newc 归档的 SHA256；后者仅将 `/init` 中唯一的 64 位 ID 字段归零，避免自引用，其他文件、脚本和元数据不变。严格回放重新解析实际 initramfs 并计算该值，同时核对归档里的 ID；仅改写 `outputs` 不能让旧日志通过新构件校验。重名路径、路径别名、非普通文件/目录、硬链接、截断和尾随归档均拒绝。压缩文件本身另以 `outputs.initramfs_sha256` 校验。

schema 1 的归档和记录保留原样，但不能声明 schema 2 的严格身份保证；重测请重新构建，历史对照可使用明确标为未验证的 legacy 回放。构建输出和 runner 的受管输出都检查彼此的路径/inode 冲突，拒绝复用目录中的硬链接覆盖。

显式 manifest 在读取嵌套字段前做共用结构校验：顶层及 inputs/selection/outputs/content 必须是对象，版本、SHA256、选择列表及类型须合法；非法文件退出 2，在任何 QEMU 版本探测、采集或判分前停止。解析与登记哈希使用同一字节快照。Lua 与 busybox 的未知 testcase 名称直接从原始行收集，独立于官方 judge 是否返回该项；不计入分母。

2026-11-17 这轮只修复边界并回放既有数据，未新采集参考 Linux。问题分析、完整检查及两份历史诊断差异见 [本轮摘要](../../tests/results/review-2026-11-17/summary.md)。

严格检查分两阶段：显式 manifest 的所有静态输入/构件校验（含内核、initramfs payload、嵌入 ID 与 BusyBox 实际内容）必须在 QEMU --version、采集或判分前完成，不匹配退出 2；采集后再核对日志的构建 ID、尝试顺序及协议。schema 2 的 builder_sha256、inventory_code_sha256、manifest_code_sha256 和 busybox_sha256 均为必填 SHA256，不能用缺少实现来源的简化对象声明严格通过。

runner 以 inode 缓存初始构件哈希，同一文件的相对路径、绝对路径、符号链接与硬链接不重复读取。声明/真实路径分别保留绑定监视；大构件逐组检查 inode、大小、mtime_ns、ctime_ns、模式与重定向，最终再完整哈希一次，小型源码及判据仍逐组重哈希。稳定镜像总计只做两次完整 SHA256，不随所选组数增长；变更立即使执行失效。源码在大构件读取前冻结；构建器仍保留两次发布检查，但在每个边界内部去重相同文件。

后续预检修复、真实 QEMU 及哈希读取次数记录见 [preflight 验证](../../tests/results/review-2026-11-17/preflight/summary.md)。旧 schema 2 证据只要原已含必填来源字段及正确构件，仍可严格回放；不把源码哈希强制改为当前版本来覆盖旧来源。

## 状态归并与构建来源

执行状态由一处表归并：明确环境/来源/协议错误优先；捕获超时或已知 EOF 截断保留 TIMEOUT；其他协议不完整、结构错误、完整日志的非零采集退出随后处理。script 失败只影响可判分状态，退出码与 judge_error 留在元数据。strict 的合法协议前缀未完成可保留 TIMEOUT，但执行无效、有效通过为零；错误构建 ID、非法顺序或 REF_FAIL 不会被超时掩盖。

APK 在同一字节快照上校验并解包；kernel/module 字节只在内存中交接给身份计算与打包，Image/modules 目录只用于发布及复核。源码、配置和输入还监视路径与文件属性；BusyBox 从打开的文件描述符核对身份，读取后复查，避免路径先验证、文件后替换。

产物先写私有临时文件，再用本地文件系统硬链接原子创建目标；同内容复用，不同内容拒绝覆盖，manifest 最后发布。构建目录应位于支持硬链接的 Linux/POSIX 文件系统。此操作不是跨文件回滚事务；失败可能留下无 manifest 的组件，不能据此认定构建成功。独立目录可避免不同版本争用默认 Image/modules。

本轮根因、受控进程/线程竞争、状态组合和最终实际启动证据见 [ownership 验证](../../tests/results/review-2026-11-17/ownership/summary.md)。旧测试已纠正为修改真实 APK 内容才改变输入身份，外来中间输出不能被采纳为本次 APK 的产物。

## Python 启动条件

-B 只禁止写 pyc，不禁止读取旧缓存。五个文件 CLI 入口在第一次项目导入前使用每次运行的私有空 pycache，并禁止写缓存；judge 子进程与 Makefile 测试命令也采用独立前缀。不会清理其他任务的共享缓存。若以 Python API 嵌入调用，调用方必须在首次项目导入前建立相同隔离条件；常规支持入口为本文的文件 CLI。

显式 --reference strict 在采集/回放入口统一要求 --reference-manifest、--reference-artifacts KERNEL INITRAMFS、--reference-image；任一缺失即退出 2，在读取日志、探测 QEMU 和判分前停止。自动检测到未经验证的参考日志仍不会成为有效通过。

后续缓存/参数复现和实际启动见 [startup 验证](../../tests/results/review-2026-11-17/startup/summary.md)。

## 判据快照与原始串口归档

runner 在采集/回放前冻结 judge 目录的全部普通文件（含包内 helper 和数据，排除 pyc 缓存），连同共用 inventory.py、lmbench_trace.py、清单和锁，核对记录的 SHA256。子进程在私有目录执行这些字节的只读副本，并移除继承的 PYTHONPATH；保留原始路径的变化监视。因此原 judge 目录在子进程打开时临时切换再恢复，也不能改变实际判分代码。自定义 judge 的本地依赖须放在该目录内；不支持从目录外动态导入未登记的项目依赖，运行环境不作为对恶意主机的沙箱。

lmbench 的 24 条冻结命令共用同一映射；未知命令按 shell 参数解析后规范化到 unexpected_in_log，改参数、空命令或坏引号都不能静默丢失；不改变 24 项分母。

四份新增原始串口含上游内核版权邮箱，已从代码树转存到 [固定外部归档](https://github.com/ApolloMonasa/Focos-contrib/releases/tag/test-001-pr29-raw-2026-11-17)。原始字节和所有已有摘要哈希不变，逐文件路径、SHA256 与归档 SHA256 见 [索引](../../tests/results/raw-evidence-2026-11-17.json)。下载、核对归档哈希后，在独立目录展开即可使用索引中的相对路径回放：

```sh
gh release download test-001-pr29-raw-2026-11-17 --repo ApolloMonasa/Focos-contrib --pattern '*.tar.gz' --dir target/raw-evidence
sha256sum target/raw-evidence/test-001-pr29-raw-2026-11-17.tar.gz
# 应为 317dc3548149b903166037ab80960a1c7680da43966f776e69479124f990ed53
tar -xzf target/raw-evidence/test-001-pr29-raw-2026-11-17.tar.gz -C target/raw-evidence
```

按归档 SOURCE.json 核对每份文件 SHA256；manifest/摘要仍在仓库原路径。后续判据快照回归与当前 runner 对既有原始采集的严格回放见 [judges 验证](../../tests/results/review-2026-11-17/judges/summary.md)。
