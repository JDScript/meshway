# Rule Sets

Server 管理的 IP／域名集合，供 [routing](/v2/configuration-reference/routing) 的 egress 规则以 `match.ruleSets` 引用。每个集合的内容是一个不可变的 `MWRS` artifact（格式见[规则集引擎设计](/v2/rule-sets-design#二进制格式)），以 `sha256:<hex>` digest 标识；节点的 netmap 只携带 digest 引用，节点按 digest 取一次并缓存（见[下发](#下发)）。

集合有三种来源：

- 本网络 `ruleSets` 中的 inline 声明；
- 本网络 `ruleSets` 中的 remote 声明（HTTPS 源，由 Server 下载并转换）；
- `<provider>:<name>` 引用：签名目录（provider）中的一个集合，例如 `geosite:openai`、`geoip:cn`。

## Structure

`ruleSets` 字段值。remote URL 是示例，提交时必须能取得符合格式的数据：

```json
{
  "office": {
    "type": "inline",
    "ipCIDR": ["10.20.0.0/16"],
    "domainSuffix": ["corp.example"],
    "domain": ["login.partner.example"],
    "domainSubdomain": ["cdn.example"]
  },
  "regional": {
    "type": "remote",
    "url": "https://rules.example.com/regional.json",
    "format": "sing-box-source",
    "updateInterval": "24h"
  }
}
```

`ruleSetProviders` 字段值（可省略；同名 key 整体替换内置 provider，`null` 禁用它）：

```json
{
  "corp": {
    "catalog": "https://rules.corp.example/dist/index.json",
    "collection": "corp",
    "publicKeys": ["ed25519:68dfc59cf585e0320129db7f1716b0dbb3d13c462fcefee7151d4b51ae1fc45c"],
    "updateInterval": "24h"
  },
  "geoip": null
}
```

## Fields

### `ruleSets`

| 字段 | 适用类型 | 说明 |
| --- | --- | --- |
| key | 所有 | 集合名，非空且不含 `:`（`:` 保留给 `<provider>:<name>` 引用） |
| `type` | 所有 | `inline` 或 `remote` |
| `ipCIDR` | inline | 规范 CIDR 列表，默认 `[]` |
| `domainSuffix` | inline | DNS 标签后缀：名称本身及其子名称，默认 `[]` |
| `domain` | inline | 精确 DNS 名称，默认 `[]`，为空时规范输出省略 |
| `domainSubdomain` | inline | 只匹配子名称、不含名称本身（sing-box／mihomo 的 `.example.com`），默认 `[]`，为空时规范输出省略；四项不能都为空 |
| `url` | remote | HTTPS 数据来源，不含 userinfo 或 fragment |
| `format` | remote | `sing-box-source`、`mihomo-domain`、`mihomo-ipcidr`、`mihomo-domain-text`、`mihomo-ipcidr-text`、`classical`，见[转换](#转换) |
| `updateInterval` | remote | 默认 `24h`，正 duration（`ms`／`s`／`m`／`h`）；调度下界与退避见[周期更新](#周期更新与生命周期) |

inline 不接受 remote 字段，反之亦然。inline 名称按 IDNA 规范化为小写 ASCII、去结尾点；任一无效条目是配置错误（`invalid_domain`／`invalid_domain_suffix`／`invalid_destination_cidr`），不跳过。key 含 `:` 报告 `invalid_rule_set_key`。

### `ruleSetProviders`

| 字段 | 说明 |
| --- | --- |
| key | provider 名，`[a-z][a-z0-9-]{0,31}` |
| `catalog` | 目录 `index.json` 的 HTTPS URL，不含 query、userinfo、fragment；签名为同目录的 `index.json.sig`，artifact 为同目录的 `artifacts/sha256-<hex>.mwrs`，完整转换报告为 `sets/<collection>/<name>.json` |
| `collection` | 目录内集合前缀，`[a-z0-9][a-z0-9-]{0,31}` |
| `publicKeys` | 1–4 个互不相同的 `ed25519:<64 位小写 hex>`，任一签名有效即接受 |
| `updateInterval` | 默认 `24h`，与 remote 相同 |
| 值为 `null` | 禁用同名内置 provider |

声明无效报告 `invalid_rule_set_provider`（`params.field` 为 `name`／`catalog`／`collection`／`publicKeys`）。

内置 provider（对齐 mihomo 的 meta-rules-dat 集合）：

| provider | 目录 | collection |
| --- | --- | --- |
| `geosite` | `https://raw.githubusercontent.com/meshway-org/meshway-rules/dist/index.json` | `meta-geosite` |
| `geoip` | 同上 | `meta-geoip` |

内置 provider 只信任发布流水线的一把公钥 `ed25519:68dfc59cf585e0320129db7f1716b0dbb3d13c462fcefee7151d4b51ae1fc45c`（Server 源码常量）。轮换该钥即发布钉入新公钥的 Server 版本；网络配置可以用同名 key 整体替换 provider，但不能修改内置 provider 的信任根。

### `<provider>:<name>` 引用

`match.ruleSets` 中含 `:` 的项是 provider 引用：第一个 `:` 前为 provider，之后为集合名（`[a-z0-9][a-z0-9!@._+-]{0,127}`），选择目录中的 `<collection>/<name>`，例如 `geosite:openai` 为 `meta-geosite/openai`、`geoip:cn` 为 `meta-geoip/cn`。引用展开为隐式集合，不写回配置原文与规范值，只出现在资源视图与 netmap 引用中。未知或已禁用的 provider 报告 `unknown_rule_set_provider`（`params.provider`），目录中没有该名称报告 `unknown_rule_set`（`params.key`，挂在引用的 JSON Pointer 上）。

## 转换

Server 只用 `meshway-rules` 一份转换器，与发布仓库相同。每个源条目要么成为成员，要么带原因跳过并进入报告；只有文档不能按声明格式解析（`invalid_source`）、源超过 32 MiB（`too_large`）、结果超出 artifact 上限（`rule_set_budget`）或没有任何条目可转换（`conversion_failed`）时整次失败。

| `format` | 输入 |
| --- | --- |
| `sing-box-source` | sing-box rule-set source JSON v1–v3：`domain`（精确）、`domain_suffix`（以 `.` 开头为只子名称，否则后缀）、`domain_keyword`、`ip_cidr`；不解析 `.srs` |
| `mihomo-domain`／`mihomo-ipcidr` | mihomo YAML `payload`：`+.x` 后缀、`.x` 只子名称、裸名精确；CIDR 列表 |
| `mihomo-domain-text`／`mihomo-ipcidr-text` | mihomo `format: text`（meta-rules-dat `.list`），每行一项，跳过空行与 `#` 行 |
| `classical` | mihomo／Clash classical `TYPE,value[,option]`：`DOMAIN`、`DOMAIN-SUFFIX`、`DOMAIN-KEYWORD`、`IP-CIDR`／`IP-CIDR6`（`no-resolve`） |

跳过的原因：`regex`（不降级）、`invalidName`／`invalidKeyword`／`invalidCidr`、`wildcard`、`mixedRule`（sing-box 规则内名称与 IP 为 AND）、`emptyRule`、`notString`、`unsupportedField:<name>`、`classical:<TYPE>`。有跳过时集合是上游语义的子集；视图的 `conversion.unconverted` 给出计数，`GET .../report` 给出前 1,000 条。名称规范化比 mihomo／sing-box 严格：含 `_` 的名称与解码后不是合法 IDNA 的 punycode 标签报告为 `invalidName`。

匹配：精确名称、后缀（名称及标签边界上的子名称）、只子名称、keyword（规范查询名包含该子串）、IPv4／IPv6 区间；同一集合内取并集。DNS 查询名只做小写与去结尾点后按字节比较，含 `_` 的查询名仍可命中后缀与 keyword。

## 上限

| 对象 | 上限 |
| --- | --- |
| 单次源下载 | 32 MiB；目录 `index.json` 同为 32 MiB |
| 单个 artifact | 16 MiB、1,048,576 个名称、1,024 个 keyword、各族 262,144 个合并区间（`rule_set_budget`） |
| 每网络绑定 | 不同 digest 合计 64 MiB、最多 128 个，按 digest 去重（多个 key 指向同一内容只计一次）；超出为 `/ruleSets` 上的 `rule_set_budget`（`params.maxBytes`／`maxSets`） |
| 单次配置准备 | 最多 32 项需下载的集合（remote 与 provider），总期限 30 秒（`rule_set_timeout`） |
| 节点 | 当前 netmap 引用的 artifact 合计 64 MiB，超出时不下载并报告 `budget` |

出口分割由节点用集合的合并区间直接计算并安装为 nft interval 集合，netmap 只带 digest 引用；`geoip:cn` 级集合可以用于出口分割。节点侧上限（16,384 个捕获区间、32,768 条捕获路由）与超限诊断 `egress_partition_limit` 见 [Routing · 节点侧上限](/v2/configuration-reference/routing#capture-limits)。

## 数据与状态

- 配置保存定义、来源和更新策略；`<provider>:<name>` 只保存引用本身。
- artifact 全局内容寻址，跨网络只存一份；每个配置版本按 key 绑定 digest。inline 内容在配置提交时转换并同样绑定 digest。保留见[artifact 保留](#artifact-retention)。
- 每个 (来源, digest) 保存一份不可变溯源记录：源格式、上游 repo／ref／commit／path（provider 集合）、目录 sequence、`held`（签名目录该集合的 `held`）与转换报告。
- provider 目录按 (目录 URL, 信任钥集合) 持久保存已接受的最大 `sequence` 及验签通过的 `index.json`／`index.json.sig` 原始字节（供[目录搜索](#浏览与命中查询)读取）；`sequence` 更小的目录（即使签名有效）为 `catalog_regressed`，不使用其内容。验签失败为 `signature_invalid`，artifact 与目录条目不符为 `integrity_failed`，目录中没有该集合为 `unknown_rule_set`。
- 更新失败保留最后一份已验证内容并记录 `problem`，不发布半份内容；首次没有可用内容为 `not_ready`，引用它的规则不可执行。
- 回滚使用历史版本绑定的 digest（`ruleSetBindings` 选择 `initial` 或 `refreshed`）；这两类绑定的 artifact 都随配置历史保留，因此历史内容总可用。

### artifact 保留 {#artifact-retention}

保留跟随配置版本历史：配置版本从不删除，未删除网络的每个版本可回滚到的两份绑定（版本自己的初始绑定、该版本当前的刷新绑定）引用的 artifact 一直保留。不再被任何可回滚绑定引用的 artifact——被同一版本的后续刷新替换的中间内容，或只被已删除网络引用的内容——在仍可能处于租约内的 netmap（当前一份，或被替换不足 24 小时的一份；24 小时是租约有效期上限，与 `artifacts/{digest}` 的授权窗口相同）都不再引用它之后，连同其溯源与当前记录一起删除。Server 启动时与此后每小时回收一次。节点 Control 的缓存回收不受影响（只保留当前 netmap 引用的文件）。

## 配置与刷新

Owner／Admin 可在一次配置 apply 中新增集合、provider 与引用它们的路由规则。Server 先验证用户、网络与输入，在 SQLite 事务外有界下载、验签和转换（inline 不需下载），最后同事务重验身份、源 revision、原绑定、目录 sequence 及完整配置权限，存储 artifact 与溯源记录并绑定 digest。失败保留原配置与已验证内容，不把缺失集合当成空集合或跳到后续 direct 规则。下载／转换失败是 422 `configuration_invalid` 的 issue `rule_set_unavailable`（`params.reason` 为 `ResourceProblem`），挂在 `/ruleSets/<key>` 或隐式集合的首个引用。进程内最多 4 项配置准备同时进行；名额用满返回 503。

validate 走相同准备与授权校验，但不存储 artifact、不预留版本；`ConfigurationPreviewResponse.preparedRuleSets` 按 key 返回 `{digest, bytes, entries}`（内容寻址，digest 即最终绑定值）。最终 apply 独立重新准备与重验。

接口相对 `/api/v1/networks/{networkId}`：

| 方法与路径 | 返回 |
| --- | --- |
| `GET /rule-sets` | `RuleSetList`：每个声明的 key 与每个被引用的 `<provider>:<name>` 一项 |
| `GET /rule-sets/{ruleSetKey}` | `RuleSetList`，仅指定集合 |
| `GET /rule-sets/{ruleSetKey}/report` | `RuleSetReport`：绑定内容的转换计数与前 1,000 条跳过条目；未绑定为 404 |
| `POST /rule-sets/{ruleSetKey}/refresh` | 请求体 `RefreshRuleSetRequest { requestId }`；`RuleSetView`，下载、验签、转换、权限重验与绑定后的结果 |
| `GET /rule-set-catalog?provider=&q=&cursor=&limit=` | `RuleSetCatalogPage`：本网络有效 provider 的已验签目录中的集合，见[浏览与命中查询](#浏览与命中查询) |
| `GET /rule-sets/{ruleSetKey}/entries?kind=&q=&cursor=&limit=` | `RuleSetEntryPage`：绑定内容中一类成员的一页；未绑定为 404 |
| `GET /rule-sets/{ruleSetKey}/match?name=` 或 `?ip=` | `RuleSetMatchResponse`：绑定内容是否包含该名称／地址及决定它的成员；未绑定为 404 |

这些资源视图限本网络 Owner／Admin；节点只经 netmap 收到 digest 引用。下载保留 HTTPS 证书／主机名校验，拒绝重定向及非公网解析结果，并固定已验证地址。

`RuleSetView`：

| 字段 | 说明 |
| --- | --- |
| `key` | `ruleSets` key 或 `<provider>:<name>` |
| `configurationRevision` | 当前配置 revision |
| `implicit` | 来自 provider 引用时为 `true` |
| `definition` | 声明的集合定义；隐式集合为 null |
| `provider` | 隐式集合的 `{name, catalog, collection, sequence}`，`sequence` 为绑定 artifact 所在目录序号 |
| `artifact` | `{digest, format, bytes, entries, createdAt}`，`createdAt` 为该内容首次入库时间 |
| `source` | 溯源：`format`，及 provider 集合的 `repo`／`ref`／`commit`／`path` |
| `conversion` | `{converted: {exact, suffix, subdomain, keyword, cidr}, redundant, unconverted}` |
| `held` | 签名目录对该集合的 `held`：发布流水线仍沿用旧 artifact 的 `{since, reason}`；否则 null |
| `problem`／`checkedAt`／`lastSuccessAt` | 最后检查的失败原因（或 `not_ready`）、检查时间与最后成功时间；没有检查记录为 null |

检查时间与 artifact 创建时间不同；失败更新检查时间与错误，保留最后成功的绑定。来源与绑定内容都不变时，检查记录随普通配置版本延续。

手动刷新按 `requestId` 幂等：同一用户在同一网络用同一 `requestId` 重试同一 key 返回当前视图，不再下载；同一 `requestId` 用于另一 key 返回 409 `request_id_conflict`。请求 ID 保留 24 小时。周期检查没有请求 ID。

刷新在同一配置 revision 下改变绑定，受影响节点的 netmap 只改变一个 digest 引用（新 serial、重签租约）。变更通过 `resources.changed` 通知有权管理该网络的 SSE 客户端。

### 浏览与命中查询

三个接口只读 Server 已验签、已绑定的内容，从不下载；权限与其他资源视图相同（本网络 Owner／Admin；成员 403，节点凭据 401）。分页沿用通用约定：`limit` 1–200、默认 100；`nextCursor` 不透明，只对同一用户、网络与相同查询参数有效，期限为该次列表首页起 15 分钟（与其他分页接口同一游标封装与期限，后续页不续期）；目录身份（URL、可信钥集合、collection、`sequence` 与已验签目录原文的 digest）或集合绑定的 digest 改变后旧游标失效；无效游标为 400 `invalid_cursor`，其他参数错误为 400 `invalid_request`。

**目录搜索** `GET /rule-set-catalog`：

- `provider` 必填，为本网络有效的 provider（内置项按 `ruleSetProviders` 替换或禁用后），否则 404；`q` 可省略，为集合名（`<name>`）的前缀，大小写不敏感，最长 128 字符。
- 读取该 provider 的 (目录 URL, 信任钥集合) 最新接受的目录，只列其 `collection` 下的集合，按名称升序。Server 自己保持目录最新（见[目录更新](#catalog-refresh)），不需要先引用任何集合；只有 Server 对该目录的首次取回尚未完成或从未成功时为 409 `rule_set_catalog_not_ready`（不是空列表）。之后的取回失败保留上一份已接受目录。
- `RuleSetCatalogPage`：`provider` 为 `{name, catalog, collection, sequence, generatedAt, verifiedAt}`；`items` 每项为 `{reference, name, set, digest, bytes, entries, upstream: {repo, ref, commit, path}, held}`，`reference` 即 `match.ruleSets` 的写法（`geosite:openai`），`set` 为目录键（`meta-geosite/openai`）；`total` 为匹配 `q` 的集合数。列出的 digest 是目录当前发布的内容，可能与某网络已绑定的不同。

**成员列表** `GET /rule-sets/{ruleSetKey}/entries`：

- 读取该 key（声明集合或 `<provider>:<name>` 隐式集合）当前绑定的 artifact；`kind` 必填，一次只列一类：

| `kind` | 成员 | 顺序 | `q` |
| --- | --- | --- | --- |
| `domain` | `{kind: exact\|suffix\|subdomain, name}` | 按反转名称（共享后缀的名称相邻） | 名称以 `q` 结尾（按字节；`openai.com` 列出它本身与其下所有名称，`.openai.com` 只列其下名称），`q` 先小写、去结尾点 |
| `keyword` | `{kind: keyword, keyword}` | 升序 | keyword 以 `q` 开头（小写后） |
| `ipv4`／`ipv6` | `{kind: ipRange, first, last}` | 升序 | 一个该族地址：从包含它的区间或其后第一个区间开始 |

- IP 成员按 artifact 原样以闭区间列出：相邻或重叠的源 CIDR 已合并，一个区间可对应多个 CIDR。`counts` 给出各类成员总数（`exact`／`suffix`／`subdomain`／`keyword`／`ipv4`／`ipv6`，后两项为区间数）。

**命中查询** `GET /rule-sets/{ruleSetKey}/match`：

- `name` 与 `ip` 恰给一个。名称按路由的查询名口径规范化（ASCII 小写、去一个结尾点，不做 DNS 名称校验，含 `_` 的名称照常判定），规范化后为空或超过 253 字节为 400；`ip` 为 IPv4 或 IPv6 地址。
- 判定与节点相同（`meshway-rules` matcher）：名称先从最短的后缀／只子名称成员试起，再精确名称，最后 keyword；`member` 为决定命中的成员（`suffix`／`subdomain` 的 `name` 是被命中的成员名，`keyword` 为该 keyword，地址为所在区间），未命中为 null。`query` 为实际判定的规范值。
- 只回答集合本身是否包含查询；该集合在路由中是否决定流量还取决于规则顺序与 profile，用[路径检测](/v2/configuration-reference/routing)判断。

## 下发

- netmap 投影的 `ruleSets` 为 `key → {digest, format, bytes, entries}`，只含被保留的公共或 profile 规则引用的 key；集合定义（URL、目录、inline 列表）不进入 netmap。见 [Netmap 设计](/v2/node/netmap-design#rule-set-artifacts)。
- 节点 Control 以节点会话 `GET /agent-api/v1/artifacts/{digest}` 取 artifact：只有该节点某份仍可能在租约内的 netmap（当前一份，或被替换不足一个租约有效期的一份）引用的 digest 返回 200 与原始字节（`application/vnd.meshway.rules.v1`，`ETag` 为 digest，`Cache-Control: private, max-age=31536000, immutable`），其他一律 404，不区分“不存在”与“无权”。
- 节点检查长度、SHA-256 与完整结构后，以 0400 只读文件 `identity/artifacts/sha256-<hex>.mwrs` 原子写入私有目录，并只保留当前 netmap 引用的文件；重启离线时从该缓存加载。每节点同时最多 2 个下载，失败按 5 秒起、上限 5 分钟退避。
- 新 netmap 立即生效，不等待 artifact：撤销与收紧不被下载阻塞。引用尚未取得的集合的规则不可用：first-match 在该规则停止（不跳到后续规则、不当作空集合、不 direct），源节点自己的出口方案依赖缺失集合时，该方案为不可用：出口捕获保持全范围捕获与 IPv6 sink、不发布 direct 例外，resolver socket 一律拒绝，与其他不可用出口方案一样本次安装报告失败（负向边界保持）。artifact 取得后 Control 以同一 netmap 再发布，自动重新应用，不需第二次 `up`。`meshway status --json` 的 `netmap.missingArtifacts` 与 `netmap.artifactProblem` 报告缺失的 digest 与最后错误（`download`／`integrity`／`storage`／`budget`）。

## 周期更新与生命周期

### 目录更新 {#catalog-refresh}

Server 启动后及周期调度中，对任一网络当前生效的每个 provider（内置项按 `ruleSetProviders` 替换或禁用后）自己取回 `index.json` 与签名并验签，不取 artifact。同一 (目录 URL, 信任钥集合) 是一个目录：无论多少网络引用，一次取回服务全部网络，期限取这些网络中最短的 `updateInterval`；集合准备（配置提交、手动或周期刷新）时验签通过的同一目录也算一次成功取回。期限、退避与远端集合相同：从上次成功起 `max(updateInterval, 1s)`，失败从 5 秒指数退避至 5 分钟，取回时间与失败次数持久保存。与其他下载共用 4 项传输预算。验签失败、取不到或 `sequence` 小于已接受值（`catalog_regressed`）都保留上一份已接受目录；接受新目录不改变任何网络的绑定，绑定只随集合刷新改变。内置目录约 2 MB，每天一次。

### 集合检查

Server 的周期调度器以当前网络配置、持久检查记录与绑定为输入，使用内部任务资格，不持有管理员会话。网络按轮转顺序检查，网内优先处理最久未检查的到期集合（remote 与 provider 集合；inline 不需更新）。调度下界为 1 秒；每轮检查间隔至少 100 ms，每个实例同时执行一项周期下载，所有配置／手动／周期调用共用进程内 4 项实际下载预算。同一来源的并发下载合并，最多同时接纳 8 项刷新工作；一次配置准备内的多个 provider 集合共用一次目录下载与验签。

正常期限从 `checkedAt` 加 `max(updateInterval, 1s)` 计算。失败按连续次数从 5 秒指数退避、上限 5 分钟，次数和时间持久保存，重启不清除退避。只有调整间隔且来源不变时，沿用绑定与检查记录。

内容不变（同一 digest）时只更新检查时间与结果；内容变化才改变绑定并重新编译受影响的 netmap。最终事务比较原检查截点，旧下载结果不能覆盖较新的刷新或配置提交。App shutdown／drop 取消网络工作，正常 shutdown join 调度器后释放存储；已接纳的 SQLite 事务仍完成提交与事件发布。

资源绑定不证明节点已取得 artifact 或应用配置；使用节点 Status、实际回执与业务观测判断运行结果。
