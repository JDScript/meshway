# 规则集引擎设计草案

> **未实现、未验收。** 本页是 R0 设计研究稿，不加入文档导航，不构成可用 API 或功能承诺。当前行为仍以 [Rule Sets](/v2/configuration-reference/rule-sets)、[Routing](/v2/configuration-reference/routing) 与 [Netmap 设计](/v2/node/netmap-design) 为准。字段名为草案，最终类型由所属 crate 定义并生成 OpenAPI／SDK。

## 目标与结论

已定方向（用户决定）：自有二进制规则集格式；发布仓库自动把 sing-box（sing-geosite／sing-geoip）与 mihomo（meta-rules-dat）来源转换后签名发布；Server 内置 provider 指向它，`geosite:openai`／`geoip:cn` 可直接引用，网络配置可覆盖 provider；inline 与自定义 remote 仍支持，且与发布仓库**共用同一个转换器**。匹配引擎进入 Policy：域名在 DNS 查询时匹配（X6），IP 在首包匹配；支持 exact／suffix／keyword 与 IPv4／IPv6 CIDR，regex 逐条报告并跳过；不做端口／进程匹配。netmap 只带 digest 引用，节点经认证端点按 digest 取一次并缓存。

本稿结论：

1. **格式 `MWRS` v1**：64 字节定长头＋段表；域名 exact／suffix／subdomain 共用一棵按字节反转的 FST，keyword 存排序字面量、加载时建 Aho-Corasick，IP 为合并后的有序区间数组。digest＝整份 artifact 字节的 SHA-256；构建确定性，集合相等即字节相等（spike 中 mihomo YAML 与文本、sing-geosite 与 meta-rules-dat 的 openai 得到同一 digest，arm64／x86_64 一致）。
2. **溯源不进 artifact**。上游 repo／commit／格式／未转换条目报告放在发布目录和 Server 资源记录中；否则上游每次提交都会改 digest，引发无内容变化的全网 netmap 更新。
3. **唯一实现 `crates/meshway-rules`**：格式、builder、loader、matcher、来源转换器。Policy 只用 matcher API；Server、发布流水线（CLI）和节点 Control 复用同一 crate。
4. **规则集内容按 digest 下发**：netmap 的 `ruleSets` 只有 `{digest, bytes, entries}`，替代现在每份 netmap 内联的 JSON 内容；规则集刷新只改变引用。
5. spike 实测：现转换器在 8 份受支持格式的真实来源中**整次失败 4 份**；新格式 artifact 为 source 的 22–39 %（域名）／46–51 %（IP），打开 3–5 µs（无 keyword、planck），`match_domain` 命中 125–212 ns／未命中 43–112 ns、`match_ip` 5–43 ns，与参考实现 0 差异；现 Policy 线性匹配同类查询要 0.48–54 ms。详见 [spike 实测](#spike-实测)。

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

所有整数小端；段起点 8 字节对齐；未使用字节为 0。

| 偏移 | 字段 | 说明 |
| --- | --- | --- |
| 0 | `magic` | `MWRULES\0` |
| 8 | `major` u16 | 不兼容变更时递增；loader 拒绝未知 major |
| 10 | `minor` u16 | 只增加可选段或可选头字段 |
| 12 | `flags` u32 | v1 为 0 |
| 16 | `sectionCount` u32 | ≤ 64 |
| 24 | `totalLength` u64 | 必须等于实际字节数 |
| 32 | `counts` 6×u32 | exact、suffix、keyword、v4 区间、v6 区间、subdomain |
| 56 | 保留 | 0 |
| 64 | 段表 | 每项 24 B：`kind` u32、`flags` u32（bit0＝required）、`offset` u64、`length` u64 |

| kind | 段 | 编码 |
| --- | --- | --- |
| 1 | `domains` | `fst::Map`：key 为规范名称的**字节反转**，value 位 0＝exact、1＝suffix、2＝subdomain。一次从尾到头遍历即可在每个标签边界判断 suffix／subdomain，在名称起点判断 exact，O(名称长度)，与条目数无关 |
| 2 | `keywords` | `count` u32、`count+1` 个 u32 偏移、字节 blob；字面量排序去重 |
| 3 | `ipv4` | `count` 个 `(start u32, end u32)` 闭区间，排序、不相交、不相邻 |
| 4 | `ipv6` | `count` 个 `(start u128, end u128)`，同上 |

**为什么是 FST 而非 hash 索引**：spike 同时实现了零拷贝的标签后缀 hash 索引（FNV-1a 增量哈希，开放寻址）。它的命中快约 3–4 倍（planck geosite-cn 37 ns 对 140 ns），但 artifact 大 5.8 倍（4,772,848 B 对 816,448 B）；域名匹配发生在 DNS 查询而非逐包，140 ns 相对 DNS 往返可忽略，而大小直接乘到每个节点的下载与常驻内存，因此 v1 只保留 FST。HashSet 等加载时构建的结构需反序列化（geosite-cn 6.8 ms），不满足“加载不反序列化”。

**keyword 在加载时建自动机**：Aho-Corasick 没有可零拷贝加载的稳定序列化格式；实测 278 个关键字的 DFA 构建 0.60 ms、1,024 个 2.0 ms／1.6 MB 堆（[关键字](#关键字自动机规模)）。v1 每个 artifact 最多 1,024 个关键字，进程内每个 digest 只建一次。

**IP 为区间而非前缀**：区间二分查找与前缀长度无关；同一数组也是 nftables interval set 的直接输入。需要展示命中条目时，在冷路径把所在区间按规范最小 CIDR 分解给出包含该地址的那一块。

### 规范化与 digest

builder 的输入是转换器产出的条目，输出唯一确定：

- 名称：IDNA→ASCII、小写、去结尾点；label 1–63 字节、全名 ≤ 253、字符 `[a-z0-9-]` 且不以 `-` 开头或结尾（与现 `canonical_domain` 相同）。
- 去冗余：被同集合 suffix 覆盖的 exact／suffix／subdomain，被 keyword 覆盖的名称，包含另一 keyword 的 keyword 均删除；`exact(n)`＋`subdomain(n)` 合并为 `suffix(n)`。CIDR 掩码规范化后合并为区间。
- `digest = "sha256:" + hex(SHA-256(artifact 全部字节))`。`bytes`＝artifact 字节数；`entries`＝exact＋suffix＋subdomain＋keyword＋v4 区间＋v6 区间（规范化后计数，不是源条目数）。

去冗余的条目数单独报告（geosite-cn 2,578、blackmatrix7 Global 8,081），不算未转换。

### 来源溯源与转换报告

溯源与报告**不进入 artifact 和 digest**。它们属于“这份内容从哪来”，由发布目录（provider 集合）或 Server 资源记录（自定义 remote）保存：

| 字段 | 说明 |
| --- | --- |
| `source.repo`／`ref`／`commit`／`path` | 上游仓库、分支、提交 SHA 与文件路径 |
| `source.format` | 输入格式（见[转换器](#转换器)） |
| `source.sha256`／`source.bytes` | 原始输入摘要与大小 |
| `converter` | `meshway-rules` 版本与源码提交 |
| `report.converted` | 按 kind 计数 |
| `report.redundant` | 去冗余删除数 |
| `report.unconverted` | 按原因计数：`regex`、`invalidName`、`wildcard`（mihomo `*.`）、`mixedRule`（sing-box 名称与 IP 混合）、`unsupportedField:<name>`、`classical:<TYPE>` 等 |
| `report.items` | 未转换条目原文与原因，最多 1,000 条，超出只计数 |

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
- regex 逐条报告并跳过；仅当正则恰好等价于 `^literal$`、`(^|\.)literal$` 或无锚点字面量时降为 exact／suffix／keyword（真实数据中 160 条一条都不满足）。跳过意味着本集合是上游语义的子集，`RuleSetView` 必须显示 `unconverted.regex` 计数。
- 匹配输入必须是规范名称；DNS owner 每次查询规范化一次（`meshway_rules::Name::parse`），不在 matcher 内重复。
- `match_domain` 返回 `Option<DomainHit>`：`Exact`、`Suffix { len }`（命中项是查询名的尾部 `len` 字节）、`Subdomain { len }`、`Keyword { index }`。由此可在不额外存储的情况下给出评估结果的 `RuleSetMatch.entry`。`match_ip` 返回 `Option<IpHit { range }>`。

## crate 与 owner

| 组件 | 持有 | 不得 |
| --- | --- | --- |
| `crates/meshway-rules` | 格式常量、builder、loader、matcher、全部来源转换器与报告类型 | 依赖 Server／Policy／网络 I/O；第二套匹配 |
| `meshway-policy` | 首个匹配规则判定；只调用 `RuleSet::match_domain`／`match_ip`／迭代器，以及从路由规则内联列表构建内存 `RuleSet` 的 `RuleSet::from_inline` | 读取文件、下载、解析来源格式；自写字符串比较 |
| Server `resources/` | 下载、provider 目录验签、转换、artifact 存储（全局内容寻址）、网络绑定、`GET /rule-sets`、`artifacts/{digest}` 授权 | 在 netmap 内联内容 |
| Node `meshway-control` | 按 netmap 引用取 artifact、校验 digest、私有原子缓存与清理 | 解析或重建 artifact 内容 |
| `meshway-engine` | 把已加载的 `RuleSet` 句柄连同 netmap 交给 Policy／组件 | 持有第二份授权状态 |
| 发布流水线 CLI（`tools/meshway-rules-cli`） | 调用 `meshway-rules` 转换并写目录 | 自带转换逻辑 |

feature 划分：默认只含 matcher、loader 与内存 builder（Policy、Engine 用）；`convert` 含来源解析与报告（Server、CLI 用）；`mapped` 含 mmap 后端（Server、Control 用）。

**mmap 与 `unsafe`**：`memmap2::Mmap::map` 是 `unsafe`（文件被截断或改写即未定义行为／SIGBUS），与“业务代码 forbid unsafe”冲突。建议放在 `meshway-rules::mapped` 一个隔离并审计的模块中，前提条件是：只映射 Control／Server 私有目录中以 digest 命名、0400、经原子 rename 写入且此后不改写的文件；进程首次打开每个 digest 时整份校验 SHA-256（planck 816 KB 0.32 ms）。loader 接受任意 `AsRef<[u8]>` 后端，堆读取（`std::fs::read`）是同一 API 的另一后端（planck 816 KB 打开 16.5 µs），因此 mmap 例外是否批准不影响其他设计（见[需用户决定](#需用户决定)）。

**每进程每 digest 一个句柄**：Server 计算所有节点的路径检测时共享同一 `Arc<RuleSet>`；1,000 个句柄共享一份映射只增加 0.8 MB，1,000 份堆副本增加 786 MB（[内存](#内存-1-000-个句柄)）。

### 转换器

| `format` | 输入 |
| --- | --- |
| `sing-box-source` | sing-box rule-set source JSON v1–v3；字段可为字符串或数组（sing-box listable，`sing-box rule-set decompile` 对单元素输出字符串） |
| `sing-box-binary` | `.srs` v1–v3（sing-geosite／sing-geoip 上游只发布此格式） |
| `mihomo-domain`／`mihomo-ipcidr` | mihomo YAML `payload`（现有） |
| `mihomo-domain-text`／`mihomo-ipcidr-text` | meta-rules-dat `.list`，每行一项 |
| `classical` | `TYPE,value[,...]` 文本 |
| `meshway` | 本格式；解析后按同一 builder 重建，digest 必须不变 |

转换只在文档无法解析、超出预算或转换后为空时整次失败（`invalid_source`／`too_large`／`rule_set_budget`／`conversion_failed`）；单条无法转换只进入报告。

## 发布仓库

| 项 | 方案 |
| --- | --- |
| 仓库 | `JDScript/meshway-rules`（公开）；`main` 放 workflow 与集合清单，orphan 分支 `dist` 放发布产物 |
| 集合 | `sing-geosite/<category>`、`sing-geoip/<cc>`、`meta-geosite/<category>`、`meta-geoip/<cc>`；默认全量上游类别 |
| 频率 | 每天 1 次 cron＋手动 dispatch；上游 commit 全未变化则不发布 |
| 转换器 | 从 meshway 指定 tag 构建 `tools/meshway-rules-cli`（`--locked`），版本写入目录 |

流水线：

1. 读取上游 `rule-set`（sing-geosite、sing-geoip）与 `meta`（meta-rules-dat）分支的当前 commit，按 commit SHA 拉取原文（raw URL 不重定向）。
2. 每个集合运行 CLI：转换、构建、报告。单个集合失败时沿用上一版 artifact，并在目录中记录 `held: {since, reason}`；规范条目数比上一版减少超过 50 % 视为可疑，同样 hold 并开 issue。
3. 生成 `index.json`：`schema: 1`、单调递增的 `sequence`、`generatedAt`、`converter`、`upstreams[]`（repo／ref／commit）、`sets: { "<collection>/<name>": { digest, bytes, entries, format, source, report } }`；完整报告另存 `reports/<digest>.json`。
4. 用 ed25519 签名 `index.json` 原始字节得 `index.json.sig`（含 key ID）。私钥存为仓库 environment secret，环境只允许 `main` 上的该 workflow 使用。
5. 强制推送 `dist`：`index.json`、`index.json.sig`、`artifacts/<digest>.mwrs`、`reports/`；保留最近 7 份目录引用的 artifact，使目录与 artifact 读取之间的更新竞态在实践中不发生。同时打 tag 并创建 GitHub Release 供人工审计。

**Server 怎样取与钉钥**：Server 从 `https://raw.githubusercontent.com/JDScript/meshway-rules/dist/index.json` 取目录与签名，用内置公钥（源码常量，可由部署设置追加或替换，网络管理员不能改内置 provider 的信任根）验签，拒绝 `sequence` 小于该 provider 上次接受值的目录；再按 digest 取 `artifacts/<digest>.mwrs` 并校验 SHA-256。GitHub Release 下载会 302 重定向，而现行下载器拒绝重定向，所以不用 Release 作为 Server 来源。

**失败行为**：目录取不到、验签失败、sequence 回退或 artifact 校验失败都保留上一份已验证内容，`problem` 记录原因，不发布半份；目录超过 7 天未前进时 `RuleSetView` 报告 `stale`，但不使执行失败。首次没有任何可用内容时与现有 remote 相同：`not_ready`，引用它的规则不可执行。

## 配置面

### `ruleSetProviders`

<!-- 草案字段，不是当前可提交 JSON -->
```json
{
  "ruleSetProviders": {
    "geosite": {
      "catalog": "https://raw.githubusercontent.com/JDScript/meshway-rules/dist/index.json",
      "collection": "sing-geosite",
      "publicKeys": ["ed25519:<base64>"],
      "updateInterval": "24h"
    },
    "corp": {
      "catalog": "https://rules.corp.example/index.json",
      "collection": "corp",
      "publicKeys": ["ed25519:<base64>"]
    },
    "meta-geoip": null
  }
}
```

| 字段 | 说明 |
| --- | --- |
| key | provider 名，`[a-z][a-z0-9-]{0,31}` |
| `catalog` | HTTPS 目录 URL，规则同现有 remote URL |
| `collection` | 目录内集合前缀 |
| `publicKeys` | 1–4 个 ed25519 公钥；覆盖内置 provider 时必须给出 |
| `updateInterval` | 默认 `24h`，调度、下界与退避同现有 remote |
| 值为 `null` | 禁用同名内置 provider |

- 内置 provider：`geosite`→`sing-geosite`、`geoip`→`sing-geoip`、`meta-geosite`、`meta-geoip`（默认指向哪组仍待用户决定）。网络配置中出现同名 key 时**整体替换**内置定义，不逐字段合并。
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
| 节点内存 | 解析后的字符串列表 | 一份 816 KB 映射 |
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
- 同一 IP 被多个名称解析且对应不相容动作时，沿用 [Exit 设计 · DNS 路径与可信域名上下文](/v2/node/exit-design#dns-路径与可信域名上下文) 与 [Routing](/v2/configuration-reference/routing#direct-unmanaged) 的既有决定：拒绝／不可执行，不选更宽路径；域名授权仍是解析地址集合。本稿不改变 X6 的可信关联生产者要求，域名型规则在 X6 前仍不可执行。
- 出口分割：IP 规则集不再展开为字符串 CIDR，Policy 用区间数组做 first-match 分割。分割结果若仍由 Server 计算并写入 `egress.routes`，geoip-cn 需要约 6,000 个 v4 前缀，远超现 4,096；推荐把分割改为在节点上由 Engine 用 artifact 区间物化（nftables interval set＋fwmark），netmap 只带有序“规则→动作”。这涉及 Underlay owner，列入[未解决](#未解决)。

## spike 实测

源码 `tools/meshway-rules-spike/`：独立 `Cargo.toml`（自带 `[workspace]`、`Cargo.lock`、`target/`），**不是 workspace 成员**，因此不影响根 manifest、lockfile 与 `mise run check`。`server` feature（默认开）链接真实的 `meshway_server::resources::convert` 与 `meshway_policy::domain_{name,suffix}_matches` 测现行实现。

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
- 上游 sing-geosite／sing-geoip 只发布 `.srs`；JSON 由 `sing-box rule-set decompile` 得到，因此 `sing-box-binary` 是必需格式。
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
| R1 格式＋crate | 新 `crates/meshway-rules`（format、build、load、match、convert、mapped）；`tools/meshway-rules-cli`；本稿格式定稿为 `docs/v2/` 下的格式参考页。manifest／lock 交协调者 | 无；不改 Server／Policy | 与本 spike 的参考实现做差分测试；`.srs` 解析与 `sing-box rule-set decompile` 差分；loader 对任意字节不 panic（fuzz）；跨架构 digest 一致；`mise run check` |
| R2a Server＋Policy＋Control 接线 | Server `resources/`（转换改用 R1、全局内容寻址存储、provider 目录与验签、`ruleSetProviders`、隐式展开、`RuleSetView` 新字段、`artifacts/{digest}` 路由与引用索引）；`meshway-configuration` netmap 类型（`RuleSetRef`、`meshway-netmap-2`）与配置类型；`meshway-policy` 的 `plan_match`／`capture`／validation 改用 matcher；`meshway-control` artifact 获取与缓存；OpenAPI／SDK 重生成 | R1 | 投影等价测试含规则集变异；缺 artifact fail closed；撤销不被下载阻塞；离线重启载入；1,000 节点引用同一 artifact 的 perf（Server 存储、刷新耗时、请求授权）；drift 与 Rust／TS 消费者 |
| R2b 发布仓库 | `JDScript/meshway-rules` 仓库、workflow、签名 secret、`dist` 分支 | R1 CLI；与 R2a 并行 | 上游未变不发布；单集合失败 hold；sequence 单调；Server 用钉住公钥验通一次真实目录 |
| R3 出口分割物化（建议） | Policy 区间分割；Engine desired；Underlay nft interval set 与 fwmark | R2a；与 X3 owner 协调 | geoip-cn 规模的 direct 例外不进 TUN；artifact 缺失保持全范围捕获；`exit/off` 行不回退 |
| X6 可信域名 | 按 [Exit 设计](/v2/node/exit-design#工作包与依赖) 的 X6 边界；DNS owner 查询时调用 Policy `egress_rule_for_name` | R2a | 同 IP 多名称冲突、TTL、跨 profile 不复用；域名规则仅在可信关联存在时可执行 |

## 需用户决定

1. **regex**：v1 逐条跳过。是否后续支持（候选：regex-automata 稠密 DFA 可序列化并零拷贝加载，作为新的 required 段）？真实影响：sing-geosite `cn` 8 条、`geolocation-!cn` 151 条、`openai` 1 条。
2. **内置 provider 默认来源**：`geosite:`／`geoip:` 指向 sing-geosite／sing-geoip（v2fly 系，`cn` 9,304 条）还是 meta-rules-dat（`cn` 111,595 条）；另一组是否以 `meta-geosite:`／`meta-geoip:` 内置。
3. **签名密钥归属**：GitHub environment secret（可每日自动发布）、离线钥＋人工发布，或 sigstore keyless（Server 需验证 Fulcio／Rekor）。以及谁持有轮换用的第二把钥。
4. **发布仓库名与归属**：建议 `JDScript/meshway-rules`。
5. **mmap 的 `unsafe` 例外**：是否批准在 `meshway-rules::mapped` 内隔离审计（不批准则节点与 Server 用堆读取，同一 API，打开 816 KB 约 16.5 µs，每进程每 digest 一份私有内存）。
6. **subdomain 语义**：真实数据有 115 条 `.example.com`（只匹配子域）。格式与转换器建议支持；inline 配置面是否也暴露对应字段（例如 `domainSubdomain`）。
7. **出口分割位置**：Server 继续计算并把前缀写进 netmap（需把 4,096 上限至少提到约 16,384，内容又回到每份 netmap），还是按 R3 移到节点物化。

## 未解决

- R3 未经 Underlay owner 评审；nftables interval set 在 6,000＋区间下的提交时延、原子替换与 conntrack 交互未测。
- spike 没有实现 `.srs` 解析，靠 sing-box CLI 解码；R1 需自行实现并做差分。
- X6 可信关联生产者仍不存在；本设计让 DNS 查询时匹配变快，但不让域名型出口规则变为可执行。
- 我们的规范化会拒绝 sing-box／mihomo 接受的部分名称（例如 `xn--wcvs22d1m.hk`），这些条目被报告并跳过，与上游语义存在差距。
- 现 Policy 路径检测在大规则集上每次查询 0.5–54 ms（实测），在 R2a 前已是可观测的性能问题。
- 未在 Linux arm64 与常驻 lab 节点上测量；macOS 时间波动较大。
- Server 端 artifact 保留期限（回滚需要历史 digest）沿用现有“保留期限在接口／运维规则中给出”，未定数值。
