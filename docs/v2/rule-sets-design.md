# 规则集引擎设计草案

> **R1 已实现格式与 crate；R2a 已接线 Server／Policy／Control。** `crates/meshway-rules` 与 `tools/meshway-rules-cli` 见 [R1 实测](#r1-实测)；R2a 的实际接口与验收见 [R2a 接线](#r2a-接线)，R3 见 [R3 节点侧出口分割](#r3-节点侧出口分割)，目录主动更新、artifact 回收与签名 held 见 [R2d 收尾](#r2d-收尾)。可用行为、字段与 HTTP 契约的唯一定义是 [Rule Sets](/v2/configuration-reference/rule-sets)、[Routing](/v2/configuration-reference/routing) 与 [Netmap 设计](/v2/node/netmap-design#rule-set-artifacts)；本页其余部分是设计记录，不加入文档导航。出口分割已移到节点（R3）；X6 可信域名关联已接线，见 [Exit 设计 · 可信域名关联](/v2/node/exit-design#trusted-dns-associations)。

## 目标与结论

已定方向（用户决定）：自有二进制规则集格式；**对齐 mihomo**：发布仓库自动把 mihomo meta-rules-dat（domain-list-community／geoip 派生）来源转换后签名发布；sing-box source 转换保留为可选输入格式（自定义 remote 用），不是默认源，不解析 `.srs`；Server 内置 provider 指向它，`geosite:openai`／`geoip:cn` 可直接引用，网络配置可覆盖 provider；inline 与自定义 remote 仍支持，且与发布仓库**共用同一个转换器**。匹配引擎进入 Policy：域名在 DNS 查询时匹配（X6），IP 在首包匹配；支持 exact／suffix／keyword 与 IPv4／IPv6 CIDR，regex 逐条报告并跳过；不做端口／进程匹配。netmap 只带 digest 引用，节点经认证端点按 digest 取一次并缓存。

本稿结论：

1. **格式 `MWRS` v1**：64 字节定长头＋段表；域名 exact／suffix／subdomain 共用一棵按字节反转的 FST，keyword 存排序字面量、加载时建 Aho-Corasick，IP 为合并后的有序区间数组。digest＝整份 artifact 字节的 SHA-256；构建确定性，集合相等即字节相等（spike 中 mihomo YAML 与文本、sing-geosite 与 meta-rules-dat 的 openai 得到同一 digest，arm64／x86_64 一致）。
2. **溯源不进 artifact**。上游 repo／commit／格式／未转换条目报告放在发布目录和 Server 资源记录中；否则上游每次提交都会改 digest，引发无内容变化的全网 netmap 更新。
3. **唯一实现 `crates/meshway-rules`**：格式、builder、loader、matcher、来源转换器。Policy 只用 matcher API；Server、发布流水线（CLI）和节点 Control 复用同一 crate。
4. **规则集内容按 digest 下发**：netmap 的 `ruleSets` 只有 `{digest, bytes, entries}`，替代现在每份 netmap 内联的 JSON 内容；规则集刷新只改变引用。
5. R1 实测（正式 crate）：16 份真实来源与 spike 参考实现 0 差异（3,150 万次域名、3,210 万次 IP 查询），arm64／x86_64 digest 一致；planck 上 `match_domain` 命中 163–261 ns／未命中 55–129 ns、`match_ip` 5–46 ns；打开（含整份 SHA-256 与 FST 结构证明）816 KB 约 4 ms、最大 3.66 MB 约 42 ms，每进程每 digest 一次。详见 [R1 实测](#r1-实测)。
6. spike 实测：现转换器在 8 份受支持格式的真实来源中**整次失败 4 份**；新格式 artifact 为 source 的 22–39 %（域名）／46–51 %（IP），打开 3–5 µs（无 keyword、planck），`match_domain` 命中 125–212 ns／未命中 43–112 ns、`match_ip` 5–43 ns，与参考实现 0 差异；现 Policy 线性匹配同类查询要 0.48–54 ms。详见 [spike 实测](#spike-实测)。

## 现状与问题

| 项 | 现行实现 | 问题（spike 证据） |
| --- | --- | --- |
| 转换 | `apps/meshway-server/src/resources/rulesets.rs::convert`，serde `deny_unknown_fields`，任一条目不可转换即 `conversion_failed` | 上游 sing-geosite 只发布 `.srs`，`sing-box-source` 无法直接消费；解码后的三份 sing-geosite 都含 `domain_regex`，整次失败；meta-rules-dat `geolocation-!cn` 因 2 个 IDNA 不合法名称整次失败 |
| 规模上限 | 源与规范化内容 8 MiB、每网 250,000 条（按 key 计） | blackmatrix7 Advertising 源 9,436,179 B、287,021 条，两项都超 |
| 存储 | 每网 artifact 为 `RuleSetData` JSON | 同内容跨网络重复存储 |
| 下发 | `ProjectedConfiguration.ruleSets` 内联 `RuleSetContent`（全部 CIDR／名称字符串），进入每份 netmap，计入 8 MiB `MAX_NETMAP_BYTES` | geosite-cn（mihomo）JSON 1,636,052 B 乘到每个节点；刷新触发全网重编（审查 H3-5／M10） |
| 匹配 | `meshway-policy/src/compile/plan.rs::plan_match` 逐条 `find`，每次比较都重新规范化 | 现路径每次查询 0.48–54 ms（见[匹配](#匹配)） |
| 出口分割 | `compile/capture.rs::prefixes` 把规则集 CIDR 原样并入，`MAX_CAPTURE_PREFIXES`＝4096 | geoip-cn 原始 8,045／9,609 条、合并后最少 5,790／6,163 个 v4 前缀，引用即 `Budget` |

## 二进制格式

### 布局

所有整数小端；段起点 8 字节对齐；头、段表与各段之外的字节（对齐填充、尾部）必须为 0。

| 偏移 | 字段 | 说明 |
| --- | --- | --- |
| 0 | `magic` | `MWRULES\0` |
| 8 | `major` u16 | 不兼容变更时递增；loader 拒绝未知 major |
| 10 | `minor` u16 | 只增加可选段或可选头字段；当前 0 |
| 12 | `flags` u32 | v1 为 0 |
| 16 | `sectionCount` u32 | ≤ 64；20–23 保留为 0 |
| 24 | `totalLength` u64 | 必须等于实际字节数 |
| 32 | `counts` 6×u32 | exact、suffix、keyword、v4 区间、v6 区间、subdomain |
| 56 | 保留 | 0 |
| 64 | 段表 | 每项 24 B：`kind` u32、`flags` u32（bit0＝required，其余位为 0）、`offset` u64、`length` u64 |

| kind | 段 | 编码 |
| --- | --- | --- |
| 1 | `domains` | `fst` 0.4.7（格式版本 3，crate 版本精确锁定，因为其输出进入 digest）的 Map：key 为规范名称的**字节反转**，value 恰为 1＝exact、2＝suffix、4＝subdomain 之一。一次从尾到头遍历即可在每个标签边界判断 suffix／subdomain，在名称起点判断 exact，O(名称长度)，与条目数无关 |
| 2 | `keywords` | `count` u32、`count+1` 个 u32 偏移（首个为 0、末个为 blob 长度）、字节 blob；字面量严格升序、非空、≤ 253 B 可打印 ASCII 且无大写 |
| 3 | `ipv4` | `count` 个 `(start u32, end u32)` 闭区间，排序、不相交、不相邻 |
| 4 | `ipv6` | `count` 个 `(start u128, end u128)`，同上 |

**为什么是 FST 而非 hash 索引**：spike 同时实现了零拷贝的标签后缀 hash 索引（FNV-1a 增量哈希，开放寻址）。它的命中快约 3–4 倍（planck geosite-cn 37 ns 对 140 ns），但 artifact 大 5.8 倍（4,772,848 B 对 816,448 B）；域名匹配发生在 DNS 查询而非逐包，140 ns 相对 DNS 往返可忽略，而大小直接乘到每个节点的下载与常驻内存，因此 v1 只保留 FST。HashSet 等加载时构建的结构需反序列化（geosite-cn 6.8 ms），不满足“加载不反序列化”。

**keyword 在加载时建自动机**：Aho-Corasick 没有可零拷贝加载的稳定序列化格式；实测 278 个关键字的 DFA 构建 0.60 ms、1,024 个 2.0 ms／1.6 MB 堆（[关键字](#关键字自动机规模)）。v1 每个 artifact 最多 1,024 个关键字，进程内每个 digest 只建一次。

四个 kind 必须各出现一次且标为 required；未知 kind 若标 required 则拒绝，否则忽略。v1 builder 按 1–4 的顺序写四段（空段也写）。

**loader 校验**（唯一入口 `RuleSet::from_vec`，任意字节要么拒绝要么得到查询不会 panic 的句柄）：大小 ≤ 16 MiB；magic、major、保留字节与 `totalLength`；段表在界内、对齐、互不重叠、填充为 0；头 `counts` 不超上限且与各段一致；keyword 表与 IP 区间的排序／不相交／不相邻；整份 SHA-256。`fst` crate 的读取端假定输入可信（畸形字节可 panic），所以 `domains` 由 `meshway-rules` 自己的有界读取器解码，加载时证明：节点从 root 向下首尾相接铺满段（builder 的写出方式）、每个转移指向更低的铺块地址（遍历必终止）、输入为严格升序的名称字节、256 B 输入索引与转移一致、每个 key 的值恰为一个 kind 位、key 总数等于头计数、最长 key ≤ 253 B。该证明对每个节点一次线性遍历，是打开耗时的主要部分（见 [R1 实测](#r1-实测)）。

**IP 为区间而非前缀**：区间二分查找与前缀长度无关；同一数组也是 nftables interval set 的直接输入。需要展示命中条目时，在冷路径把所在区间按规范最小 CIDR 分解给出包含该地址的那一块。

### 规范化与 digest

builder 的输入是转换器产出的条目，输出唯一确定：

- 名称（`meshway_rules::Name::parse`）：`idna::domain_to_ascii`（UTS 46 ToASCII）→小写、去结尾点；label 1–63 字节、全名 ≤ 253、字符 `[a-z0-9-]` 且不以 `-` 开头或结尾，与现 Server `canonical_domain` 逐字相同。重复项由集合去重。
- 与上游的差异：mihomo／sing-box 按字面比较名称，不做 IDNA 校验，也接受 `_`。因此 punycode 标签解码后不是合法 IDNA 的名称（meta-rules-dat `geolocation-!cn` 的 `xn--wcvs22d1m.hk`、`xn--wcvs22d1m.xn--j6w193g`）与含 `_` 的条目在我们这里报告为 `invalidName` 并跳过，本集合是上游的子集。
- keyword（`Keyword::parse`）：小写后 1–253 字节可打印 ASCII（不含空格）；故意比名称宽，查询名可能含 `_`（blackmatrix7 Advertising 有 `_vmind.qqvideo.tc.qq.com`）。
- CIDR（`Cidr::parse`）：`address/bits`，清除主机位（`10.4.3.2/8`→`10.0.0.0/8`），与现 Server 相同；裸地址、zone 不接受。
- 去冗余：包含另一 keyword 的 keyword；包含某 keyword 的名称；`exact(n)`＋`subdomain(n)` 合并为 `suffix(n)`；位于某 suffix 之下（exact／subdomain 含该 suffix 本身）或严格位于某 subdomain 之下的名称均删除。CIDR 合并为区间（相邻也合并）。
- `digest = "sha256:" + hex(SHA-256(artifact 全部字节))`。`bytes`＝artifact 字节数；`entries`＝exact＋suffix＋subdomain＋keyword＋v4 区间＋v6 区间（规范化后计数，不是源条目数）。

去冗余的条目数单独报告（geosite-cn 2,578、blackmatrix7 Global 8,081），不算未转换。R1 比 spike 多删“严格位于 subdomain 之下”的名称：sing-geosite `geolocation-!cn` 的 `ads.yandex`（在 `.yandex` 下）与 `adtrafficquality.google`（在 `.google` 下），这是 16 份中唯一与 spike digest 不同的一份，匹配结果相同。

### 来源溯源与转换报告

溯源与报告**不进入 artifact 和 digest**。它们属于“这份内容从哪来”，由发布目录（provider 集合）或 Server 资源记录（自定义 remote）保存：

| 字段 | 说明 |
| --- | --- |
| `upstream.repo`／`ref`／`commit`／`path` | 上游仓库、分支、提交 SHA 与文件路径（`catalog::Location`） |
| `source.format` | 输入格式（见[转换器](#转换器)） |
| `source.sha256`／`source.bytes` | 原始输入摘要（小写 hex）与大小 |
| `converter` | `version`（`meshway-rules` 版本）、`formatMajor`／`formatMinor` |
| `report.summary.converted` | 接受的源条目按成为的 kind 计数：`exact`、`suffix`、`subdomain`、`keyword`、`cidr`（去冗余前，含重复） |
| `report.summary.redundant` | 接受后因重复或被覆盖而删除的名称／keyword 数（IP 合并不计） |
| `report.summary.unconverted` | 跳过原因→计数，见下表 |
| `report.items` | `{at, reason, entry}`：位置（`/rules/3/domain_regex/0`、`/payload/7`、`line 12`、`/domainSuffix/2`）、原因与原文（截断到 256 字符）；最多 1,000 条，超出只计数 |

| 原因 | 含义 |
| --- | --- |
| `regex` | `domain_regex`／`DOMAIN-REGEX`，v1 不降级 |
| `invalidName`／`invalidKeyword`／`invalidCidr` | 不满足[规范化](#规范化与-digest) |
| `wildcard` | mihomo `*`，或 `+.` 之外的 `+` |
| `mixedRule` | 一条 sing-box 规则同时有名称与 `ip_cidr` 字段（AND），整条的每项都报告 |
| `emptyRule` | sing-box 规则无条件（上游等于匹配全部），按一项报告 |
| `notString` | 列表项不是字符串 |
| `unsupportedField:<name>` | sing-box 规则含 `domain`／`domain_suffix`／`domain_keyword`／`domain_regex`／`ip_cidr` 之外的字段（`invert`、logical 的 `mode` 等；`type: default` 视为无），整条的每项以首个（按名排序）未知字段报告 |
| `classical:<TYPE>` | 无等价的 classical 类型（`IP-ASN`、`PROCESS-NAME` 等），或 `DOMAIN*` 带选项、`IP-CIDR*` 带 `no-resolve` 之外的选项 |

### 上限

| 对象 | 上限 | 依据 |
| --- | --- | --- |
| 单次源下载 | 32 MiB | 实测最大真实源 9.4 MB |
| 单个 artifact | 16 MiB、1,048,576 个名称、1,024 个关键字、各族 262,144 个区间 | 实测最大 3,663,064 B／286,254 个名称／278 个关键字 |
| 每网络绑定 | 不同 digest 合计 64 MiB、最多 128 个 | **按 digest 去重**计，多个 key 指向同一 digest 只计一次 |
| 节点 | 当前 netmap 引用的 artifact 合计 ≤ 64 MiB | netmap 声明 `bytes`，Control 下载前检查 |

超限仍为配置诊断 `rule_set_budget`，`params` 改为 `maxBytes`／`maxEntries`／`maxKeywords`／`maxRanges`／`maxSets`，不截断内容。

### 兼容策略

- 未知 major 拒绝；同 major 未知的 required 段拒绝，未知的可选段忽略。
- 新增匹配语义（例如 regex）只能作为新的 required 段或新 major，旧 Node 必须拒绝而不能忽略后放宽或收窄匹配。
- builder 只写当前版本；Server 可以同时读取旧 minor。改变规范化规则即使不改布局也要升 minor 并记录在 `converter` 中，因为同一来源会得到新 digest。
- 格式版本出现在 netmap 引用中（`format`），Node 在下载前即可判定能否使用。

## 匹配语义

| kind | 匹配 | 来源 |
| --- | --- | --- |
| exact | 名称完全相等 | sing-box `domain`、mihomo 裸名称、classical `DOMAIN` |
| suffix | 名称本身或任一标签边界上的子名称 | sing-box `domain_suffix`、mihomo `+.`、classical `DOMAIN-SUFFIX` |
| subdomain | 只匹配子名称，不含本身 | sing-box `.example.com`、mihomo `.example.com` |
| keyword | 规范名称包含该子串 | sing-box `domain_keyword`、classical `DOMAIN-KEYWORD` |
| ipv4／ipv6 | 目的地址落在区间内 | `ip_cidr`、mihomo ipcidr、classical `IP-CIDR`／`IP-CIDR6` |

- 集合内所有条目取并集；一条 sing-box 规则内名称与 IP 字段为 AND，不能表达，整条报告为 `mixedRule`。
- regex（用户决定）：v1 逐条报告并跳过，不做字面量降级（真实数据中 160 条可降级的为 0）；后续若支持只能按[兼容策略](#兼容策略)作为新的 required 段。跳过意味着本集合是上游语义的子集，`RuleSetView` 必须显示 `unconverted.regex` 计数。
- 查询输入为小写 ASCII、无结尾点的名称（如 `Name::as_str()`），按字节比较，不在 matcher 内再规范化；其他写法不会命中。规范名称不含 `_`，但 matcher 本身接受任意字节，`_dmarc.example.com` 可以命中 suffix `example.com` 与 keyword；R2a 已决：Policy 的 `egress_rule_for_name` 只做 ASCII 小写与去一个结尾点后交给 matcher，`Name::parse` 只用于配置与转换。
- `match_domain(&str) -> Option<DomainHit>`：`Exact`、`Suffix { len }`（命中项是查询名的尾部 `len` 字节）、`Subdomain { len }`、`Keyword { index }`（`RuleSet::keyword(index)` 给出字面量）。suffix／subdomain 从最短的尾部开始尝试，exact 只在全名处，keyword 最后。由此可在不额外存储的情况下给出 `RuleSetMatch.entry`。
- `match_ip(IpAddr) -> bool`：IPv4 只查 v4 区间、IPv6 只查 v6 区间（`::ffff:a.b.c.d` 不当作 IPv4）。`ipv4_ranges()`／`ipv6_ranges()` 按升序给出合并后的闭区间（`RangeInclusive<Ipv4Addr>`／`<Ipv6Addr>`），即 nftables interval set 的输入；需要展示命中区间的 CIDR 时由调用方在冷路径分解。
- `domains()` 按反转名称顺序列出全部 exact／suffix／subdomain，`keywords()` 列出字面量；`counts()`、`digest()`、`sections()`、`as_bytes()` 给出头与段信息。

## crate 与 owner

| 组件 | 持有 | 不得 |
| --- | --- | --- |
| `crates/meshway-rules` | 格式常量、builder、loader、matcher、名称／keyword／CIDR 规范化、全部来源转换器与报告类型、签名目录类型与签名／验签 | 依赖 Server／Policy／网络 I/O；第二套匹配 |
| `meshway-policy` | 首个匹配规则判定；只调用 `RuleSet::match_domain`／`match_ip`／迭代器，以及从路由规则内联列表构建内存 `RuleSet` 的 `RuleSet::from_inline` | 读取文件、下载、解析来源格式；自写字符串比较 |
| Server `resources/` | 下载、provider 目录验签、转换、artifact 存储（全局内容寻址）、网络绑定、`GET /rule-sets`、`artifacts/{digest}` 授权 | 在 netmap 内联内容 |
| Node `meshway-control` | 按 netmap 引用取 artifact、校验 digest、私有原子缓存与清理 | 解析或重建 artifact 内容 |
| `meshway-engine` | 把已加载的 `RuleSet` 句柄连同 netmap 交给 Policy／组件 | 持有第二份授权状态 |
| 发布流水线 CLI（`tools/meshway-rules-cli`，bin `meshway-rules`） | 文件 I/O 与参数，调用 `meshway-rules` 转换、组装与签名目录 | 自带转换、校验或签名逻辑 |

feature 划分：默认含规范化、builder、loader、matcher 与 inline 转换（Policy、Engine 用）；`convert` 含来源解析（`serde_json`、`serde_yaml_ng`；Server、CLI 用）；`catalog` 含 `index.json` 类型、组装、签名与验签（`ed25519-dalek`、`chrono`；Server、CLI 用，蕴含 `convert`）。目录签名钥只签规则目录，与 netmap 租约签发无关，不构成第二份授权状态。

`inline` 模块：`Inline { domain, domain_suffix, domain_subdomain, ip_cidr }`（`&[String]`）。`inline::convert` 返回带报告的 `Conversion`（位置 `/<字段>/<序号>`）；`RuleSet::from_inline` 要求零跳过且非空，否则返回首个问题。

**只有堆读取，零 `unsafe`**：artifact 一律读入堆缓冲再经 `RuleSet::from_vec` 校验；规则集代码不含 `unsafe`（workspace 唯一的例外是 `meshway-netns` 的一次审计 `getsockopt`，与规则集无关）。曾评估的 mmap 后端（`memmap2::Mmap::map` 是 `unsafe`，文件被截断或改写即未定义行为／SIGBUS，安全 API 无法让调用者证明该前提）已删除：R1 实测打开 4 ms 几乎全是结构校验与 digest，映射省下的读取可以忽略。Control 从私有目录按 digest 打开 0400 文件、有界读取（≤ 16 MiB）后与网络下载走同一校验；调用方比较 `digest()` 与期望值。

**每进程每 digest 一个句柄**：Server 计算所有节点的路径检测时共享同一 `Arc<RuleSet>`，每个 digest 只有一份堆副本（[内存](#内存-1-000-个句柄)中的 mmap 列为 R1 评估数据，现已不用）。

### 转换器

| `format` | 输入 |
| --- | --- |
| `sing-box-source` | sing-box rule-set source JSON v1–v3（顶层只允许 `version`、`rules`）；字段可为字符串或数组（sing-box listable）；`domain_suffix` 以 `.` 开头为 subdomain。可选格式，不是默认源；不解析 `.srs` |
| `mihomo-domain`／`mihomo-ipcidr` | mihomo YAML `payload`（顶层只允许 `payload`）；`+.x` suffix、`.x` subdomain、裸名 exact |
| `mihomo-domain-text`／`mihomo-ipcidr-text` | mihomo `format: text`（meta-rules-dat `.list`）：每行一项，去首尾空白，跳过空行与 `#` 行 |
| `classical` | mihomo／Clash classical 文本 `TYPE,value[,option]`：`DOMAIN`、`DOMAIN-SUFFIX`、`DOMAIN-KEYWORD`、`DOMAIN-REGEX`（跳过）、`IP-CIDR`／`IP-CIDR6`（`no-resolve`） |
| `meshway` | 本格式；按同一 builder 重建，字节必须不变（非规范 artifact 为 `invalid_source`） |

转换只在文档无法解析（`InvalidSource`→`invalid_source`）、源超过 32 MiB（`TooLarge`→`too_large`）、结果超出 artifact 上限（`Budget`→`rule_set_budget`）或无任何条目可转换（`Empty`→`conversion_failed`）时整次失败；单条无法转换只进入报告。对现 Server 转换器的语义变化（均已有测试 `crates/meshway-rules/tests/convert.rs`）：未知字段／regex／通配／混合规则由整次失败改为逐条报告；`domain_keyword` 与 `.example.com` 由拒绝改为 keyword／subdomain；被 suffix 覆盖的同名 exact 不再单独保存。

## 发布仓库

**已运行**（R2b）：[`meshway-org/meshway-rules`](https://github.com/meshway-org/meshway-rules)（公开）发布 meta-rules-dat 全部 geosite／geoip 类别；当前目录由 Actions 签发。仓库 README 是对外的格式与验证说明。

| 项 | 实际形态 |
| --- | --- |
| 分支 | `main`：集合清单 `sources.toml`、流水线 `scripts/publish.py`（含单测）与 `scripts/push-dist.sh`、`.github/workflows/publish.yml`；`dist`：发布产物，每次发布是一个无父提交并强制推送，同时打 tag `catalog-<sequence>`、建 GitHub Release（`index.json`、`.sig`、`status.json` 与发布说明）供审计 |
| 目录 URL | `https://raw.githubusercontent.com/meshway-org/meshway-rules/dist/index.json`（及 `index.json.sig`、`artifacts/sha256-<hex>.mwrs`），raw URL 不重定向 |
| 集合 | `meta-geosite/<name>` ← `geo/geosite/classical/<name>.list`，`meta-geoip/<name>` ← `geo/geoip/classical/<name>.list`（MetaCubeX/meta-rules-dat `meta` 分支），格式 `classical`；每个上游文件一项，全量 |
| 频率 | 每天 03:17 UTC cron＋手动 dispatch；输入（上游 commit、`sources.toml` 的 SHA-256、`meshway-rules --version`）与 `dist/status.json` 记录的完全相同则不发布 |
| 转换器 | 本仓库 Release `cli-v<版本>` 的 `meshway-rules-x86_64-unknown-linux-musl.tar.gz`；`publish.yml` 以常量 `MESHWAY_RULES_CLI_VERSION`（tag）与 `MESHWAY_RULES_CLI_SHA256` 匿名下载并校验，不编译、不需要外部 token；版本记入目录 `converter` |
| 签名钥 | environment `publish`（只允许 `main`）的 secret `MESHWAY_RULES_SIGNING_KEY`（64 位 hex 种子），只在签名步骤写入 0600 临时文件；公钥见下 |

选 classical 而非 `geo/geosite/*.list`（`mihomo-domain-text`）：domain 文本由上游生成时静默丢弃 `DOMAIN-KEYWORD` 与 `DOMAIN-REGEX`，classical 保留 keyword（全量 16 条）并让 regex 逐条进入报告（394 条）；IP 类别两种编码得到相同 digest。sing-geosite／sing-geoip 上游只发布 `.srs`，不解析，因此不发布。

**转换器交付**：主仓库私有，发布仓库的 Action 不读取它。`build.yml` 与 Node 二进制一起交叉编译 `meshway-rules-cli`；`release.yml` 的 `rules-cli` job（`environment: Production`）在 tag `v*` 发版时核对 `meshway-rules --version` 等于 tag，打包 x86_64 musl 归档与 `SHA256SUMS`，以 `gh release create cli-<tag>` 推到 `meshway-org/meshway-rules`；令牌为 Production 的 secret `MESHWAY_RULES_PUSH_TOKEN`（fine-grained PAT：resource owner `meshway-org`、仅 `meshway-rules`、Contents read/write），缺失时该 job 失败。之后在 `publish.yml` 中把两个常量改为新 tag 与归档 SHA-256。首个 `cli-v0.1.0-alpha.3` 由 `4503138a` 在 planck 以 `cargo zigbuild --release --locked` 手动构建发布（SHA-256 `81d0066f…9603`）。

流水线（`scripts/publish.py build`，之后由 workflow 调用 CLI）：

1. `git ls-remote` 取 `meta` 当前 commit，`git fetch --depth 1 <commit>` 后只检出 `sources.toml` 中的路径。
2. 每个上游文件运行 `meshway-rules convert --format classical`，并行；名称不合 `[a-z0-9][a-z0-9!@._+-]{0,127}` 的文件不转换（当前没有）。
3. hold（脚本实现，CLI 不含）：对上一份目录中已有的集合，本次转换失败，或规范 `entries` 少于上一版的一半，即沿用上一版 `SetRecord` 与 artifact，并在记录上写 `held`；首次出现的 hold 开 issue。从未发布过的集合转换失败则跳过并记入 `status.json.skipped`。上游删除的文件从目录移除。条目骤减属实时，手动 dispatch 并在输入 `accept` 列出集合名即发布新内容；转换失败的 hold 不能接受。
4. 组 `sets/<collection>/<name>.json`（`upstream` repo／ref／commit／path＋CLI 的 ConversionRecord），把 artifact 移为 `artifacts/sha256-<hex>.mwrs`；复制上一份 `index.json` 到输出，使 `meshway-rules index` 的 sequence 接续。
5. `meshway-rules index --sign-key`、`meshway-rules verify --public-key <CI 钥>`。
6. 保留：`history/<sequence>.json` 存前 6 份目录原文，`artifacts/` 保留当前与这 6 份引用的全部 artifact（共 7 份目录）；更早的 artifact 仍在对应 tag。
7. `scripts/push-dist.sh`：以 `--force-with-lease` 指向构建开始时的 `dist` commit（期间 `dist` 变化即失败）原子推送 `dist` 与 tag，再建 Release。

hold 的权威记录是签名目录：held 集合的 `SetRecord.held`／`CatalogSet.held` 为 `{since, reason}`，`reason` 为 `conversion failed: <错误>` 或 `entries dropped from <旧> to <新>`，`since` 为首次 hold 时间，下一次构建从上一份 `index.json` 读取。`dist/status.json` 的 `held: { "<collection>/<name>": { since, reason, digest } }` 只是给人看的副本。

上游一致规则（R2d 核实，写入发布仓库 README）：`upstreams[]` 列出目录内容实际来自的 (repo, ref, commit)，`Catalog::assemble` 要求同一 (repo, ref) 只有一个 commit。分支 ref 只对应本次转换的 commit；held 集合的内容来自更早的 commit，它的 `upstream.ref` 写成该 commit 本身（git 可按 commit 直接取回），所以 `upstreams[]` 多一项 `ref` 等于 `commit` 的条目，多个从同一 commit hold 的集合共用这一项。两者不冲突：分支 ref 与 commit ref 是不同的 (repo, ref) 键，同一 commit ref 只能对应它自己。以真实 CLI（`0.1.0-alpha.4`）在本地上游上构建“发布→hold（条目骤减）→持续 hold→accept”四份目录，`index`／`verify` 均通过，`since` 跨构建不变，accept 后 `held` 与 commit ref 一起消失；目录原文在私有证据 `.meshway/rules-publish/r2d-held-e2e/`。

| 公钥 | 用途 |
| --- | --- |
| `ed25519:68dfc59cf585e0320129db7f1716b0dbb3d13c462fcefee7151d4b51ae1fc45c` | CI 钥，`publish` environment secret |

签名对象为 `"meshway-rules catalog v1\0"` 前缀＋`index.json` 原始字节（域分隔，目录钥不会被诱导签其他内容），`index.json.sig` 为 `{"keyId": "ed25519:<64 hex 公钥>", "signature": "<128 hex>"}`；验签用 `verify_strict`，`keyId` 必须是受信公钥（当前只有 CI 钥；轮换＝发布钉新公钥的 Server 版本）。Server 内置这一把公钥（`meshway_configuration::providers::BUILTIN_PUBLIC_KEYS`）；CI 钥泄露时换钥并发布钉入新公钥的 Server 版本。目录签名钥只签规则目录，与 netmap 租约签发无关。

**首发**：`catalog-1` 以本地流水线生成（相同脚本，macOS arm64 CLI；planck x86_64 musl CLI 对全部集合 digest 相同），CI 钥签名后推送；随后 Actions 首次运行因输入未变不发布，手动 dispatch（`accept`）签发内容相同的 `catalog-2`，打通 Actions 下载、签名、验证、推送与 Release。上游 `83c8208b`：2,177 个上游文件，目录 2,168 项；9 项只含 `DOMAIN-REGEX`（`conversion failed: No source item could be converted.`）未发布，0 项 hold；59 项有逐条跳过（`regex` 394、`invalidName` 8）；2,097 个不同 artifact 共 13,107,552 B，不含 `index.json` 的树 15,154,676 B，`index.json` 2,047,159 B；本地构建 18.7 s、Actions 9.7 s。digest 清单在私有证据 `.meshway/rules-publish/catalog-1/digests.txt`。

**Server 怎样取与钉钥**：Server 启动后及按 provider 间隔主动取 `https://raw.githubusercontent.com/meshway-org/meshway-rules/dist/index.json` 的目录与签名（不需要先引用集合，见 [Rule Sets · 目录更新](/v2/configuration-reference/rule-sets#catalog-refresh)），用内置公钥（源码常量，可由部署设置追加或替换，网络管理员不能改内置 provider 的信任根）验签，拒绝 `sequence` 小于该 provider 上次接受值的目录；再按 digest 取 `artifacts/sha256-<hex>.mwrs`，打开后比较 `digest()` 并以 `CatalogSet::check` 核对目录项。GitHub Release 下载会 302 重定向，而现行下载器拒绝重定向，所以不用 Release 作为 Server 来源。

**`dist` 目录与 CLI**：

```text
index.json  index.json.sig
artifacts/sha256-<hex>.mwrs        不可变 artifact
sets/<collection>/<name>.json      SetRecord：{ upstream, conversion }，含完整报告
history/<sequence>.json            前 6 份目录原文（保留其 artifact）
status.json                        流水线输入、held 副本、skipped（不签名）
```

集合名 `<collection>` 为 `[a-z0-9][a-z0-9-]{0,31}`，`<name>` 为 `[a-z0-9][a-z0-9!@._+-]{0,127}`。

```sh
meshway-rules convert --format classical geo/geosite/classical/openai.list -o openai.mwrs > conversion.json
#   stdout：ConversionRecord { digest, bytes, entries, format, counts, source, report }
#   流水线把它与 upstream 组成 sets/meta-geosite/openai.json，并把 artifact 移为 artifacts/sha256-<hex>.mwrs
meshway-rules index --sign-key ci.key dist -o dist/index.json      # 写 index.json 与 index.json.sig
meshway-rules verify --public-key ed25519:<hex> dist/index.json dist
meshway-rules inspect artifacts/sha256-<hex>.mwrs                  # 头、段表、计数、digest
meshway-rules match artifacts/sha256-<hex>.mwrs api.openai.com     # 或 IP
```

输出均为 stdout JSON，日志在 stderr（`--log-filter`／`--log-format`）。

**失败行为**：目录取不到、验签失败、sequence 回退或 artifact 校验失败都保留上一份已验证内容，`problem` 记录原因，不发布半份；目录超过 7 天未前进时 `RuleSetView` 报告 `stale`，但不使执行失败。首次没有任何可用内容时与现有 remote 相同：`not_ready`，引用它的规则不可执行。

**held**：`CatalogSet`／`SetRecord` 的可选 `held: {since, reason}`（省略即未 hold）由发布流水线写入（R2d，`meshway-org/meshway-rules` `379bdb4`），Server 在 `RuleSetCatalogSet.held` 与 `RuleSetView.held` 原样透出。

## 配置面

### `ruleSetProviders`

<!-- 草案字段，不是当前可提交 JSON -->
```json
{
  "ruleSetProviders": {
    "geosite": {
      "catalog": "https://raw.githubusercontent.com/meshway-org/meshway-rules/dist/index.json",
      "collection": "meta-geosite",
      "publicKeys": ["ed25519:<hex>"],
      "updateInterval": "24h"
    },
    "corp": {
      "catalog": "https://rules.corp.example/index.json",
      "collection": "corp",
      "publicKeys": ["ed25519:<hex>"]
    },
    "geoip": null
  }
}
```

| 字段 | 说明 |
| --- | --- |
| key | provider 名，`[a-z][a-z0-9-]{0,31}` |
| `catalog` | HTTPS 目录 URL，规则同现有 remote URL |
| `collection` | 目录内集合前缀 |
| `publicKeys` | 1–4 个 ed25519 公钥，`ed25519:<64 位小写 hex>`（`catalog::PublicKey`）；覆盖内置 provider 时必须给出 |
| `updateInterval` | 默认 `24h`，调度、下界与退避同现有 remote |
| 值为 `null` | 禁用同名内置 provider |

- 内置 provider（用户决定对齐 mihomo）：`geosite`→ `meta-geosite`、`geoip`→ `meta-geoip`，目录均为 `https://raw.githubusercontent.com/meshway-org/meshway-rules/dist/index.json`；不内置 sing-box 系 provider。网络配置中出现同名 key 时**整体替换**内置定义，不逐字段合并。
- 引用：`match.ruleSets` 中含 `:` 的项是 `provider:name`（第一个 `:` 前为 provider）；不含 `:` 的项仍是本网络 `ruleSets` 的 key。本网络 key 从此不得含 `:`。
- 展开：`geosite:openai` 等价于一个隐式 remote `{ type: "remote", provider: "geosite", name: "openai", format: "meshway" }`，URL 由目录解析；它不写回配置原文与规范值，只出现在资源视图与 netmap 引用中。
- 配置校验：未知 provider、目录中不存在的 name 分别报 `unknown_rule_set_provider`／`unknown_rule_set`；目录尚未取得时与远端准备相同，进入 `PreparationKind::RuleSetContent`。

### `GET /rule-sets` 呈现

每个被引用的 `provider:name` 作为一项出现，`key` 即 `geosite:openai`；没有被引用的目录项不列出。`RuleSetView` 增加：

| 字段 | 说明 |
| --- | --- |
| `implicit` | `true` 表示来自 provider 引用 |
| `provider` | `{ name, catalog, collection, sequence }`，sequence 为该 artifact 所在目录序号 |
| `source` | [溯源字段](#来源溯源与转换报告)中的 repo／commit／path／format |
| `conversion` | `converted`、`redundant`、`unconverted`（原因→计数），完整清单走发布仓库报告或 `GET /rule-sets/{key}/report` |

### 与 inline／remote 的关系

- inline 字段（用户决定暴露 subdomain）：现有 `domain`（exact）、`domainSuffix`（suffix）、`ipCIDR` 之外新增 `domainSubdomain`（只匹配子名称，不含本身；对应 sing-box／mihomo 的 `.example.com`），默认 `[]`，规范化同其他名称字段；四项不能都为空。R2a 接入配置类型。
- inline：配置原文不变；Server 用同一 builder 生成 artifact，与 remote 一样按 digest 进入 netmap，节点不再在投影中收到 inline 列表。
- remote：`format` 增加上表新格式；转换、刷新、幂等、回滚语义不变，只把产物从 JSON 改为 `MWRS`。
- routing 规则自身的 `match.domain`／`domainSuffix`／`ipCIDR` 仍在投影中；Policy 以 `RuleSet::from_inline` 构建内存句柄并按 netmap serial 缓存，与规则集走同一 matcher。

## 下发面

### netmap 字段

`ProjectedConfiguration.ruleSets` 从 `key → RuleSetContent` 改为：

<!-- 草案字段，不是当前可提交 JSON -->
```json
{
  "ruleSets": {
    "geosite:openai": { "digest": "sha256:44ab0c1a…", "format": 1, "bytes": 592, "entries": 22 },
    "office": { "digest": "sha256:…", "format": 1, "bytes": 584, "entries": 3 }
  }
}
```

- 引用进入 netmap content digest；投影中删除 `configuration.ruleSets` 的定义（节点不需要 URL，同时修复 M10 的披露）。
- 投影只保留被保留路由规则引用的 key（现规则不变）。
- `compilerVersion` 升为 `meshway-netmap-2`；delta 对 `ruleSets` 按 key 增删。
- 规则集刷新只改变受影响节点 netmap 中一个约 120 B 的引用，Server 不需要重新投影；仍需为每个受影响节点递增 serial 并重签租约。

### `GET /agent-api/v1/artifacts/{digest}`

| 项 | 行为 |
| --- | --- |
| 认证 | 与 `sync` 相同的节点凭据；不能用用户凭据 |
| 授权 | digest 必须出现在 Server 为该节点签发且租约未到期的某份 netmap 中；否则 404，不区分“不存在”与“无权” |
| 响应 | `200 application/vnd.meshway.rules.v1`，原始字节；`ETag: "sha256:<hex>"`；`Cache-Control: private, max-age=31536000, immutable` |
| 节点校验 | 长度等于 netmap `bytes`，SHA-256 等于 digest，loader 结构检查通过；任一失败丢弃并按退避重试 |
| 并发 | 每节点同时最多 2 个下载；Server 端与 sync 共用连接预算 |

Control 把 artifact 写入私有目录 `artifacts/sha256-<hex>.mwrs`（0400、原子 rename），与 `netmap.json` 一起作为离线状态；只保留当前及仍在租约内的 netmap 引用的文件。重启离线载入时，netmap 与其引用的 artifact 必须同时可读并通过 digest 校验。

### 缺 artifact 时 fail closed

新 netmap **立即应用**，不等待 artifact：撤销与收紧不能被下载延迟阻塞。引用尚未取得或校验失败的规则集的规则按现有 `UnavailableDependency` 处理——first-match 在该规则停止，结果为不可用，不跳到后续规则、不当作空集合、不 direct；出口捕获保持全范围捕获／sink 边界，不发布 direct 例外。Status 报告缺失的 digest 与最后错误。

### 1,000 节点共享一个 artifact

以 mihomo geosite-cn 为例，数字由实测大小计算：

| 项 | 现行（内联 JSON） | 按 digest |
| --- | --- | --- |
| Server 存储 | 每份 netmap 1,636,052 B，1,000 份约 1.64 GB | 一份 816,448 B（全局内容寻址，跨网络去重） |
| netmap 每份增量 | 1.64 MB，计入 8 MiB 上限 | 约 120 B |
| 内容变化时传输 | 1,000 × 1.64 MB 的 netmap 块 | 1,000 × 816 KB 一次（gzip 后 594 KB）；netmap 只变引用 |
| 节点内存 | 解析后的字符串列表 | 一份 816 KB 堆副本 |
| Server 内存 | 每份投影各一份 | 每 digest 一个句柄 |

授权检查需要“节点当前租约内 netmap 是否引用该 digest”的索引：Server 在提交 netmap 时同事务写 `(network, node, serial) → digest` 引用表，artifact 请求只做一次索引查询，不解密 netmap。

### 与共享 peer 段的关系

`artifacts/{digest}` 只依赖“内容寻址＋由已签名 netmap 引用授权”，不依赖内容是规则集。“1,000 节点全互访共享 peer 段”可以复用同一端点与缓存（另加 `kind`），但 peer 段的切分与 delta 语义不在本稿内定义。

## Policy 接口与 X6

| 时机 | 调用 | 结果 |
| --- | --- | --- |
| DNS 查询（X6） | DNS owner 规范化查询名一次 → Policy `egress_rule_for_name(&Name)`；Policy 对有效规则按 first-match 依次调用各规则引用集合的 `match_domain` | `Option<RuleIndex>`＋`RuleSetMatch { key, entry }` |
| 首包 | Policy `egress_rule_for_ip(addr)` → 各集合 `match_ip` | 同上 |
| 路径检测 | 同一 `plan_match`，替换现线性比较 | 同上 |
| 配置校验 | `shadowed_rule`、`exit_outside_access` 通过 matcher 迭代器读取 artifact 名称 | 诊断 |

- `match_domain(name) -> Option<rule index>` 的“rule index”属于 Policy；`meshway-rules` 只回答集合成员关系与命中项。
- 同一 IP 被多个名称解析且对应不相容动作时，沿用 [Exit 设计 · DNS 路径与可信域名上下文](/v2/node/exit-design#dns-路径与可信域名上下文) 与 [Routing](/v2/configuration-reference/routing#direct-unmanaged) 的既有决定：拒绝／不可执行，不选更宽路径；域名授权仍是解析地址集合。名称规则只对可信关联的地址生效（X6 已交付）：出口 served 路径与 Server 校验都调用 `egress_rule_for_name`，节点判定以 `decide_with` 对“地址或其关联名称”first-match。
- 出口分割：IP 规则集不展开为字符串 CIDR。节点用 Policy 从投影规则与 artifact 的合并区间（`ipv4_ranges()`／`ipv6_ranges()`）做 first-match 区间分割并安装为 nftables interval 集合，netmap 只带有序“规则→动作”（R3，[R3 节点侧出口分割](#r3-节点侧出口分割)）。

## R1 实测

正式 crate（源码 `83adf0f9`），Apple M3 Pro／macOS 27.0 与 AMD Ryzen 9 9950X／Linux 6.8（planck，`taskset -c 4`），rustc 1.98.0，workspace release profile（fat LTO）。输入为 R0 的 16 份私有数据（`.meshway/rules-spike/data.sha256`）；原始输出在 `.meshway/rules-spike/r1/{macos-m3pro,planck-9950x}/`（`digests.txt`、`perf.json`、`artifacts/`、`r1diff.jsonl`），fuzz 日志在 `.meshway/rules-spike/r1/fuzz/`。

```sh
# 16 份转换（CLI），两台主机各跑一次后比较 digests.txt
.meshway/rules-spike/r1/convert-all.sh target/release/meshway-rules .meshway/rules-spike <out>/artifacts > <out>/digests.txt
# 与 spike 参考实现差分（spike crate 已删除；r1diff 在历史提交 4503138a 的工作树中运行）
git worktree add .worktree/rules-r1 4503138a
mise exec -- cargo build --release --no-default-features --manifest-path .worktree/rules-r1/tools/meshway-rules-spike/Cargo.toml
.worktree/rules-r1/tools/meshway-rules-spike/target/release/meshway-rules-spike r1diff .meshway/rules-spike 1000000
# 打开与匹配耗时（每集合 3 轮 × 每类 100 万次查询、200 次打开取 p50）
mise run perf:rules -- <artifacts...>
# 长时间 fuzz：溢出检查与 debug 断言打开
CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS=true CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true PROPTEST_CASES=400000 \
  mise exec -- cargo test --release -p meshway-rules --test fuzz
```

**差分**：16 份全部 0 差异。查询包括每个规范名称本身、其随机子名称、无标签边界的前缀拼接与父名称、每个 keyword 及包含它的名称，再加每份 100 万个随机名称与其规范名称下的随机子名称（合计 31,494,338 次域名查询）；IP 为每个区间的两端及两端外一位，再加每族 100 万个随机地址（合计 32,079,864 次）。跳过计数与 spike 逐份相同。15 份 digest 与 spike 逐字节相同；`sing-geosite geolocation-!cn` 因多删两条严格位于 subdomain 下的 suffix 而不同（见[规范化](#规范化与-digest)）。`perf:rules` 另以 artifact 自身枚举构建的朴素匹配器核对全部计时查询，两台主机 0 差异。

**跨架构 digest**：CLI 在 arm64（macOS）与 x86_64（planck）上转换 16 份，`digest`／`bytes`／`entries`／跳过原因逐行相同；mihomo YAML 与 `.list`、sing-geosite 与 meta 的 `openai` 仍各为同一 digest（`openai` 为 `sha256:44ab0c1a4c37…`）。

**fuzz**：`tests/fuzz.rs` 以 proptest 生成任意字节（≤ 2 KiB）以及对真实构建 artifact 的 1–5 次变异（改字节、异或、截断、追加、覆写头与段表字段，可选修正 `totalLength`），对能打开的句柄执行查询与全部迭代器。macOS 上 release＋溢出检查／debug 断言，`PROPTEST_CASES=400000` 两个测试各 40 万例，用时 147.5 s，0 panic；其中 8,192–16,383 份变异 artifact 通过了 loader 并被完整调用。常规 `mise run check` 用默认 256 例。

**耗时**：planck／macOS（v4、v6 列为 planck，括号内为 macOS），3 轮中位数。打开包含读取（堆）或映射、全部结构校验、FST 证明、keyword 自动机构建与整份 SHA-256；每进程每 digest 只需一次。

| 集合 | 字节 | 读入堆＋打开 µs | mmap＋打开 µs | 其中 SHA-256 µs | 域名命中 ns | 域名未命中 ns | v4 命中／未命中 ns | v6 命中／未命中 ns |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| sing-geosite `cn` | 50,184 | 308.7／362.5 | 310.8／368.0 | 18.5／103.3 | 163.5／167.2 | 55.5／58.6 | — | — |
| meta `geosite/cn` | 816,448 | 3,958／5,141 | 3,959／5,101 | 303.4／1,688 | 172.7／204.4 | 71.9／80.8 | — | — |
| sing-geosite `geolocation-!cn` | 205,528 | 1,491／1,659 | 1,492／1,655 | 76.7／422.5 | 190.3／200.9 | 73.8／85.8 | — | — |
| sing-geoip `cn` | 98,632 | 40.2／221.8 | 43.2／226.5 | 35.7／202.9 | — | — | 9.2／9.3（12.3／12.3） | 42.2／6.1（13.1／11.8） |
| meta `geoip/cn` | 107,088 | 44.3／239.5 | 47.4／244.0 | 39.4／220.4 | — | — | 9.3／9.3（12.2／12.2） | 45.5／6.4（14.5／12.7） |
| bm7 `Global`（36 keyword） | 239,208 | 1,767／1,979 | 1,765／1,979 | 88.5／492.4 | 182.3／197.4 | 129.4／159.9 | 4.8／4.8（5.7／5.8） | 11.3／3.7（2.6／2.1） |
| bm7 `Advertising`（278 keyword） | 3,663,064 | 42,389／42,541 | 42,310／40,911 | 1,337／7,592 | 261.0／274.5 | 112.9／121.2 | 6.1／6.1（7.8／7.7） | 9.5／3.7（2.6／2.1） |

- `match_domain` 比 spike（命中 125–212 ns）慢约 20–30 %，来自有界读取器的逐次边界检查；仍远低于 DNS 往返，且比现 Policy 线性路径（0.48–54 ms）快 3–5 个数量级。
- 打开比 spike 的 3–5 µs 慢两到三个数量级，是有意的：spike 只做头部检查并依赖会在畸形输入上 panic 的 `fst` 读取器；R1 在打开时证明整个 FST，成本与节点数线性相关（Advertising 约 160 万个节点，planck 约 41 ms，SHA-256 只占 1.3 ms）。heap 与 mmap 打开时间相同，区别只在常驻内存（[内存](#内存-1-000-个句柄)）。

## spike 实测

源码已从活跃树删除，保存在历史提交 `a2891655`（R0 测量）与 `4503138a`（含 R1 `r1diff`）的 `tools/meshway-rules-spike/`：独立 `Cargo.toml`（自带 `[workspace]`、`Cargo.lock`、`target/`），不是 workspace 成员。以下命令在该提交的工作树中运行（`git worktree add .worktree/rules-r0 a2891655`；`tools/` 路径相对该工作树，私有数据 `.meshway/rules-spike` 按主仓库路径传入）。`server` feature（默认开）链接真实的 `meshway_server::resources::convert` 与 `meshway_policy::domain_{name,suffix}_matches` 测现行实现。

```sh
# 输入（私有，不提交），按固定上游 commit 下载；sing-box 1.14.3 仅用于把 .srs 解码成 JSON
tools/meshway-rules-spike/fetch.sh .meshway/rules-spike .meshway/rules-spike/bin/sing-box-1.14.3-darwin-arm64/sing-box
# 全部测量：coverage、7 个集合 × 3 轮 bench（各 100 万次命中／未命中）、内存、压缩大小
mise exec -- tools/meshway-rules-spike/run.sh .meshway/rules-spike macos-m3pro
PATH=$HOME/.cargo/bin:$PATH tools/meshway-rules-spike/run.sh .meshway/rules-spike planck-9950x
# 关键字自动机规模（合成关键字）
tools/meshway-rules-spike/target/release/meshway-rules-spike acbuild 4096
```

| 项 | 值 |
| --- | --- |
| 上游 | sing-geosite `df6851d8`（rule-set）、sing-geoip `7fe82a87`（rule-set）、meta-rules-dat `83c8208b`（meta）、blackmatrix7 `19c84682`（keyword 真实样本，v2fly 当前 0 个 keyword） |
| 主机 | Apple M3 Pro、macOS 27.0；AMD Ryzen 9 9950X、Linux 6.8（planck）；rustc 1.98.0，release，thin LTO |
| 原始输出 | `.meshway/rules-spike/results/{macos-m3pro,planck-9950x}/`：`coverage.json`、`bench-<set>-{1,2,3}.txt`、`memory.txt`、`sizes.txt`、`acbuild.txt`、`host.txt`；输入摘要 `.meshway/rules-spike/data.sha256`，上游提交 `upstream-commits.txt` |

表中时间取 3 轮中位数。macOS 无法绑核，P／E 核调度使其波动明显，以 planck 为准。macOS 结果在提交 `3b1b12a9` 前以相同源码运行（`host.txt` 记为 `9617eb4a`）。

### 转换覆盖

“现转换器”为真实 `convert` 的结果；分类为 spike 逐条转换的计数。

| 来源 | 源字节 | 现转换器 | 源条目 | exact | suffix | subdomain | keyword | regex 跳过 | CIDR | 其他未转换 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| sing-geosite `cn` | 232,270 | 整次失败 | 9,304 | 592 | 8,686 | 18 | 0 | 8 | 0 | 0 |
| sing-geosite `openai` | 934 | 整次失败 | 23 | 9 | 13 | 0 | 0 | 1 | 0 | 0 |
| sing-geosite `geolocation-!cn` | 619,382 | 整次失败 | 23,987 | 238 | 23,499 | 97 | 0 | 151 | 0 | 2 invalidName |
| sing-geoip `cn` | 213,971 | 成功 8,045 | 8,045 | | | | | | 8,045 | 0 |
| meta `geosite/cn.yaml` | 2,305,601 | 成功 111,595 | 111,595 | 0 | 111,595 | 0 | 0 | 0 | 0 | 0 |
| meta `geosite/openai.yaml` | 664 | 成功 22 | 22 | 9 | 13 | 0 | 0 | 0 | 0 | 0 |
| meta `geosite/geolocation-!cn.yaml` | 619,678 | 整次失败 | 27,189 | 756 | 26,431 | 0 | 0 | 0 | 0 | 2 invalidName |
| meta `geoip/cn.yaml` | 210,825 | 成功 9,609 | 9,609 | | | | | | 9,609 | 0 |
| meta 4 份 `.list` | 同上 | 格式不支持 | 同 YAML | | | | | | | 同 YAML |
| bm7 `Global` | 1,011,323 | 格式不支持 | 35,183 | 127 | 34,898 | 0 | 36 | 0 | 119 | 1 PROCESS-NAME、2 invalidName |
| bm7 `China` | 99,896 | 格式不支持 | 3,721 | 17 | 3,674 | 0 | 9 | 0 | 21 | 0 |
| bm7 `OpenAI` | 1,353 | 格式不支持 | 35 | 7 | 24 | 0 | 1 | 0 | 2 | 1 IP-ASN |
| bm7 `Advertising` | 9,436,179 | 格式不支持 | 287,021 | 16,440 | 269,814 | 0 | 278 | 0 | 489 | 0 |

- 受支持格式的 8 份中现转换器**整次失败 4 份**：三份 sing-geosite 都含 `domain_regex`（`deny_unknown_fields`），另有 `.`前缀名称；meta `geolocation-!cn` 因 `xn--wcvs22d1m.hk`、`xn--wcvs22d1m.xn--j6w193g` 未通过 IDNA 校验。新转换器对同样输入只跳过 regex 与 2 个非法名称。
- 上游 sing-geosite／sing-geoip 只发布 `.srs`；spike 的 JSON 由 `sing-box rule-set decompile` 得到。用户决定不解析 `.srs`，这些集合只作 sing-box source 转换的测试样本。
- 160 条 regex 中可无损降级的为 0。
- sing-geosite `cn`（9,304）与 meta `cn`（111,595）内容不同，二者不能共用 `geosite:cn` 这个名字而不说明来源。

### 大小

| 集合 | 源 | 现 JSON artifact | `MWRS` | gzip -9 | 规范条目 E／S／Sub／K／v4／v6 | 去冗余 |
| --- | --- | --- | --- | --- | --- | --- |
| sing-geosite `cn` | 232,270 | — | 50,184 | 38,870 | 518／6,182／18／0／0／0 | 2,578 |
| sing-geosite `geolocation-!cn` | 619,382 | — | 204,784 | 156,193 | 99／23,413／96／0／0／0 | 226 |
| sing-geoip `cn` | 213,971 | 141,521 | 98,632 | 25,531 | v4 4,155／v6 2,037（v4 最少 5,790 前缀） | 0 |
| meta `geosite/cn` | 2,305,601 | 1,636,052 | 816,448 | 594,389 | 0／111,595／0／0／0／0 | 0 |
| meta `geosite/geolocation-!cn` | 619,678 | — | 231,208 | 176,404 | 95／26,431／0／0／0／0 | 661 |
| meta `geoip/cn` | 210,825 | 172,410 | 107,088 | 27,759 | v4 4,340／v6 2,255（v4 最少 6,163 前缀） | 0 |
| `openai`（两种来源） | 934／664 | 582 | 592 | 471 | 9／13／0／0／0／0 | 0 |
| bm7 `Global` | 1,011,323 | — | 239,208 | 182,010 | 126／26,818／0／36／112／4 | 8,081 |
| bm7 `Advertising` | 9,436,179 | — | 3,663,064 | 2,843,394 | 16,440／269,814／0／278／448／3 | 0 |

16 份 artifact 在两台主机上 digest 完全相同；meta YAML 与 `.list` 相同；sing-geosite 与 meta 的 `openai` 均为 `sha256:44ab0c1a4c37…`。构建耗时 planck 0.0–216 ms（Advertising 最大）。hash 索引变体：meta `geosite/cn` 4,772,848 B，Advertising 18,714,256 B。

### 加载

p50，µs；planck／macOS。

| 集合 | mmap＋open | 读入堆＋open | SHA-256 全量 | 校验（FST 校验和＋区间顺序） | 现 JSON 解析 | HashSet 构建 |
| --- | --- | --- | --- | --- | --- | --- |
| sing-geosite `cn` | 3.4／15.4 | 1.9／12.8 | 21.7／125.5 | 6.6／11.9 | — | 400／500 |
| meta `geosite/cn` | 4.3／91.2 | 16.5／138.8 | 320.7／2,051 | 107.0／198.8 | 4,400／4,900 | 6,800／8,500 |
| sing-geosite `geolocation-!cn` | 4.0／29.2 | 3.5／25.2 | 81.7／484.2 | 26.7／48.6 | — | 1,500／1,900 |
| sing-geoip `cn` | 4.6／16.2 | 2.5／14.5 | 41.9／241.3 | 2.6／4.3 | 300／300 | — |
| bm7 `Global`（36 keyword） | 59.4／103.9 | 59.0／103.6 | 95.6／572.1 | 30.9／57.0 | — | 1,500／1,800 |
| bm7 `Advertising`（278 keyword） | 678.2／1,090 | 746.4／1,568 | 1,429／8,549 | 480.6／878.1 | — | 31,200／35,000 |

无 keyword 时打开只做头／段表／FST 头检查，与大小基本无关；有 keyword 时打开时间主要是自动机构建。

### 匹配

每类 100 万次查询，ns／次，planck／macOS；每次查询都与参考实现比对，**全部 0 差异**。命中查询从规范条目取样（suffix 一半加随机子标签，keyword 集合 10 % 为包含关键字的名称），未命中为随机名称并经参考实现确认不命中。

| 集合 | `MWRS` 命中 | 未命中 | hash 索引变体 命中／未命中 | HashSet 基线 命中／未命中 | 现 Policy 线性 命中／未命中（200 次） |
| --- | --- | --- | --- | --- | --- |
| sing-geosite `cn` | 124.7／180.9 | 42.7／64.8 | 30.1／43.7 | 37.2／55.5 | 0.48 ms／1.08 ms |
| meta `geosite/cn` | 140.3／244.4 | 53.0／96.7 | 37.2／51.3 | 43.6／65.0 | 6.65 ms／17.95 ms |
| sing-geosite `geolocation-!cn` | 153.7／223.1 | 56.7／79.9 | 36.6／45.5 | 44.8／51.9 | 1.75 ms／3.84 ms |
| bm7 `Global` | 150.5／195.4 | 112.4／176.5 | 40.6／99.7 | 46.8／62.2 | 2.23 ms／4.41 ms |
| bm7 `Advertising` | 212.2／428.8 | 89.3／126.8 | 103.0／68.5 | 100.8／54.9 | 29.6 ms／53.7 ms |

hash 索引、HashSet、现 Policy 三列为 planck 值。HashSet 基线不含 keyword 步骤，keyword 集合上不等价。

| 集合 | v4 命中 | v4 未命中 | v6 命中 | v6 未命中 |
| --- | --- | --- | --- | --- |
| sing-geoip `cn` | 17.5／20.4 | 17.6／20.1 | 42.8／21.9 | 7.9／19.7 |
| meta `geoip/cn` | 18.7／23.5 | 18.5／23.1 | 43.3／28.2 | 8.4／23.5 |
| bm7 `Global` | 10.5／11.7 | 11.5／13.5 | 11.3／7.2 | 4.7／3.9 |

IP 结果另与独立的 `partition_point` 实现逐条比对、前 2,000 条与线性扫描比对，均 0 差异。

### 内存：1,000 个句柄

meta `geosite/cn`（816,448 B），每个句柄打开后各执行 1,151 次查询使页驻留。planck 取 `/proc/self/smaps_rollup` 差值，macOS 取 `ps` RSS 与 `footprint` 差值。

| 方式 | planck Rss Δ | planck Pss Δ | macOS RSS Δ | macOS footprint Δ |
| --- | --- | --- | --- | --- |
| 1,000 份堆副本 | 785,884 kB | 785,884 kB | 769,456 KB | 782 MB |
| 1,000 个独立 mmap（同一文件） | 800,064 kB | 864 kB | 800,128 KB | 1 MB |
| 1 个 mmap、1,000 个句柄共享 | 800 kB | 800 kB | 912 KB | 0 MB |

共享 mmap 只计一份：约 0.8 MB。独立 mmap 的 Rss 按映射重复计数，但 Pss／footprint 证明物理页只有一份。hash 索引变体（4,772,848 B）对应为堆 Pss Δ 4,656,856 kB、独立 mmap 3,960 kB、共享 4,024 kB。

### 关键字自动机规模

合成随机关键字（5–12 字节），planck／macOS。

| 关键字数 | DFA 构建 ms | DFA 堆 | DFA 匹配 ns | 连续 NFA 构建 ms | 连续 NFA 堆 | 连续 NFA 匹配 ns |
| --- | --- | --- | --- | --- | --- | --- |
| 278 | 0.60／0.52 | 480,476 B | 23.1／28.3 | 0.11／0.15 | 67,148 B | 80.1／95.6 |
| 1,024 | 2.02／2.04 | 1,644,428 B | 34.3／35.1 | 0.39／0.51 | 194,840 B | 114.7／123.3 |
| 4,096 | 7.73／8.75 | 6,119,684 B | 47.9／61.1 | 1.69／2.18 | 493,156 B | 87.6／124.9 |
| 16,384 | 41.06／49.99 | 22,780,956 B | 76.0／89.6 | 8.56／11.00 | 1,342,344 B | 103.0／130.5 |

## 实施拆包

| 包 | 独占范围 | 依赖 | 必需验证 |
| --- | --- | --- | --- |
| R1 格式＋crate（**已完成**） | `crates/meshway-rules`（format、build、load、match、inline、convert、catalog）；`tools/meshway-rules-cli`；`tools/meshway-rules-perf`（`perf:rules`） | 无；未改 Server／Policy | 已完成：与 spike 参考实现 0 差异；loader fuzz；跨架构 digest 一致；`mise run check`（见 [R1 实测](#r1-实测)） |
| R2a Server＋Policy＋Control 接线（**已接线**，见 [R2a 接线](#r2a-接线)） | Server `resources/`（转换改用 R1、全局内容寻址存储、provider 目录与验签、`ruleSetProviders`、隐式展开、`RuleSetView` 新字段、`artifacts/{digest}` 路由与引用索引）；`meshway-configuration` netmap 类型（`RuleSetRef`、`meshway-netmap-2`）与配置类型；`meshway-policy` 的 `plan_match`／`capture`／validation 改用 matcher；`meshway-control` artifact 获取与缓存；OpenAPI／SDK 重生成 | R1 | 投影等价测试含规则集变异；缺 artifact fail closed；撤销不被下载阻塞；离线重启载入；1,000 节点引用同一 artifact 的 perf（Server 存储、刷新耗时、请求授权）；drift 与 Rust／TS 消费者 |
| R2b 发布仓库 | `meshway-org/meshway-rules` 仓库、workflow、签名 secret、`dist` 分支 | R1 CLI；与 R2a 并行 | 上游未变不发布；单集合失败 hold；sequence 单调；Server 用钉住公钥验通一次真实目录 |
| R2d 收尾（**已完成**，见 [R2d 收尾](#r2d-收尾)） | Server 目录主动更新、artifact 回收；发布流水线签名 held | R2a、R2b | 见该节验收 |
| R3 出口分割物化（**已实现**，见 [R3 节点侧出口分割](#r3-节点侧出口分割)） | Policy 区间分割；Engine desired；Underlay nft interval 集合与私有捕获表 | R2a | 已完成：7,000 个 direct 区间的 `exit/off` 与全量 16 行矩阵；artifact 缺失保持全范围捕获 |
| X6 可信域名（**已交付**） | 按 [Exit 设计](/v2/node/exit-design#trusted-dns-associations) 的 X6 边界；DNS owner 查询时调用 Policy `egress_rule_for_name` | R2a | 已验：同 IP 多名称冲突、TTL、跨 selection 不复用、域名规则仅在可信关联存在时可执行（`exit/off` 第 10 阶段） |

## 需用户决定

R0 列出的七项已由用户决定并写入相应章节：regex v1 逐条跳过（[匹配语义](#匹配语义)）；内置 provider 对齐 mihomo、指向 meta-rules-dat（[配置面](#配置面)）；签名钥为单把 GitHub environment secret，轮换即发布钉入新公钥的 Server 版本（[发布仓库](#发布仓库)）；仓库 `meshway-org/meshway-rules`；`mapped` 的 `unsafe` 例外后经 alpha.4 审阅撤回，改为只用堆读取（[crate 与 owner](#crate-与-owner)）；subdomain 在格式、转换器与 inline（`domainSubdomain`）支持；出口分割移到节点（R3，[Policy 接口与 X6](#policy-接口与-x6)）。

## R2a 接线

| 范围 | 实现（owner） |
| --- | --- |
| 配置 | `meshway-configuration`：`RuleSet::Inline.domainSubdomain`；`RuleSetFormat` 增 `mihomo-domain-text`／`mihomo-ipcidr-text`／`classical`；`ruleSetProviders: key → RuleSetProvider \| null`；内置 provider 表与 `<provider>:<name>` 解析（`providers`）；校验码 `invalid_rule_set_key`、`unknown_rule_set_provider`、`invalid_rule_set_provider`（R2a 的警告 `rule_set_partition_limit` 已由 R3 的 `egress_partition_limit` 取代） |
| netmap | `projection.ruleSets: key → RuleSetRef { digest, format, bytes, entries }`，投影不含任何集合定义；`compilerVersion` `meshway-netmap-2` |
| Policy | `compile::rules`：规则自身列表经 `RuleSet::from_inline`、引用集合经已加载句柄，逐规则 `match_domain`／`match_ip`；路径检测、出口分割（R3 起为区间分割，见下节）、节点 `LocalCompiler::{egress_rule_for_ip, egress_rule_for_name, effective, missing}` 与配置校验（`shadowed_rule`／`exit_outside_access` 读 artifact 迭代器）共用；缺句柄即 `UnavailableDependency` |
| Server | `resources/rulesets`（来源与转换适配）、`catalogs`（目录验签，一次准备内共用目录）、`artifacts`（全局内容寻址 `rule_set_artifacts`、每 (来源, digest) 不可变溯源 `rule_set_records` 与随每次刷新更新的当前记录 `rule_set_record_heads`（同 digest 的新目录 sequence、upstream、held 与报告只更新后者）、每进程每 digest 一个 `Arc<RuleSet>`）、`rule_set_catalogs`（持久 sequence 单调，保存验签通过的目录原文）、`browse`（目录搜索、成员列表与命中查询，只读）、`node_netmap_artifacts`（netmap 写入同事务的引用索引）；`GET /rule-sets` 含隐式项，`GET /rule-sets/{key}/report`、`/entries`、`/match`，`GET /rule-set-catalog`，`GET /agent-api/v1/artifacts/{digest}`；schema 版本 5 |
| Node | `meshway-control::netmap::artifacts`（取、校验、0400 原子落盘、按引用 GC、离线载入，sync 后顺序取回）；`meshway_engine::Netmap.artifacts` 与 `Desired::{missing, same}`；组件以 `Desired::same` 判断期望状态是否变化；IPC `NetmapStatus.{missingArtifacts, artifactProblem}` |
| 生成链 | 生成器支持 `application/vnd.meshway.*` 二进制 GET（Rust 返回 `Vec<u8>`，TS 客户端不生成该 Node 专用操作） |

## R3 节点侧出口分割

| 范围 | 实现（owner） |
| --- | --- |
| 分割计算 | `meshway-policy` `compile::capture`：`compute`／`partition`（first-match 区间扣除，覆盖两族、相邻区间动作不同的 `EgressPartition`）、`summary`（`CaptureSummary`）、`EgressPartition::capture`（受管 IPv4／sink IPv6 合并区间的 `EgressCapturePlan`）；Server 校验（`egress_partition_limit`）、路径检测 `profile.capture`、`NetmapDns.resolvers` 与节点 `LocalCompiler::{partition, capture, decide, transit}` 共用；节点按方案缓存 |
| netmap | `EgressPlan { generation, rules: [{rule, action}], fallback, unavailable }`，不含前缀与 `EgressRoute`；`generation` 绑定生效规则所用集合的 digest，与节点持有的 artifact 无关 |
| desired | `meshway_engine::Desired::{capture, partition, capture_summary}`；DNS 的本地 resolver 判定与 served 上游检查读 `Desired::partition` |
| 安装 | `meshway-underlay` `kernel::egress`：受管区间为 `managed4`／`managed6` interval 元素（无回显批替换、读回逐元素与计数核验，大小为两份方案的暂存并集），未经活动 TUN 的受管目的立即拒绝（TCP reset／ICMP admin-prohibited）；一条 IPv4 RPDB 选择器查私有捕获表，表中为活动时受管区间的最小 CIDR 覆盖；journal v2 只记 gate（区间与设备），表内路由由 gate 推出，恢复时精确对账删除；路由变更每 64 条等待内核监视器观测；DNS 锚点冲突检查豁免该表 |
| 可信关联（X6） | `LocalCompiler::associated_capture`：关联地址按名称规则并入（出口／冲突）或移出（direct／unmanaged）受管区间，`Desired::with_associations` 重算捕获，到期由 Engine 重新应用；超出节点侧上限即全范围捕获；Server 不再校验捕获大小 |
| 诊断 | IPC `NetmapStatus.capture`、CLI `meshway status` 的 Egress 行；`mise run perf:capture` |

不变量与选择：分割计算只在 Policy，Underlay 只把区间转换为 nft 元素与 CIDR 覆盖，不做匹配；源 socket 的源地址在 `connect` 时由 FIB 选定，nft 标记后重路由会保留物理接口地址，因此 IPv4 导向仍用私有表路由（区间的 CIDR 覆盖），nft 集合承载原子的负向门与 IPv6 sink，不引入源端 SNAT／conntrack 依赖，严格 rp_filter 下回复路径不变。

实测与上限见 [Routing · 节点侧上限](/v2/configuration-reference/routing#capture-limits)；隔离组件证据在私有 `.meshway/validation/r3/kernel/`（`default_capture_…`、`large_partitions_…`、SIGKILL 后负向保留），矩阵在 `.meshway/validation/r3/matrix/`（合并 X6 后 `20261012T115405Z-61e14403` 16 行通过）。

## R2d 收尾

| 范围 | 实现（owner） |
| --- | --- |
| 目录主动更新 | `resources::scheduler` 每轮在本网络有效 provider 的目录与集合中取到期时刻最早的一项（`catalogs::due_at`：从未验签为 0，否则上次成功／失败加间隔／退避；集合准备的验签算成功；同一时刻目录在前），选中目录即 `catalogs::refresh`：`Downloads::catalog` 在共用 4 项预算内下载并验签，同一事务 `accept_catalog`（sequence 单调，回退为 `catalog_regressed`）并写 `rule_set_catalog_checks`（schema 7）。键为 (目录 URL, 信任钥集合)，多网络只取一次；调度串行，不需合并 |
| artifact 回收 | `resources::artifacts::collect`（调度器启动时及每小时一次，单个写事务）：保留未删除网络全部版本的初始绑定（经 sealer 打开 `configuration_versions.bindings`）与 `rule_set_bindings` 刷新绑定引用的 digest，以及 `node_netmap_artifacts` 中当前或被替换不足 `MAX_LEASE_MS`（24 小时，租约有效期上限）的引用；其余连同 `rule_set_records`／`rule_set_record_heads` 删除，并移出进程内句柄缓存。删除 `rule_set_artifacts_retained` 触发器 |
| 签名 held | 发布流水线写 `SetRecord.held`，`meshway-rules index` 带入 `CatalogSet.held`；Server 视图早已透出 |

保留期常量来源：配置版本由 `configuration_versions_no_delete` 触发器永久保留，因此“可回滚”的绑定没有时间上限；时间窗口只来自租约——`store::resources::artifacts::MAX_LEASE_MS`（`netmap::SyncTiming` 允许的最长租约，24 小时），与 `node_netmap_artifacts` 修剪、`GET /agent-api/v1/artifacts/{digest}` 授权使用同一常量。

验收：`app::tests::resources::catalogs`（未引用时取回；两网络同 URL 只取一次；从未成功为 409；失败保留；新 sequence 替换、低 sequence 不替换；held 在目录页与视图透出）、`app::tests::resources::retention`（租约窗口内不回收、窗口外回收被替换内容及其记录，初始与刷新绑定均保留并可经 `initial`／`refreshed` 回滚）；发布仓库 `scripts/test_publish.py`；Planck 16 行矩阵 `20261012T212926Z-abd9b9c8`、`20261012T232408Z-75436f19` 与合入 main 后的 `20261013T015222Z-59ebfafc` 全部通过；新部署的本地 Server 在无任何引用时 3 秒内取得真实内置目录（`geosite`／`geoip`，sequence 3），证据 `.meshway/validation/r2d/builtin-prewarm/`。

## 未解决

- sing-box `ip_cidr` 接受裸地址（视为 /32、/128），现转换器与 R1 都报告为 `invalidCidr`；是否放宽未定。
- 打开耗时与 FST 节点数线性相关（3.66 MB 约 42 ms）；若需更快，可只在 Server 入库时做 FST 证明、节点以 digest 信任，但那会让“任意字节不 panic”依赖 digest 来源，需另行决定。
- R3 的导向用私有捕获表路由而不是设计稿的 nft fwmark：fwmark 重路由不改源地址，需源端 SNAT 并在严格 rp_filter 下失去回复路径；是否仍要求 fwmark 方案待用户决定。
- 我们的规范化会拒绝 sing-box／mihomo 接受的部分名称（例如 `xn--wcvs22d1m.hk`、含 `_` 的名称），这些条目被报告并跳过，与上游语义存在差距（[规范化](#规范化与-digest)）。
- 目录超过 7 天未前进时的 `stale` 视图字段未实现；Server 自己取回目录的最后错误只持久保存（`rule_set_catalog_checks.problem`），尚未出现在目录页或 409 的 params 中。
- 1,000 节点共享一个 artifact 的 Server perf（存储、刷新耗时、授权查询）未测。
- 未在 Linux arm64 与常驻 lab 节点上测量；macOS 时间波动较大。R1 的 Linux clippy 覆盖 `aarch64-unknown-linux-musl`，测试与实测只在 macOS arm64 与 planck x86_64 上运行。
- 测试不访问外网：库单测、集成黑盒测试（`apps/meshway-server/tests/`）与各测试 Server（`meshway-test-server`、daemon／CLI／control-client 的测试）都以 `App::open_with_connector(…, resources::download::Offline)` 装配 App（离线连接器不解析、不连接，取回立即失败），资源测试注入自己的 HTTPS 源。例外是启动 `meshway-server` 可执行文件的 `http_cli` 测试：可执行文件没有连接器注入点，`open_event_streams_do_not_block_signal_shutdown_or_storage_release` 创建网络后它在后台尝试取回内置目录（H1 在 planck 上以 `strace` 跟踪全量 243 个 Server 测试，唯一的非回环连接是这个进程对 `raw.githubusercontent.com:443` 的一次取回及其 DNS 查询；离线时失败并退避，不影响结果）。
