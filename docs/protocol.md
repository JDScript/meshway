# HTTP 契约与类型安全

**Rust DTO、实际路由声明与服务端业务校验是权威来源。** OpenAPI、Agent Rust client、TypeScript SDK、运行时与编辑器 schema 由同一生成链维护，不手抄 DTO/endpoint，不以文档快照代替正在运行的合同。

## 来源与生成入口

```text
Rust 契约 + Controller 实际 routes
                 ↓
packages/protocol/openapi.json
           ↙             ↘
Agent Rust client      TS SDK + Zod / JSON Schema
```

- `crates/protocol/src/`：共享 HTTP/执行 DTO；`crates/policy/src/v3/`：策略源与纯编译器。
- `apps/controller/src/http/`：实际操作、认证声明、请求/响应；独立于 handler 的控制面契约位于 `src/contracts/`。
- `apps/controller/src/bin/generate-rust-client/main.rs`：节点操作白名单与 checked 生成适配。
- `scripts/generate-protocol.mjs`：统一导出/生成；产物位于 `apps/agent/src/generated`、`packages/control-client/src/generated`。

```sh
pnpm protocol:generate
pnpm protocol:check
pnpm test:codegen
pnpm test:protocol
```

drift check 临时重生成并逐文件比较，不直接编辑产物。完整模型/操作/字段按需[重现导出](/model-api-audit)，不常驻一份巨型重复 dump。检查全部消费者的要求见[开发与验收](/development)。

## 权限与作用域

| 主体 | 权限边界 |
| --- | --- |
| `userSession` | 不透明 `mws_` 会话，账户本人操作与设备批准 |
| `managementBearer` | 用户当前网络 Read/Manage；保留实例 bootstrap 兼容入口 |
| `adminBearer` | 实例初始化/恢复等专属操作，网络 Owner 不反向获得此权限 |
| `enrollmentBearer` | 原子接入绑定，不授予管理或已有节点权限 |
| `nodeBearer` | 凭据推导自身节点/网络，下载本节点配置与获准材料、报告/ACK |

管理资源使用 `/api/v1/networks/{network_id}/…`，在实际根事务内重新检查 session、过期、credential version、membership/role；网络 ID 本身不是授权。Agent/App 由凭据推导网络，不接受任选身份。日常 Web 登录为用户账户，节点私钥/材料不进入浏览器。

## 当前源与执行合同

多网络、账户、托管承载与 IPv4 Mesh v3 管理/执行操作均已合入。旧 `policy` / `access` 等兼容 API 保留旧源语义；`policy-v3` 显式激活新源模式，不能把旧接口改口称为 grants。具体 [Mesh 执行合同](/p3a-mesh-execution)、[账户](/local-accounts)、[listener/材料](/managed-transports-api)按职责说明行为。

`NetworkPolicy` 是编辑源，compiler internal 不直接反序列化为可信计划；节点执行是独立版本化、封闭 DTO。publication 在同事务读取策略、库存、能力、邻接与公钥，投影同 revision 的完整配置。Agent 仍验证网络/身份/地址、digest、路径与容量及本机资源。

网络内 CAS 使用 `expected_revision`，冲突不覆盖草稿；JSONC 和 typed policy 必须等价，注释编辑也需 CAS。ACK 是持久保存回执，不是 applied。历史缓存只提供回滚下限，没有 fresh HTTP 不启动执行/续租；更高 revision、撤回或 lease 到期独立 fence 旧运行，不等待阻塞请求。

## 严格解码与兼容

- Rust 输入拒绝未知字段；节点成功响应按 schema 的 required/nullable 与封闭对象严格解码。非 2xx status-first，401/403/409 不等待错误 body，成功 body 有界。
- TS 请求/成功响应使用生成 Zod，类型声明不能代替运行时检查。缺失、null、未知、不适用、失效是不同语义；不能补 0、当前时间或伪造状态。
- 封闭对象、唯一判别 tagged union、typed map、字符串约束与属性预算由生成器检查；不支持的形状生成失败，不用任意 JSON/`any`/未验证断言绕过。合法 map own keys（含 `__proto__`）安全保留。
- 成功 DTO 的 strict unknown-field 意味着新增字段也可能不兼容；需要新操作/版本、显式协商或协调全部消费者升级。策略 v3 不意味着全部 HTTP 路径改成 `/api/v3`。
- 旧 App exchange/heartbeat/configuration/DNS 与浏览器批准路径保持兼容。数据面 wire/能力与 HTTP 成功 DTO 的演进分别处理。

SDK `ControlApiError.status` 保留非成功 HTTP 状态，当前错误 body 不回显；schema/媒体类型/网络错误不会伪装成认证拒绝。每个操作的确切字段、status/security 见生成 OpenAPI，业务引用/资源/权限/总预算还须查实际 validator。

## HTTP 与观测

配置 revision、runtime epoch、report sequence、socket/sampler generation 分属不同生命周期。`last_seen` 只说明控制面联系，候选不表示可达，preview 不代表授权或实际 trace。观测在没有新 revision/sequence 时也会老化；历史只来自新真实测量。

SPA 的登录上下文、草稿/CAS、分页与轮询边界见[接口消费](/spa-api-audit)。WASM 仅是未来复用策略逻辑的可选方式，HTTP 类型安全不依赖它。
