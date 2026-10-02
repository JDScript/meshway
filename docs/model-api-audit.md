# 模型、schema 与 SPA 接口审查

冻结基线：`2132434`。本页盘点已发布 HTTP 契约、已合入但未接 HTTP 的 v3 策略源，
以及从该版本 Controller 新建临时数据库导出的 SQL 结构。开发中的 v3 执行合同不混入本快照。
导出只包含定义，没有现网业务行、账户、凭据或节点材料。

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

字段 / schema 与 pure SPA 专项正在基于同一冻结版本审查，具体发现和迁移建议将汇入本页：

- null / 缺省是否有唯一含义；输入 / 输出 DTO、状态组合、默认值、范围 / 单位和约束是否一致。
- SPA 的登录 / 会话、网络权限、跨登录响应 / cache 隔离如何由 HTTP 契约承接；不假设 Next Server Action 的保护会自动存在于 Controller。
- 配置读取、动态状态轮询、按需历史分页和 mutation 的接口分工；观测过期与配置 revision 分开。
- 类型化错误与幂等重试；一次性秘密、创建 / 撤回操作不作为周期轮询。
- ETag / 304、增量游标若引入，必须正确处理授权变化、无新 sequence 的过期和测量时间乱序。

当前 `apps/console` 由用户的独立工作流维护；本次不修改或审读其活跃实现，只审后端合同与 SPA 所需边界。

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
