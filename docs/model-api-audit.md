# 模型、schema 与 SPA 接口审查

冻结基线：`2132434`。本页盘点已发布 HTTP 契约、已合入但未接 HTTP 的 v3 策略源，
以及从该版本 Controller 新建临时数据库导出的 SQL 结构。开发中的 v3 执行合同不混入本快照。
导出只包含定义，没有现网业务行、账户、凭据或节点材料。

后续主线已合入 [IPv4 Mesh v3 执行合同](/p3a-mesh-execution)：新增 24 schemas / 4 operations，
总计 185 schemas / 62 paths / 71 operations，原有定义逐项完全相同。
本页导出继续固定在 `2132434`，不将其 161 / 67 统计冒充新主线数量；新增合同的独立审查和兼容验收见[关闭记录](/p3a-review-fixes)。

## 完整目录

| 范围 | 数量 | 阅读入口 |
| --- | --- | --- |
| 当前 HTTP | 161 schemas；58 paths / 67 operations（25 GET、42 POST） | [模型与全部字段](/contract-audit/2132434/http/models)、[全部接口](/contract-audit/2132434/http/operations) |
| v3 策略源 | 12 schemas、45 个对象 / 分支字段；不是已发布 HTTP API | [源模型字段](/contract-audit/2132434/policy-v3/models) |
| SQLite v11 | 29 张表、148 列、54 个显式 schema objects | [SQL 字段与外键](/contract-audit/2132434/storage/models) |

仓库中的机器可读文件：

- `docs/contract-audit/2132434/http/inventory.json`：完整 schema / operation 定义、引用关系和字段分类。
- 同目录 `fields.csv` / `operations.csv`：可排序的字段与接口清单。
- `docs/contract-audit/2132434/policy-v3/schema.json` 及 `inventory.json`：真实导出的 v3 源 schema 与字段。
- `docs/contract-audit/2132434/storage/schema.sql`、`inventory.json`、`columns.csv`：DDL、表 / 字段 / FK / 索引结构。

这些是带版本的审查快照，不是另一份手工 DTO、安装脚本或迁移来源。正式 HTTP 权威仍是 Rust
类型和实际路由及其生成的 `packages/protocol/openapi.json`。

## 先区分 optional 与 nullable

当前 HTTP 的 **477 个对象字段中，472 个必填，只有 5 个可省略**：

| 字段 | 可省略 | 可 null | 当前用途 |
| --- | --- | --- | --- |
| `ClaimOwnerRequest.email` | 是 | 是 | 新账户认领分支的邮箱 |
| `ClaimOwnerRequest.password` | 是 | 是 | 新账户认领分支的密码 |
| `ClaimOwnerRequest.session_token` | 是 | 是 | 已有账户认领分支的身份凭据 |
| `SaveMeshAccessRequest.source_jsonc` | 是 | 是 | 授权源文本 |
| `SavePolicyRequest.source_jsonc` | 是 | 是 | 路由源文本 |

另有 **52 个必填但可 null** 的字段，因此总共 57 个可 null。`null` 常用于未配置、未观测、
不适用或历史状态，但这些含义是否应合并，需要结合具体模型审查；不能直接删去所有 null。
本轮已将全部 477 项的可空 / 可省略标记与当前生成 Zod 实际验证行为逐项比对，结果一致。
这只证明这些结构标记一致，不替代完整业务校验与状态组合审查。

v3 源的两个可省略字段是 `ExitRule.src` 与 `NetworkPolicy.automatic`，均不允许 null；
省略时的 serde 默认与显式 null 是不同输入。分支字段的 required 仅适用于所属分支。
SQL 的 `notnull` 是 PRAGMA 标记，还需结合主键、STRICT、CHECK、trigger 和 JSON 列内部模型判断。

## 审查重点

Pure SPA 专项已完成并校准：[接口消费、会话与轮询审查](/spa-api-audit)，附[全部 67 个操作的使用建议](/contract-audit/2132434/spa-usage)。
字段 / schema 专项已完成。全部 57 个 nullable 字段已逐项归类并与导出集合核对：
[空值语义与处置目录](/contract-audit/2132434/nullable-review)，同目录另有 `nullable-review.csv`。

核对范围：

- null / 缺省是否有唯一含义；输入 / 输出 DTO、状态组合、默认值、范围 / 单位和约束是否一致。
- SPA 的登录 / 会话、网络权限、跨登录响应 / cache 隔离如何由 HTTP 契约承接；不假设 Next Server Action 的保护会自动存在于 Controller。
- 配置读取、动态状态轮询、按需历史分页和 mutation 的接口分工；观测过期与配置 revision 分开。
- 类型化错误与幂等重试；一次性秘密、创建 / 撤回操作不作为周期轮询。
- ETag / 304、增量游标若引入，必须正确处理授权变化、无新 sequence 的过期和测量时间乱序。

当前 `apps/console` 由用户的独立工作流维护；本次不修改或审读其活跃实现，只审后端合同与 SPA 所需边界。

## 字段专项：八项需要收敛的边界

这是对冻结实现的审查，建议尚未应用到生产契约。多数问题是 schema 比 Controller 业务校验宽，
不是服务器允许无效配置；不要把收紧输入与删除有效未知 / 不适用状态混为一谈。

| 项目 | 当前具体问题 | 建议与迁移边界 |
| --- | --- | --- |
| 输入 / 输出状态复用 | runtime 报告 schema 接受 `awaiting_report`，discovery 报告接受 `expired`；Controller 明确拒绝这些 server-only 状态 | 拆 reported / view 状态类型，保留合法旧字符串；复杂 state / reason / resources 先内部校验成明确类型 |
| listener 条件参数 | `transport=vless` 可在 schema 中配 `vless=null`，native 可配 VLESS 参数；TCP/WS 参数组合也只靠业务校验 | 按 transport / stream 表达条件分支；保留旧 wire 适配。settings JSON 还进入数据库及材料密文，不能只改一个 enum 就假定无迁移 |
| 初始 listener 身份 | 注册令牌的 initial_listener 要传空 node_id，编辑已绑定 listener 又要求真实节点引用，却复用同一 DTO | 新输入拆成不含 node_id 的 initial listener，身份由注册事务填充；已绑定配置使用明确引用 |
| Owner 认领 | 平铺 kind 加三个可选 credential 字段，`{kind:new_account}` 可通过 schema，再由 handler 拒绝 | 规范输入用 new-account / existing-account 分支；旧请求的无关显式 null 与缺失当前等价，须保留兼容或发布新输入合同 |
| ownership 空值来源 | 注释把 membership_id=null 称为 legacy，但 AFTER INSERT trigger 也给当前新 token 节点产生 null | 描述改为“尚未绑定账户 membership”，保留 null；不能把所有旧空值回填为 legacy，也不自动认领到当前管理员 |
| JSONC 保存 | source_jsonc 缺失和 null 都清除旧文本，不是 preserve；带 source 时保存 typed 原意图，不带时可能保存 normalized policy | 明确完整替换语义。若增加 preserve/clear/replace 用显式命令或新版本；不能静默改变 omission 含义 |
| 声明约束不足 | 账户 TTL / ID、密码字节数、STUN RTT 条件和 cursor 范围部分只在 validator 中 | 先补已存在的静态上限 / 格式 / 单位；UTF-8 bytes、Unicode code points、UTF-16 长度分开，不能错用 minLength |
| 时间精度与 v3 默认 | latency body / 输入可有亚毫秒时间，SQL 索引 / 边界截断为毫秒；v3 空 Vec、Default 与默认模板含义不同 | 历史精度须与分页一起明确；v3 发布前固定默认 / 条件分支 / 预算。空 grants 不能在反序列化时偷偷变为全允许 |

上表为便于交付合并了原专项的相邻项；完整证据见批准 TMP 的
`meshway-model-schema-audit-2132434.md`，包括具体 Rust 行号与 JSON Pointer。

### 可直接看出的反例

协调者对当前生成 Zod 实跑确认以下七种结构输入仍被 schema 接受：server-only runtime / discovery 状态、
VLESS 缺配置、native 携带 VLESS 配置、new-owner 缺分支必填项、邀请 TTL 超过七天、超 i64 的 20 位游标。
Controller 拒绝这些输入的判断来自冻结 validator / handler；本次未向现网发送这些 mutation。
证据在 `model-api-audit-export/schema-relationship-proof.json`。

可优先对齐的具体约束：

| 字段组 | 服务器当前已经执行的规则 |
| --- | --- |
| 邀请 expires_in_seconds | 1–604800 秒 |
| 密码重置 expires_in_seconds | 1–3600 秒 |
| user / membership / invitation / reset ID | 64 位小写十六进制标识，各自类型与资源作用域仍分开 |
| 密码输入 | 12–1024 UTF-8 字节，不能直接写成 minLength=12 字符 |
| STUN mapped 的 rtt_ms | 必须有值且不超过 1500；失败时为空，不能只表达为任意 nullable u32 |
| history after | 非负 i64 的十进制字符串，长度 20 本身不足以保证数值可接受 |

这些规则的声明补齐不替代邮箱规范化、资源存在性、权限、同 revision 和总发布预算等业务校验。

SQL 精度的例子：样本在 `T+100µs`，查询 `[T,T+500µs)` 本应包含，但截断为毫秒后变为空窗；
查询 `[T+500µs,T+1000µs)` 本应排除，却可能命中同一 `measured_ms`。
这是 wall-clock 索引 / 查询边界问题，不能据此改变单调时钟的 `rtt_us` 或重写历史 measurement identity。
可以明确毫秒级边界并处理现有客户端，或改查询 / 索引以支持更高精度；仅修改文案不算修复。

### 应保留的模型语义

- `private_key_pem=null` 对拨号端是有意的保密投影，不能为了非空而下发 listener 私钥。
- 无 public key 可能是整行 identity 尚不存在；无 ACK 是没有 receipt，不是 SQL 对应列允许 NULL。
- `FlowQuery` 的 node+IP、domain+IP 是合法上下文组合，不能简单改成互斥三选一。
- config revision、runtime epoch、各报告 sequence、socket / sampler generation 是不同层次，不能统一成一个 version。
- current / stale / reported_at / expires_at 分别表达配置匹配、时间与来源；不用 0 或当前时间填补未知。
- typed policy 与 JSONC 原文共同保存有用途，服务端 equality 与 CAS 是必须保留的约束。

## 清理顺序

**交付边界补充（2026-10-02）**：上述完成项是完整导出与审查，源码目录、模块命名及迁移组织的收敛尚未交付。
用户再次强调源码整洁。当前 `store/policy_v3.rs` 承担策略源存取、迁移提案及执行 epoch 协商，
版本名掩盖了领域职责；`schema_v9/v10/v12.sql` 为运行时安装 / 迁移片段，`schema_v8_test.sql` 仅为历史测试 fixture，
二者不应以相似名称散放在业务入口。后续按职责整理生产模块、集中迁移及测试支持，保留必要历史兼容和行为回归。
`#[cfg(test)]` 的领域测试本身是正常 Rust 组织方式，不会编入生产 binary；目录收敛不能只以测试文件数量判断质量。

1. **先明确已有行为**：ownership 来源、JSONC 替换、时间单位 / 精度、当前 / 过期语义；同步权威字段说明及生成物。
2. **收紧服务器本就拒绝的输入**：server-only enum、账户 TTL / ID 等；检查新客户端提前校验的错误路径。
3. **按用途拆条件输入**：ClaimOwner、initial listener、transport / stream、outcome；旧输出和持久化 JSON 用版本 / 显式适配过渡。
4. **跟随 v3 发布**固定空策略与默认模板、source / facts / compiler / node execution 边界及总预算；避免再次为生成器局限牺牲类型语义。
5. **再改有持久化影响的结构**：保持 ID、密钥、摘要、receipt、历史 cursor / 去重绑定，不能仅按 Option 数量大改表。

本轮 SQL 导出覆盖全部结构，字段专项核对了相关业务映射和两个内存 SQLite 反例，
不声称已逐列证明 148 列的所有业务写入路径。现有字段及接口未因审查而改变。

## 重现导出

在仓库根目录创建独立冻结 checkout；下面的输出位于忽略的 `target`，不会覆盖已保存快照：

```sh
git worktree add --detach .worktree/model-api-reproduce 2132434
ROOT="$PWD"
SNAPSHOT="$ROOT/.worktree/model-api-reproduce"
OUTPUT="$ROOT/target/model-api-audit"
export CARGO_TARGET_DIR="$SNAPSHOT/target/model-audit"
export CARGO_BUILD_JOBS=2
mkdir -p "$OUTPUT/policy-v3"

cargo build --locked --manifest-path "$SNAPSHOT/Cargo.toml" -p meshway-controller --bin meshway-controller
"$CARGO_TARGET_DIR/debug/meshway-controller" export-openapi --output "$OUTPUT/openapi.actual.json"
cmp "$OUTPUT/openapi.actual.json" "$SNAPSHOT/packages/protocol/openapi.json"
python3 scripts/dump-contract.py "$OUTPUT/openapi.actual.json" --output "$OUTPUT/http" --source-ref 2132434

cargo run --locked --quiet --manifest-path "$SNAPSHOT/Cargo.toml" -p meshway-policy --example policy_v3_schema > "$OUTPUT/policy-v3/schema.json"
python3 scripts/dump-contract.py "$OUTPUT/policy-v3/schema.json" --output "$OUTPUT/policy-v3" --source-ref 2132434-policy-source-not-http
python3 scripts/dump-store-schema.py --controller-bin "$CARGO_TARGET_DIR/debug/meshway-controller" --output "$OUTPUT/storage" --source-ref 2132434
```

SQL 工具自行创建临时 store、分配回环端口，读取 schema 后停止进程并删除临时数据；
只写结构化导出。数据面、Docker、现网配置均不参与此次导出。
原始构建 / 导出记录位于批准 TMP 的 `model-api-audit-export/`；实际路由导出与提交 OpenAPI 逐字节一致。
