# TEST-001 重设计：阶段各自拥有输入和结果

本页定义构建、采集、运行包和批次边界；第三轮[解析契约与 A0–A3 门槛](test001-analysis-contract.md)替代 #29/#46 的解析实现路线。**当前仅有设计，没有新 runner 或新测量成绩。** 首个实现 PR 从冻结清单和日志分析开始；后续接构建与真实采集。

## 旧方案教训

审计基线为 [PR #29 的 50eee12](https://github.com/MindScrath/Focos/tree/50eee125a409207d8842b32d7d8d4264b10370e1)。该提交 CI 成功，问题是多轮评审持续发现边界缺陷，不能把“反复失败”简化为 CI 未通过。旧修复中已有有用的负例、固定输入和实际采集，保留作迁移依据。

| 反复出现的问题 | 设计原因 | 新约束 |
| --- | --- | --- |
| [judge 校验后换路径](https://github.com/MindScrath/Focos/pull/29#discussion_r4594088886)、[共享构建输出竞争](https://github.com/MindScrath/Focos/pull/29#discussion_r4593193057) | 校验者和消费者不拥有同一份输入 | 每阶段消费私有快照；摘要描述实际消费字节 |
| [strict 回放冒充原采集](https://github.com/MindScrath/Focos/pull/29#discussion_r4596324951) | 日志标记承担了证明主机运行条件的职责 | 运行包记录采集；外部日志只能作观察 |
| [TIMEOUT 被退出码覆盖](https://github.com/MindScrath/Focos/pull/29#discussion_r4593193046) | 一个 status 承载互相独立的事实 | 采集、协议、段完整性和判分分开保存 |
| [重复尝试抹掉缺失](https://github.com/MindScrath/Focos/pull/29#discussion_r4595882833) | 只按最终结果名称聚合 | 按尝试保留结果，先检查完整性再归并 |
| [输出祖先链接](https://github.com/MindScrath/Focos/pull/29#discussion_r4596172124) | 输出复用要求不断补别名和清理规则 | 输出目录独占新建，不支持原地回放或覆盖 |
| [自定义清单改判据来源](https://github.com/MindScrath/Focos/pull/29#discussion_r4596172107) | 外部输入可以定义自己是否“官方” | 首版只接受仓库冻结清单；来源由受审版本定义 |

不能把每条评审建议当作事实直接实现。例如[模块顺序评论](https://github.com/MindScrath/Focos/pull/29#discussion_r4584591073)与旧分支记录的锁定 APK 依赖方向有冲突：应读取该 APK 的 `modules.dep` 并真实启动验证，不能靠手写顺序或只返回成功的 shim 裁决。

## 四个阶段与四个命令

冻结清单是提交到仓库、人工审查的数据，不增加通用插件或配置语言。下面是拟定接口，**尚不能运行**：

```text
test001 build --profile PROFILE --image IMAGE --kernel-apk APK --libc glibc --libc musl \
  --group lua --out NEW_BUILD
test001 capture --build NEW_BUILD --firmware OPENSBI --timeout 180 --out NEW_RUN
test001 analyze --run NEW_RUN --criteria CRITERIA --out NEW_REPORT --require-pass
test001 analyze --log SERIAL --profile PROFILE --criteria CRITERIA --libc glibc --group lua --out NEW_REPLAY
test001 campaign --plan PLAN --out NEW_CAMPAIGN
```

以上四个公开子命令按 A（analyze）、B（build）、C（capture 与运行包分析）、D（campaign）交付；campaign 编排既有构建/采集/分析能力，不另设第五个基础阶段。PLAN 是批次输入计划，不是已有成功报告清单；其输入与验证要求见下文批次契约。

`--run` 与 `--log` 互斥。运行包分析默认涵盖其全部组与 libc；不允许通过只选成功部分获得整次运行的通过。capture 必须显式提供 `--firmware`，具体固件来源、版本与 SHA256 随 C 阶段冻结输入提交审阅；缺失或与冻结哈希不符时在启动前退出 2。外部日志必须显式指定 libc 和组；报告固定 `subject=unknown`、`platform=unknown`，不从日志标记、文件名或调用方描述推断来源。重复/未知组、空选择、非法时限和已存在输出均退出 2。首版不提供自定义 judge、清单、shell 命令、legacy/strict 自动切换和自由平台标签；新增后端需先补 SPEC。

| 阶段 | 输入与输出 | 不承担的职责 |
| --- | --- | --- |
| 冻结 | 版本化清单：采集配置（源码、镜像、APK、固件、调用/协议）与独立分析判据（judge、映射、单位/策略） | 不填写 FocOS 执行成绩 |
| build | 私有输入 → 内核、initramfs、基盘、构建清单 | 不启动 QEMU，不发布通过率 |
| capture | 校验构建包与显式冻结固件 → 原始串口、固件副本、采集清单 | 不加载判分器，不因文本 success 决定通过 |
| analyze | 运行包或外部日志 + 固定判据 → JSON/Markdown 报告 | 不重跑被测程序，不修改运行包 |

每阶段是独立进程；内部模块按这四项职责分开。CLI 只校验参数和调用阶段接口。分析器可重复消费同一运行包；更新判据产生新的报告，并记录新判据身份。

## 版本化配置与 LTP 适配

采集配置与分析判据分别版本化，二者均为仓库内已审 ID，不接受任意配置路径或自定义脚本：

- `PROFILE` 只描述采集事实：schema_version、profile_id/revision、输入哈希、显式调用清单、运行适配器/日志协议身份、环境和分片表。旧非 LTP 与含 LTP 全套使用不同 ID；用例、参数、调用/分片顺序、运行适配器、环境或分片变化生成新 revision 和哈希。build/capture 绑定这份配置；report/campaign/summary 原样引用其身份和实际 selection，不能用新清单补出旧运行没有的覆盖。
- `CRITERIA` 描述分析行为：criteria_id/revision、结果适配器、判据/验收策略、完整分析实现及 helper/数据文件哈希，并列出支持的采集配置精确哈希、日志协议和每个冻结项的映射。只修改解析器或判据时只更新此身份，不改变原采集配置。支持声明须经审查，不能只用版本范围推测兼容；不得增删、合并或重编号来源配置的覆盖项。新判据所需证据若未采集，报告 BLOCKED 并列出缺失，不猜测或补造。

`analyze` 两种输入均须显式指定 `--criteria CRITERIA`，未知 ID、缺失参数、配置不受支持或覆盖映射不完整均在分析前退出 2。`--run` 的采集配置从运行包读取，不接受 `--profile`/组/libc/分片覆盖；`--log` 须显式指定 `--profile` 和范围，但 origin 仍为 external_log、subject/platform 仍为 unknown。例如同一 RUN 绑定 P1，分别用支持 P1 的 J1/J2 产生两个新 REPORT：run_id、采集配置哈希、日志哈希和分母不变，判据身份与结论可不同，不启动 QEMU。

批次在启动前同时冻结采集 PROFILE 和本批使用的 CRITERIA 身份；原批次的所有子报告必须匹配两者，汇总仍验证原计划的分析实现身份。在批次执行器之外分析带 campaign 绑定的 RUN 时（包括仍用 J1）保留其来源 campaign/slot 绑定，但另记 `analysis_role=reanalysis`；只有批次执行器按预登记判据产生的原报告为 `campaign_original`，独立运行/外部日志报告为 `standalone`。角色不是凭据，汇总还须核对原 results.json 中的报告哈希、两种身份及全部绑定；重判报告不能替换旧批次子报告或覆盖旧 summary，也不能冒充新批次采集。当前不提供整批重判汇总命令，单次重判报告只代表来源运行范围。

每条调用冻结稳定 invocation_id、组/libc、二进制与数据哈希、argv、工作目录、环境变量、预置条件、运行适配器及预期结束协议；结果适配器按 CRITERIA 独立冻结。同一程序的不同参数和计划重复执行必须有不同身份；同一调用的意外重试阻断验收。分片为固定有序调用列表，PROFILE 同时冻结片间顺序；预期序列推导、观察序列和逐项核对规则见[解析契约](test001-analysis-contract.md)。任何排除项须事先记录依据和范围影响并审阅，不能把 FocOS 缺失系统调用当作排除依据。含排除的配置只能声称其明确覆盖范围通过，不能声称 2025 全部通过；区域赛完成仍需所有必达项。

比赛固定源码中的 [LTP 脚本](https://github.com/oscomp/testsuits-for-oskernel/blob/8b58dd16d26d30f7c74d48d5832d870d3051b703/scripts/ltp/ltp_testcode.sh)遍历镜像 `ltp/testcases/bin` 的文件，返回后无条件打印 `FAIL LTP CASE ... : <ret>`。所以不能把 FAIL 字样直接当失败，也不能以源码目录名推断镜像用例集合。A 必须核对实际镜像展开项及顺序、参数、二进制、`ltp-full-20240524` 源码与判据对应关系，再冻结清单；当前数量仍未知。不能声称比赛范围等于最新上游完整 LTP。

运行适配器只调度被冻结的调用，记录独立 start/end、调用 ID、退出码/信号、前置与清理结果；不得由日志出现顺序猜测身份。LTP 原生输出、汇总与退出码分别留存，原始字节和定位可回查。新增边界标记、环境准备或分片调度属于显式 instrumentation，须冻结补丁/实现与版本，保留比赛原脚本对照，不能声称未经修改的原版执行。

结果适配器按比赛固定版本解析 TPASS/TFAIL/TBROK/TCONF/TWARN，[固定版本结果标志](https://github.com/oscomp/testsuits-for-oskernel/blob/8b58dd16d26d30f7c74d48d5832d870d3051b703/ltp-full-20240524/include/tst_res_flags.h)中的 TINFO/TDEBUG 仅为信息；原始事件与原生汇总分别保存，不把两者重复加总。保留原生计数和未知标志，按固定版本处理退出位组合，不把非零一律解释为断言失败。解析器崩溃/非法输出是 JUDGE_ERROR；测试程序 TBROK、信号终止、超时是被测调用结果，不能改成判分器错误。未知/无法归属输出、缺结束或无有效结果均阻断验收。

每调用只有在结束完整、退出 0、至少一个有效 TPASS、无 TFAIL/TBROK/TCONF/TWARN、前置与清理均成功且满足冻结判据时才能 judgment=PASS；混合 TPASS 与跳过/警告仍不通过。TCONF/跳过、TBROK 和 TWARN 各自保留计数、原因及阻断项，不降级为通过。原生断言计数可动态变化，`selected_total` 只统计预先冻结的调用；`observed_pass` 按通过的调用计，动态断言另列，不能用运行后观察到的断言数或跳过数改覆盖分母。原有 origin/capture/protocol/input_integrity 等验收条件仍全部适用。

配置为每调用/分片冻结权限、用户、挂载/设备/文件系统、网络、工作目录、环境、CPU/内存、时限、输出限额与清理策略。开始前真实检查依赖；失败后不执行该调用，保留失败事实及未运行项。分片内串行执行且无隐式重试，前一调用清理失败即停止该槽，后续调用记 NOT_RUN；下一槽必须全新 QEMU 与写时复制层。主机超时仍回收整个进程组。参考 Linux 校准不能代替 FocOS 能力，FocOS/VF2 采集适配器仍须另行审规格。

新增验收负例：纯 TCONF、TPASS 混合 TCONF/TWARN、TBROK、退出位组合、只有退出 0 而无结果、事件与汇总重复计数、按动态断言数量改覆盖分母、缺结束、重复调用、清理失败后仍继续、分片漏项/重叠、配置身份不匹配、事后修改资源策略。均先用冻结日志/计划证明失败；动态断言数量变化但调用完整通过时，调用分母应保持不变；正例须有真实参考 Linux 的 LTP 调用与完整证据，不用模拟器返回成功代替。

## 文件与发布契约

以下为待实现的包布局，不是当前仓库文件树：

```text
BUILD/  kernel  initramfs.gz  base.img  build.json
RUN/    serial.log  firmware.bin  capture.json
REPORT/ report.json  report.md  [source.log：仅外部日志分析]
```

本次拟定包格式为 `schema_version: 2`（含配置与显式调用绑定），旧 v1 不自动升级或推断缺失字段；本仓库 regional-plan.json 是范围计划，不是可执行配置。所有包 JSON 独立声明版本，未知版本、重复键、未知必需语义、NaN/Inf、非法计数均拒绝。清单内文件路径只允许固定相对名称；拒绝绝对路径、`..`、重复成员、符号链接和特殊文件。每个消费阶段先读取同一份清单字节，再校验其所有必需文件的长度与 SHA256。清单自己的 SHA256 由下游记录，不引入自引用哈希。

- `build.json`：采集配置 ID/revision/哈希与内容、显式 selection、由 PROFILE 推导的各组/libc expected_sequence、冻结清单哈希、构建实现文件集合及哈希、工具版本、输入哈希、组/libc 顺序、内核/initramfs/base.img 哈希及长度、环境适配项。`base.img` 是冻结镜像的完整字节副本，其输出哈希必须等于冻结输入镜像哈希。不得设置“已运行”。相同输入、选择、实现和压缩工具版本应得到相同构建输出；运行时间不写进可复现载荷。
- `capture.json`：每次 capture 调用新生成的 run_id（UUID）、构建清单哈希与副本内容、继承的采集配置身份、selection 与 expected_sequence、实际内核/initramfs/基盘哈希、固定 `subject=reference_linux` 与 `platform=qemu_riscv64`、QEMU 路径/二进制哈希/版本、实际 OpenSBI 固件的来源/版本/原路径及 firmware.bin 的 SHA256/长度、实际 argv、开始/结束时间、采集状态、退出码、限额、原始串口哈希及长度、写时复制层的初始 SHA256/长度/格式、创建结果及基盘绑定、只读依赖启动前/进程回收后的校验哈希与 input_integrity、采集实现身份、campaign 绑定（独立 capture 时为 null，批次运行时包含 campaign.json SHA256 和 slot_id）。主机启动生成的 argv 才是采集条件，来宾打印的 build ID 只作一致性检查。
- `report.json`：输入种类与字节哈希、被引用采集清单哈希、run_id 与来源 campaign 绑定、analysis_role、来源采集配置 ID/revision/哈希与 selection、实际 CRITERIA ID/revision/哈希和完整分析实现/判据文件哈希、subject/platform、所选范围与分母、各组/libc 的 expected_sequence/observed_sequence 及顺序诊断、逐项结果、额外名称、各独立状态、验收阻断原因和计数。报告完成标记为最后写入的 `report.json`；消费者须一并核对其中记录的 `report.md` 哈希。串口保留在运行包；外部日志报告保存实际消费的来源副本及哈希，不覆盖原文件。

输出路径从可信工作目录逐级以不跟随符号链接的目录句柄访问，独占创建最终目录；后续写入相对此句柄，不重新解析可能被改名的祖先。文件独占新建，不采用先 exists 再覆盖写。文件写入失败只留下未完成目录；清单在所有文件关闭、校验完成后发布。读者仅接受完整清单及其全部文件。首版不承诺掉电事务；重启后仍重新校验，损坏包不能通过。

目录权限为创建者私有。这里防的是正常开发中的并行构建、路径误用、陈旧缓存和半写结果，不承诺抵御拥有同一账户或 root 权限的恶意进程。SHA256 证明内容一致性，不证明发布者身份。

## 输入如何真正固定

构建器把 APK 与镜像复制到自己的暂存区，校验复制后字节与冻结值，再从该副本提取内核、模块和 BusyBox。内存中的提取结果直接交给归档器，不通过共享 `Image/modules` 路径回读。已校验镜像以 `base.img` 纳入构建包，不只保留外部路径；移动完整 BUILD 或删除原始 IMAGE/APK 后仍能采集。模块按该 APK 的依赖图排序；缺模块、环或 release 不符在启动前拒绝。

采集器只从 `--build` 指定包取得内核、initramfs 与 `base.img`，复制到私有运行区并核对哈希；基盘缺失或不匹配在启动前退出 2，不回查 build 时的 `--image` 路径。QEMU 从主机工具链解析、复制并记录身份。显式 `--firmware` 的 OpenSBI 文件先复制到私有运行区的 `firmware.bin`，对副本计算 SHA256 并匹配冻结值，实际 argv 必须使用 `-bios <该副本的绝对路径>`；禁止 `-bios default`、`none` 或隐式搜索主机固件。固件副本随运行包保留，分析时校验其长度、哈希及与采集 argv 的绑定；原始固件路径变化不影响已固定的副本。argv 的被测文件只指向这些私有副本。基盘只读，以本次独占新建且尚无来宾写入的写时复制层承载测试写入；不接受已有层或其他运行遗留层，创建失败不启动。启动前记录该层初始哈希、长度、格式和所绑定基盘的哈希；禁止其他进程复用该层。该层是运行期间唯一允许来宾写入的磁盘文件，内容变化属于预期行为；内核、initramfs、基盘、固件、QEMU 等只读依赖仍须在整个采集进程组回收后复核未变，校验结果记入 input_integrity；无法完成后检为 UNVERIFIED，变化为 CHANGED，两者均阻断验收。动态链接器/系统库属于受信主机工具链，记录版本，不声称只凭 QEMU 哈希已冻结完整主机。磁盘空间不足即失败，不退回原路径或硬链接。原始输入在复制期间变化，只有副本哈希仍等于冻结输入时才可继续。

分析器先把运行包全部必需文件或外部日志复制到私有快照，校验后从同一快照计算报告哈希并解析，不在判分后重新读取来源路径。外部日志报告保留该字节副本 `source.log`，report.json 记录其长度与哈希；原路径只作来源提示。运行包的 capture.json/serial.log/firmware.bin 在复制后按清单校验，缺失或不匹配退出 2。分析器再冻结完整的源码与判据文件集合，并在隔离 Python 搜索路径、空 pycache 的子进程中执行该副本；主脚本、helper、包、数据均显式列入。用于独立对照的官方 judge 只能使用本次 CRITERIA 冻结提交中列出的文件，额外同名标准库模块也拒绝。被执行的项目模块均来自副本，报告哈希也从副本生成；不依赖运行前后扫描原目录来证明实际执行身份。整轮历史回放共享一次分析源码快照。

镜像完整复制和哈希只发生在阶段边界，不能随判分组数增长。构建包和运行包容量、串口限额、判据输出限额需在相应实现 PR 给出默认值和超限负例；不为省空间改用共享可写文件。

## 分开事实与验收结论

| 字段 | 允许状态 | 由谁决定 |
| --- | --- | --- |
| origin | `recorded_capture / external_log` | 完整运行包校验或显式日志输入 |
| capture | `EXITED / TIMEOUT / OUTPUT_LIMIT / START_ERROR / NOT_RECORDED`，另存退出码 | 主机采集器 |
| input_integrity | `VERIFIED / CHANGED / UNVERIFIED` | 采集器对只读依赖的前后校验；外部日志为 UNVERIFIED |
| protocol | `VALID / INCOMPLETE / ERROR / NOT_RECORDED`，附阶段错误和脚本退出码 | 协议解析器 |
| section | `COMPLETE / TRUNCATED / ABSENT / INVALID` | 组与 libc 的标记配对 |
| judgment | `PASS / PARTIAL / FAIL / MISSING / JUDGE_ERROR / NOT_RUN` | 固定判据与规范化 |
| acceptance | `PASS / BLOCKED`，附全部原因 | 分析器的单一纯函数 |

`acceptance=PASS` 当且仅当：来源为校验完整的运行包、capture 为 EXITED 且退出码为 0、input_integrity 为 VERIFIED、协议 VALID、每个所选脚本退出 0、范围内所有段 COMPLETE、范围内所有冻结项 judgment 为 PASS、每个组/libc 的观察调用序列与冻结预期序列长度及逐项身份完全相等、没有判据错误、未知用例或 selection 外的已观察调用。未知项单列且不改分母，但阻断验收，促使重新审查清单。运行包伪造不在本地主机信任模型内；把普通日志附上当前构件或相同 build ID 不改变其 origin。

例如 `capture=EXITED(exit=7)`、`section=TRUNCATED` 与 `judgment=PASS` 可以同时存在，报告保留三项事实，验收为 BLOCKED，原因包含非零退出与截断；不挑一个状态覆盖其他信息。`observed_pass` 统计规范 judgment 为 PASS 的调用（旧 judge 对照计数另列），`accepted_pass` 只有本运行范围验收通过时才等于该范围的 `selected_total`，否则为 0；始终并列显示 origin 和 subject。性能指标保留单位与原值，不合成总分。

外部历史日志仍检查组边界，并可诊断其中协议，但报告的 capture 为 NOT_RECORDED，subject/platform 均为 unknown。运行包分析的 subject/platform 从已校验的采集清单读取，首版仅允许 reference_linux/qemu_riscv64；未知或冲突值作为输入错误拒绝。完整运行包在另一次分析中保持 recorded_capture，表示可追溯的记录，不表示此次重新执行、硬件认证或 FocOS 成绩。

## 范围、汇总与退出码

构建清单的 `selection` 是已审配置中显式 `(group, libc, invocation_id)` 集合，非 LTP 冻结项也映射为稳定调用身份；`selected_total` 为该集合大小。组/libc 参数只能选择配置内集合，build 与外部日志 analyze 可用 `--shard SHARD_ID` 选择已审 LTP 分片（须恰好单 libc、group=ltp，缺省选择该组全部调用；未知或冲突参数退出 2）；LTP 分片只能通过已审 shard_id 选择，不能传任意命令或临时过滤器。消费者须从已校验 PROFILE 和 selection 独立推导 expected_sequence，再核对构建/采集/报告中的副本；副本缺失或不符是输入错误，分析前退出 2，不能使用运行包自报的任意顺序。原日志中实际顺序偏离则按解析契约保留报告、阻断验收。分析器核对完整集合，不得以旧 427 项代替本次范围，也不得在分析时删掉失败组。PROFILE 内未选择的项不因未出现而生成 MISSING/NOT_RUN；但实际观察到未选择的调用（即使是 PROFILE 已知的同组同 libc 调用）必须按[解析契约](test001-analysis-contract.md)记 OUT_OF_SELECTION、协议 ERROR 并阻断验收，不计入范围内通过数或改变分母。范围内缺组为 NOT_RUN、活动组内缺项为 MISSING；有明确前置/清理失败证据而未执行的后续调用为 NOT_RUN。运行包中出现范围外组/libc 是协议错误，不能悄悄扩展范围。

按候选旧清单，Lua 每套 libc 为 9 项：示例的双 libc Lua 包分母为 18，全部条件满足时 `accepted_pass=18`、`--require-pass` 退出 0；报告必须标明只覆盖 Lua。旧非 LTP 候选全清单为每套 libc 427 项、双 libc 共 854 项；这些数值不适用于含 LTP 配置，不能将 18/18 展示成全套通过。

D 阶段由单一批次执行器负责登记、运行和汇总，拟定入口 `test001 campaign --plan PLAN --out NEW_CAMPAIGN`（待 D 实现）。独立 `capture` 或事后挑选的运行包只能作单次报告，不能填入全套批次。批次执行器遵循下列规则：

1. 在任何 QEMU 启动前，校验并私有复制整批依赖，独占创建批次目录，发布不可变的 `campaign.json`。它包含随机 campaign_id、采集 PROFILE 与分析 CRITERIA 各自的 ID/revision/哈希及内容、冻结清单哈希、从已审配置推导的固定顺序全部槽位、每槽唯一 slot_id、单组单 libc 且含显式调用集合的 selection、构建包清单哈希、统一的固件/QEMU/运行策略以及构建、采集、分析、campaign 实现与判据身份。各槽调用集合必须无重叠，其并集必须恰等于配置的全部纳入调用；非 LTP 每组/libc 一槽，LTP 每用例或预审固定分片一槽。缺少、重复、额外调用或未冻结配置在启动前退出 2，不预设 22 或 24 槽。此时尚无采集/报告哈希；这些只在结果产生后登记，不能声称预知未来产物哈希。
2. 同一批次仅一个执行器持有排他锁；每槽固定新目录，在启动前以独占创建写入 `started.json`，含 campaign.json SHA256、slot_id 和采集器生成的 run_id。关闭并同步该文件及父目录成功后才可启动 QEMU。claim 已存在就拒绝再次启动，不能覆盖、删除或分配新的 run_id 来重试。批次只能新建运行，不提供恢复或重试入口；执行器崩溃后该批次保留为未完成。写 claim 失败则不启动该槽。
3. 执行器按计划每槽最多启动一次，采集清单必须写入相同 campaign 哈希、slot_id 和 run_id，分析报告须继承这项绑定。普通 capture 不接受调用方传入的 campaign 标签；批次绑定仅由持有锁的执行器交给内部采集接口。某槽启动失败、超时或退出非零也保留 claim 和可发布的失败运行包，继续其余槽；若进程、磁盘或发布故障阻止继续，则余槽保持未运行。
4. campaign.json 和 started.json 只证明登记/认领，不是批次完成标记。汇总从 campaign.json 的全部固定槽位和实际记录枚举输入，禁止调用方另交成功报告列表。每个 claim 必须关联唯一的完整采集清单与报告，核对所有哈希、身份、selection 及 PROFILE 推导的 expected_sequence；未启动、缺 claim、缺结果、绑定冲突、额外槽位/claim/结果或任何失败都使全套 BLOCKED。额外的独立重跑不能补回缺槽或覆盖失败。结果索引在运行后记录全部槽位（含失败和未完成）的清单/报告哈希及缺失原因，最后发布包含该索引哈希的汇总报告。批次结构固定为 `campaign.json`、`implementation/` 源码快照、`slots/<slot_id>/started.json`、各槽的 `run/` 和 `report/`、`results.json` 与最后写入的 `summary.json`；slot_id 由执行器生成且只允许安全单段名称。

campaign 实现身份必须覆盖编排入口、槽位登记、跨槽校验、汇总判定与序列化，以及它们的本地 helper/包/数据的完整文件集合和逐文件 SHA256；只记主脚本或 Git 提交号不足。发布 campaign.json 前冻结这份私有只读源码快照，随后编排与最终汇总都从该快照执行，隔离 Python 导入路径和 pycache，不重开原路径执行。最终 summary.json 必填采集配置与分析判据各自的身份、campaign_id、campaign.json SHA256、results.json SHA256、实际 campaign 实现文件集合及逐文件哈希、全套结论与全部阻断原因；实现身份必须与计划完全一致，缺失或不符只能报告 BLOCKED，不能发布全套 PASS。快照按固定相对路径归档在批次的 implementation/ 中，summary.json 记录其文件长度和哈希；消费者复核归档与计划/汇总一致，不将不同汇总版本的结论归因给同一实现。

每个槽位还须满足以下条件，否则全套为 BLOCKED 并列出冲突槽位：

- 对槽位 `(g, c, shard_id)`，构建包、批次计划、采集清单内的构建选择、报告范围必须一致，且组列表恰好为 `[g]`、libc 列表恰好为 `[c]`，显式调用集合与预审分片完全一致；非 LTP shard_id 固定为整组。不能从多组/多 libc 运行中截取子集。双 libc Lua 示例可用于 C 阶段接口验证，不能作为 D 的单槽证据。
- 每槽来自独立一次采集和一次全新 QEMU 启动。所有槽的 run_id、采集清单哈希和报告哈希分别不得重复；复制、改名或对同一运行包重新分析不能成为另一槽。报告必须绑定对应采集清单哈希和 run_id，并完整校验运行包；文件路径不同不构成独立运行证据。串口哈希不要求互异，独立运行可能产生相同文本。
- 要求相同冻结清单、被测系统/平台、基盘/APK/工具链、实际固件 SHA256、实际 QEMU 二进制 SHA256、构建实现、采集实现与环境适配策略，以及同一分析实现和判据快照。采集实现一致性以各 capture.json 中实际执行的完整实现文件集合及逐文件 SHA256 相等为准，不只比较工具名称或版本文本；首版不接受兼容版本集合。固件来源/版本用于追溯，版本文本相同不能替代字节哈希；身份缺失、文件集合不同或任一哈希不同均阻断全套验收。
- 实际 QEMU argv、采集时限/输出限额须与批次预登记策略匹配。比较 argv 时，只将已校验并绑定的内核/initramfs/基盘/写时复制层/固件及 QEMU 私有路径替换为固定角色名；其他机器型号、CPU、内存、核数、设备、内核启动参数及启动选项逐项匹配预审配置，不忽略未知参数。公共条件跨槽相等；允许差异的设备、资源与时限必须在配置中按槽列明并随计划冻结，不允许运行失败后放宽。构建选择、构建包哈希、run_id、时间及串口允许不同。每槽有效参数及只读依赖角色文件哈希须同时匹配计划；写时复制层匹配预登记的创建策略、格式和基盘绑定，各槽必须新建，初始哈希只记录到各自 capture.json，不要求跨槽相等，也不把来宾写入后的哈希与初始哈希作不变性比较。不能仅以“同一采集器”代替实际条件核对。
- 每个槽位覆盖恰好一次，每个子报告均满足本范围验收，才可报告全套 PASS。缺槽、重复槽或条件不一致均 BLOCKED。重测必须从新的 campaign_id、该配置完整槽位及独立目录重新开始，保留并引用前一批次身份；不支持在旧批次补跑后换掉失败包。分母由各槽调用集合大小相加；动态断言不累加到覆盖分母。原非 LTP 配置须单独报告，不因新配置通过而省略其回归。

run_id 由受信采集器在启动前生成，不能由调用方指定；campaign/claim/结果记录用于检查本工具正常使用下的遗漏与重复。登记、claim 与汇总记录只独占新建，各槽 run/report 仍遵循原有包发布规则；缺最后 summary.json 的批次不能当作已完成，通过只读审计可报告其未完成原因。所有记录沿用本地主机信任边界，不承诺阻止同账户恶意改写记录或隐瞒整个批次；全套报告结论仅针对明确展示 campaign_id 的这一批次。

下表均以参数与包校验通过为前提；输入/参数错误优先退出 2，不执行被测程序。发布失败或工具自身无法完成阶段退出 1，无可消费的完成清单。

| 命令及结果 | 退出码 | 报告/证据语义 |
| --- | --- | --- |
| build 完成并发布构建包 | 0 | 尚未采集 |
| capture 正常退出 0 并发布运行包 | 0 | 尚未判分；来宾协议/脚本仍可能失败 |
| capture 启动失败、超时、超限或非零退出 | 1 | 能完成发布时保留失败运行包供诊断；不表示验收通过 |
| 普通 analyze 完成报告，含 BLOCKED、缺项或 JUDGE_ERROR | 0 | 分析完成，观察结果和验收结论见报告 |
| analyze --require-pass 完成报告且本范围 acceptance=PASS | 0 | 只认可报告明确列出的范围 |
| analyze --require-pass 完成报告且 acceptance=BLOCKED | 1 | 保留报告及全部阻断原因；外部日志必在此列 |
| campaign 完成且全套 PASS / 任一槽失败或批次未完成 | 0 / 1 | 保留全槽索引与能发布的汇总；启动前计划/输入错误为 2 |

待实现检查按边界分别验证：

| 边界 | 负例与预期 |
| --- | --- |
| 实际输入 | 缺固件/复制后哈希错误时不启动；替换主机默认固件不改变私有副本与 -bios；篡改运行包 firmware.bin 时分析退出 2；移走构建原始输入后仍可采集；构建包缺失/篡改 base.img 时不启动 |
| 磁盘写入所有权 | 来宾写入只改变本槽写时复制层时正常保留结果；复用旧层/错基盘时不启动；只读基盘变化阻断；初始层字节不同但创建策略和基盘一致不误判条件冲突 |
| 分析快照 | 冻结外部日志后替换原路径，报告的 source.log、哈希和解析结果仍对应同一快照；截断或篡改已记录运行包则退出 2 |
| 判据升级与重判 | 同一 P1/RUN 用支持 P1 的 J1/J2 分别生成报告，无 QEMU 启动且采集/日志哈希及分母不变；不支持的配置或映射缺项退出 2，新增证据要求无法满足则 BLOCKED；J2 报告替换原批次 J1 报告必须被拒绝 |
| 单槽与范围 | Lua 单组成功无需其他组，范围内缺项仍失败；把双 libc 或全套报告拆填多槽、改名复制或重新分析同次采集均 BLOCKED |
| 批次所有权 | 某槽失败后独立重跑成功，替换进批次必须因绑定不符/claim 冲突而 BLOCKED；省略失败槽、孤立 claim、缺失结果、额外尝试均 BLOCKED；并发争用同槽仅一个 claim 成功且最多启动一次；claim 发布失败不启动；claim 后崩溃不能重启该槽 |
| 汇总实现身份 | 改动本地汇总源码后冻结快照的执行和身份仍保持不变；计划或 summary 缺实现集合、增删 helper 或改变哈希均不得全套 PASS；不提供用新汇总实现重写旧 summary 的入口；独立重判报告标明实际判据身份，不能替换原子报告或覆盖旧批次汇总 |
| 条件一致性 | 各槽都 PASS，但一槽采集实现/固件/QEMU 身份缺失或不同，或机器参数/时限/输出偏离该槽预登记策略，全套仍 BLOCKED；已审配置显式允许的跨槽差异且各自匹配计划时不误判；不能用相同版本文本掩盖字节不同 |
| 完整成功与退出 | 全新批次内配置规定的全部单组单 libc 独立槽位且全部条件满足才全套 PASS；外部全 PASS 日志普通分析退出 0，require-pass 退出 1，均显示 unknown 来源 |

## 尝试、判分与真实环境

解析、尝试归属、规范判分、旧 judge 对照和性能指标以[解析契约](test001-analysis-contract.md)为唯一详细定义。旧 judge 的 `all/pass` 只保留为对照，不直接决定最终 judgment；不能与独立 proof 拼接通过。

模块、挂载、网络、目录/权限、链接和 chroot 预检分别检查真实返回值；失败停止后续组。`ALL_DONE` 只表示循环结束。真实 QEMU 的正常与故障运行必须覆盖整条构建—启动链；伪造命令返回值的 host 测试只验证控制流。

## 实现顺序与验收矩阵

各阶段从最新 main 开独立 PR，复用旧代码前先迁移与该阶段有关的失败测试。先审本设计，再进行以下实现；不要把整张表再次塞进一个实现 PR。

| 顺序 | 可独立交付的内容 | 必须先暴露的缺陷 | 合入证据 |
| --- | --- | --- | --- |
| A | 冻结清单、纯日志分析和报告 | 外部全 PASS 被当成验收；重复/嵌套/错 libc；缺项、异常 JSON、NaN/Inf、错单位、重复及缺失重试；未知名被吞掉 | host 负例；15 份固定历史结果逐项对照，诊断差异单列，来源始终 external_log |
| B | 独占目录与构建包 | 共享 APK/模块输出污染；输出软/硬链接；重复构建不确定；缺模块/环；发布前崩溃和磁盘满 | 并行进程测试；不同目录/时间相同产物；失败无可消费清单；旧输入/证据哈希不变 |
| C | QEMU 采集与运行包分析 | 换内核/盘/固件/argv；前置动作失败继续执行；超时后代仍运行；半写包；非零退出或截断仍通过 | 真实双 libc Lua；坏盘/缺模块故障；进程回收和输出限额；完整包篡改与独立日志对照 |
| D | 原 11 组与比赛 LTP × 两套 libc 参考校准 | 分母、脚本、判据与实际镜像不一致；跨组污染；回放混用版本；采集实现不一致、重复引用多选运行或省略失败尝试仍汇总通过 | 每槽单组单 libc 独立启动；采集前批次登记与完整尝试索引、全部槽原始日志及逐项结果、平台/工具条件；未通过项明确原因 |

A 必须按[解析契约的 A0–A3](test001-analysis-contract.md)分别提交（先审计，再纯解析，再外部日志 CLI，再逐组扩展）；A 不接受运行包，`--run` 到 C 才开放。但未覆盖全部冻结组前不能声称 A 完成；所有组（含 LTP）的判据与日志测试仍须完成；15 份历史回放仅覆盖旧非 LTP 路线，LTP 须新增来自冻结版本的真实日志样本及上述 LTP 负例。B 补齐 LTP 数据/设备依赖打包，C 增加 LTP 小规模真实正反例，D 覆盖完整冻结调用集合。D 是参考 Linux 校准，不能关闭 #8 的 FocOS 和 VF2 验收。只有稳定通过且队友确认后，另行调整 CI 必过集合。

组合测试对上述独立状态做笛卡尔积：只允许唯一完整成功组合通过，其余组合保留全部原因。状态测试不调用被测验收函数来计算期望值。另做变异检查：移除退出码约束、允许外部日志通过、丢弃首次失败、改回原路径执行、覆盖已有输出，分别必须触发失败。

## 迁移与历史材料

新设计替代 #29 和 #46 的合入路线，不删除其分支或改写历史。#46 的代码、测试和候选清单仅作审计/移植材料；第三轮先审规格，不继承其实现验收结论。旧代码仅作逐模块移植来源；本 PR 不导入 34 个修补提交、历史摘要或未复核成绩。旧分支的 [清单](https://github.com/MindScrath/Focos/blob/50eee125a409207d8842b32d7d8d4264b10370e1/tests/inventory.json)含 11 组、427 项、5 个官方组与 6 个团队组，是 A 的候选输入，仍须重新核对源码、镜像和 judge；不能把此前错误评论中的类名当成 judge 输出键。

历史重判使用[冻结锁](https://github.com/MindScrath/Focos/blob/50eee125a409207d8842b32d7d8d4264b10370e1/tests/reference.lock.json)中的归档和 SHA256，原始内容不修改。历史记录中的 `execution_valid=true` 不能继承为新报告的验收通过；含上游版权邮箱的原始串口继续使用[外部归档索引](https://github.com/MindScrath/Focos/blob/50eee125a409207d8842b32d7d8d4264b10370e1/tests/results/raw-evidence-2026-11-17.json)，不重新放入源码仓库。

需要 reviewer 决定的是本页的职责边界、验收定义和分期是否可用。所有命令、格式、负例与真实采集仍为待实现/待运行；当前 PR 的文档检查与工程回归记录在 PR 描述，不把方案当成成绩。
