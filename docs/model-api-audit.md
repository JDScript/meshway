# 模型、schema 与 API 导出

当前 HTTP 定义以 [Rust 实际路由生成链](/protocol)为准；SQLite 结构以当前 Controller 安装/验证的迁移定义为准。导出是按需审查工具，不维护另一份“最新”模型大全，也不把历史冻结基线的统计作为现行接口数量。

## 从当前 checkout 重现

在仓库根执行，输出到忽略的 `target`；独立 Cargo target 避免替换并行 CLI 验收二进制：

```sh
export CARGO_BUILD_JOBS=2
export CARGO_TARGET_DIR="$PWD/target/contract-export-build"
OUTPUT="$PWD/target/contract-export"
SOURCE_REF="$(git rev-parse HEAD)"
mkdir -p "$OUTPUT/policy-source"

cargo build --locked -p meshway-controller --bin meshway-controller
"$CARGO_TARGET_DIR/debug/meshway-controller" export-openapi --output "$OUTPUT/openapi.actual.json"
cmp "$OUTPUT/openapi.actual.json" packages/protocol/openapi.json
python3 scripts/dump-contract.py "$OUTPUT/openapi.actual.json" --output "$OUTPUT/http" --source-ref "$SOURCE_REF"

cargo run --locked --quiet -p meshway-policy --example policy_v3_schema > "$OUTPUT/policy-source/schema.json"
python3 scripts/dump-contract.py "$OUTPUT/policy-source/schema.json" --output "$OUTPUT/policy-source" --source-ref "$SOURCE_REF-policy-source"
python3 scripts/dump-store-schema.py --controller-bin "$CARGO_TARGET_DIR/debug/meshway-controller" --output "$OUTPUT/storage" --source-ref "$SOURCE_REF"
```

`dump-contract.py` 导出模型、全部字段/约束和操作关系；策略源 schema 不自动等于 HTTP 或节点执行 DTO。`dump-store-schema.py` 创建临时 store/回环进程，读取结构后停止并清理，不访问现网 DB 或数据面。固定重现使用仓库内 `.worktree/<任务名>` 的明确 commit，并记录 dirty diff、工具/源码摘要；发布契约和未合入候选分别导出。

## 字段审查必须保留的语义

- required、可省略与 nullable 分开；私钥对 dialer 为 null 是最小披露，不能补发 listener 私钥。没有公钥/ACK/观测不意味着对应数据库列可随意变空。
- 网络内 CAS、runtime epoch、sequence、measurement ID、socket generation 不可合并为一个 version。时间来源、单位、精度、有效期和缺测理由分别解释。
- schema 校验不替代跨字段、资源引用、权限、同 revision 与总预算；OpenAPI 可能尚未表达服务端全部 validator 限制。
- 旧成功 DTO strict unknown-field，新字段也须消费者迁移；持久化 wire/材料 JSON 的条件分支变化另审兼容。
- JSONC 省略/null 的保存行为以当前接口为准；不能默改成 preserve。源与执行/派生视图分开，不能因清理丢失管理员保存意图。
- latency cursor 只用于固定窗口分页，时间精度按现有毫秒持久化合同；[SPA 消费边界](/spa-api-audit)说明扩大窗口的风险。

冻结 `2132434` 的原专项报告保留于批准 TMP：`meshway-model-schema-audit-2132434.md`、`meshway-spa-api-audit-2132434.md` / `-addendum.md` 与 `model-api-audit-export/`。这些是历史发现的追溯入口，不代表其中建议已实施；现行行为要从当前权威源核对。源码组织清理同样不能用“导出/审查完成”代替。
