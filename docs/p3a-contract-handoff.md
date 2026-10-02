# P3a 契约生成与后续接线交接

本切片补齐 TS runtime schema 的 tagged union、typed map、字符串 const 与属性预算支持；现有 HTTP OpenAPI、生成产物与操作签名保持字节兼容。未接入 v3 管理 API、节点执行快照或 Linux 数据面；不表示 P3a 完成。

## 真实源 schema 与已完成修复

`CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=target/p3a-codegen cargo run --locked --quiet -p meshway-policy --example policy_v3_schema` 输出真实 `NetworkPolicy` JSON Schema。当前使用 `oneOf`、字符串 singleton enum、typed `additionalProperties`、字符串 `propertyNames` 和 `maxProperties`；没有 `allOf` / `const`。const 仅补齐下一步显式 tagged DTO 的字符串 literal 表达。

`ProfileFallback` 的 variant 已通过 maxProperties 封闭，`AutomaticRouting.regions` 可生成并执行验证。`ExitMatch` 的 utoipa variant 原先未继承 serde 的 `deny_unknown_fields`，缺少封闭约束。**utoipa 6 不接受 tuple variant 上的 `max_properties` 注解**；第一提交建议的三行注解不可编译，已撤销该建议。

本切片第二提交在权威源 `crates/policy/src/v3/mod.rs` 为 `ExitMatch` 显式实现 `PartialSchema` / `ToSchema`，使用 typed `ObjectBuilder` / `OneOfBuilder` 构造三个 variant：字符串 singleton enum `kind`、原生 String schema 的 `value`，两字段必填，`additionalProperties: false`、`maxProperties: 2`。保留现有 tuple Rust API、serde tag/content/name 和序列化字节，不新增源 DTO 或依赖，不在生产生成器按名称修补 schema。

Rust wire/schema test 对照三个实际 serde variant 的序列化字节、反序列化、缺失或未知字段、null / 错误类型 / unknown tag 拒绝，并核对实际导出的属性、required 与封闭约束。生成器行为测试直接消费未经修补的完整真实 `NetworkPolicy` schema：合法 / 非法实例经过生成 Zod，且 Hey API DTO、Zod 输出和生成操作共同通过严格 TypeScript 编译。没有 fixture 自动加约束；人工构造的开放 variant 仍作为 fail-closed 负例。

根命令建议：协调者在根 `package.json` scripts 加 `"test:codegen": "node --test scripts/generate-protocol.test.mjs"`，并将它纳入合并检查；本切片未改根 manifest / lock / CI。当前可直接运行该命令。

## 三个边界

1. **源策略**：`NetworkPolicy` 是可编辑 / 保存 / 迁移的用户配置；管理 DTO 携带它、CAS 基准、原始 JSONC 和诊断。字段验证不替代 policy 编译的引用、授权和预算校验。
2. **compiler internal**：`NetworkCompilation` / 索引 / matcher / planner 状态是可信进程内输出，不能通过 Deserialize 或节点客户端重新构造，更不能直接当 stable node wire DTO 下发。共享缓存与安装预算仍按源、方案及最终安装结果计数，任何错误拒绝整个发布。
3. **stable node execution snapshot**：另建显式、封闭、版本化 DTO，从编译成功输出投影本节点的执行所需计划、授权路径、端点公钥与 typed carrier 参数。生成客户端负责解码，Agent 仍须防御性验证身份、revision / digest、资源限额、支持能力及本机资源。schema 验证不是执行成功证据，ACK 只代表校验与持久保存。

`meshway-policy` 已依赖 `meshway-protocol`，禁止让 protocol 反向依赖 policy。最小可行接线是管理源 DTO 先留在 policy 的专用 source/API 模块（或 controller 的管理 envelope 引用 policy 源类型），Controller 实际路由注册这些 schemas；stable 节点 wire DTO 放现有 protocol 模块，policy / controller 单向投影到它。源类型是否后续整体下沉到 protocol 应单独迁移，不手工复制 NetworkPolicy。

## Controller publication / 迁移 / 协商

- Controller 必须在同一网络 revision 快照中捕获策略源、成员 / 标签、连接事实、能力及公钥，整体编译 / 投影 / 校验后原子发布；不能组合不同 revision 的业务配置与连接 / 公钥。保存 CAS、节点 lease / 撤回及代际停止按真实生命周期落实。
- 旧 v2 迁移必须无损。不能等价表达的 ordered allow/reject、路径 / fallback 或其他旧语义应报告诊断并保留旧源及已发布配置；不部分迁移、不丢弃拒绝规则、不自动改成 permissive direct。成功迁移与新 publication 必须整体完成。
- Controller 的可用协议目录不等于 Agent 实际编译支持能力。stable snapshot 需显式 schema / execution 版本与所需能力，Controller 在发布前匹配，Agent 在执行前复核。旧 Agent 不得把 v3 计划重新解释成旧 v2 或未经确认的降级；不兼容应拒绝新计划并按已定义 lease / 撤回边界处理，不能无限保留已失效代际。
- 下一步 typed carrier 应使用封闭 tagged object（required 字符串 kind + 唯一 enum/const + 每 variant 参数），不使用 string 协议名加任意 JSON 配置。该 fixture 仅验证生成形状，不定义新承载参数或产品模型。域名上下文 / 完整出口分流仍属 P3c。

## Rust 节点生成的真实限制

本切片不扩张 Agent 操作白名单。现有 `generate-rust-client/closed_objects.rs` 只接收显式封闭、带 properties 的 object；typed map 或仅靠 maxProperties 封闭的对象目前会失败。`required_nullable` 的模型遍历、上游 tagged enum 的 serde 表达及 unknown-field 行为，须在 stable DTO 真正进入所选节点操作时增加实际生成 / 解码测试并适配，不能因为 TS 已支持就宣称 Rust 已支持。管理源 DTO 不应为绕过这些限制加入 Agent 白名单。

验证入口：`node --test scripts/generate-protocol.test.mjs`、`pnpm protocol:check`、SDK typecheck/tests、Web / Apple scripts 与五个 experiments 的非浏览器 TS 检查。Rust 全工作区消费者与新 publication / TUN 行为验证由协调者在实际接线整合时运行。
