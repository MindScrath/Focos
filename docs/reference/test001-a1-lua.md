# TEST-001 A1：Lua 单组纯解析

本批实现 [解析契约](test001-analysis-contract.md) 的 Lua 子集，只处理内存 bytes，不读文件、不运行 judge、不发布报告。真实语法来自 [A0 样本](test001-a0-lua-samples.md)。其他组、CLI、完整 PROFILE/CRITERIA 和运行包仍未支持。

## 接口与语言

`analyze_lua(raw, libc, selection=None)` 接收不可变 bytes、musl/glibc，以及九个已审 Lua 参数的非空子集；默认全组。selection 不决定顺序，顺序始终从原脚本九项投影。未知/重复参数和非法 libc 属于 ValueError；不接受自定义 PROFILE。此局部计划是解析测试接口，不宣称运行配置已经冻结。

词法阶段按 [#57 的重设计](https://github.com/MindScrath/Focos/pull/57) 实现：原字节先按 LF 分记录，再按原始 UTF-8 边界生成 Unit，最后在 Unit 上生成完整 Token 流。每个 Unit 是带原字节区间的合法标量或单个坏字节；Token 连续覆盖 Unit，控制字符串外层优先。任何颜色删除都发生在完整性校验之后，不能将颜色两侧的坏字节拼成合法字符。

每条 LF 记录保留原始字节、半开区间、1 起行号和识别字符到原始字节的映射。CRLF 仅去识别副本的尾 CR。允许 SGR 仅为 ESC[0m、ESC[31m、ESC[32m、ESC[33m、ESC[39m；不接受其他转义。已登记 CSI 入口为 ESC[，依次消费参数字节 0x30–0x3f、中间字节 0x20–0x2f 和终止字节 0x40–0x7e；其他完整 CSI 拒绝，未完整序列保留到行尾为 opaque。U+009B 按拒绝的 C1 单字符处理，不扩展为第二种 CSI 入口。结构允许 ASCII SP/HT 首尾及字段间隔，完整单行匹配；NBSP、VT、FF、裸 CR、非法 UTF-8 不得产生结果。无末尾 LF 的结构记损坏，不能构成成功结束。本子集对非法 UTF-8、未支持控制符（固定 Unicode 16.0 的 Cc/Cf 范围，除已允许的 ASCII HT）或非 ASCII 空白的整条记录报错，即使它是诊断；普通可打印 Unicode 诊断仍可保留为信息。Cf 范围固定在源码，依据本机 Python 3.14 的 Unicode 16.0 数据核对为 170 个字符；不调用运行时分类，避免 Python 3.11 的 Unicode 14 将新格式控制符当成未分配字符而漏检。

合法结构为组 START/END 和 `testcase GROUP NAME success|fail`；仅 lua 结果可被本接口消费。保留前缀为去掉 ASCII SP/HT 后的 `####`、`testcase`、RUN/FAIL LTP CASE、Summary、passed/failed/broken/skipped/warnings/Total Tests，以及 TPASS/TFAIL/TBROK/TCONF/TWARN。LTP 汇总还登记无冒号的计数前缀：passed/failed/broken/skipped/warnings 后接 ASCII SP/HT 和十进制计数（计数后为 SP/HT 或行尾），例如 `failed  1`；原有冒号形式继续保留。依据[比赛固定源码](https://github.com/oscomp/testsuits-for-oskernel/blob/8b58dd16d26d30f7c74d48d5832d870d3051b703/ltp-full-20240524/lib/tst_test.c#L889-L894)。`failed to open file` 等普通诊断不因此成为汇总。损坏或未支持结构记错误，不降为信息。TINFO/TDEBUG 等普通消息不递归扫描其中的标记；未知诊断保留 opaque 信息，不承诺识别任意自然语言伪造。

损坏结果前缀直接在同一 Unit/Token 流上标记：只从记录开头越过空白、已登记控制字符、坏字节、允许 SGR、完整拒绝 CSI 和完整 OSC，遇到普通文字即停止。OSC 支持 ESC] / U+009D 开始，以 BEL、ESC + 0x5c 或 U+009C 结束；消息体不透明，内部 SGR 不单独识别。未终止 OSC 或未知转义保留到行尾为 opaque，不向内部搜索。坏输入的 text 保留控制内容，以字面 \xNN 显示坏字节；它只供显示，绝不作为前缀或合法结果匹配的输入。此标志只允许生成身份/状态均未知的观察，不修改识别文本，也不能恢复成合法结果。原始记录仍为 INVALID_STRUCTURE，另记 ORDER_UNVERIFIABLE；不从请求中的 selection 猜名称、libc 或成功状态。损坏组标记不因此变成调用，普通消息中的 testcase 不递归识别。

## 状态与结果

组从 idle 经唯一匹配 START 进入 active，再由匹配 END 闭合。错组/错 libc、嵌套、重复或错配标记不改变有效活动组；EOF 活动组记 TRUNCATED。组前/组后结果记归属错误，不能借用请求中的 libc 绑定为有效调用。

结果行是 Lua 的单行调用观察。保留全部观察及尝试，按原位置比较预期顺序，未知/范围外/重复/组外结果不从观察序列过滤。仅组内、当前 libc 的已知参数可归属调用；范围外项仍保留真实身份和结果，但不扩大 selection。结果缺项：合法组已开始为 MISSING，否则 NOT_RUN。重复尝试为 JUDGE_ERROR，保留先前 fail；合法单次 success/fail 分别为 PASS/FAIL，仅表达包装器协议。顺序、边界和词法错误独立保留在 protocol 中，不以单条 PASS 清除。只有 ORDER_MISMATCH 的合法单次结果仍归 invocation，保持 PASS/FAIL；观察上的顺序诊断同时保留，不冒充调用归属错误。ORDER_INCOMPLETE 仅在观察序列是预期序列的严格前缀时生成；较短但错位的序列保留 ORDER_MISMATCH，实际缺项由调用的 MISSING 表达。

返回冻结 dataclass 与 tuple：Unit、Token、Record、Assignment、Observation、InvocationResult、AnalysisResult。Unit/Token 区间完整覆盖记录正文，并校验每个 Unit 可复原对应原字节；显示字符映射仍引用原字节位置。每条记录恰有一个 assignment；错误记录归 error，合法信息归当前组或日志范围。证据只引用同一批 record_id，没有第二次正文扫描。protocol 为 COMPLETE/INCOMPLETE/ERROR；错误优先于不完整，零组或缺顺序尾部也不完整。acceptance 永远 BLOCKED、trust=external_log/unknown，不提供 accepted_pass。没有 JSON/Markdown 渲染或 CLI；这些留到 A2。

## 验证门槛

真实两套组片段为正例；合成反例覆盖 LF/CRLF/SGR、字节位置、非法分隔符/编码/控制序列、跨 LF、消息引用、组前后和错组结果、缺组/缺项/截断、重复与失败后成功、未知/范围外参数、顺序重排和子集投影。每个测试同时检查记录归属、保留事实和协议结论，不能只断言 acceptance=BLOCKED。

词法重设计新增两/三/四字节字符全部内部边界 × 五种 SGR、合法码点边界、坏编码、OSC 内嵌颜色/消息、区间覆盖与混合归属反例。变异检查增加先删 SGR 再解码、UTF-8 ignore/replace、显示文本识别前缀、扫描/删除 OSC 内容、损坏视图删颜色及丢弃未知开始；本批还保留非法结构降信息、丢弃首次失败和忽略顺序；完整契约中的 LTP 孤立汇总、对照 judge 授权、包快照等变异随对应阶段实现，不声称已完成完整 A1。

首次提交的变异结果（各运行当时的 15 个测试，不改期望；后续回归结果见 PR 回复）：

| 删除/替换的不变量 | 触发断言失败数 |
| --- | ---: |
| 含非法字节/未闭合结构的 kind 从 invalid 改为 information | 9 |
| 调用只保留最后一个 attempt | 1 |
| 禁用逐项身份与预期顺序比较 | 1 |
| 保留前缀但不匹配语法的结构全部改为 information | 3 |

正常实现同组测试通过；变异运行均为断言失败而非导入/执行异常。LTP 孤立字段、对照授权、旧运行包实现快照等尚无对应实现，不计为已完成变异。详细日志在本地 target/a1-lua-evidence/，PR 列实际命令与结果。
