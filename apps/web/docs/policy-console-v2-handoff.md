# Policy console v2：语义与交付交接（提案）

状态：**提案，未确认**。本文件只放在 `apps/web/docs`，不属于正式文档站；访问语义、标签模型、中转许可由 main 与用户决定。前端不得把本文中的新字段当作已实现。

基线：`a02a6f4`，分支 `feat/policy-console-v2`。

## 1. 现状（以生成契约为准）

| 面 | 当前契约 | 问题 |
| --- | --- | --- |
| 访问 | `MeshAccessPolicy { grants: [{ id, path: NodeID[2..8] }] }`，≤128 条 | 授权 = 完整有序路径，与路由结果**精确相等**才生效。3 节点双向全通需要 6 条路径；增加 via 要重写授权；无 `*`、无标签、无默认动作字段（隐式 reject）。 |
| 路由 | `RoutingPolicy { default_action: direct\|reject, rules: [{ id, source{node_ids,countries}, destination_scope, destination_any[], action{kind, via[], exit_node_id} }] }` | 已是有序 first-match，结构合理；缺可选 `name`/`description`，没有可复用的源组。 |
| 预览 | `POST previewPolicy(flow, policy)` → `RouteDecision` | 只算路由，不算访问授权；不是 trace。 |
| 观测 | `getConnectionRuntimes`：邻接 nonce 探测（`probe_fresh`、`probe_rtt_ms`） | 只证明邻接通道，不证明业务流量、TUN 安装或授权。没有流量/吞吐 API。 |
| Schema | `zod.gen.ts` 中的 `zMeshAccessPolicy`、`zRoutingPolicy` 等 | **`@meshway/control-client` 的 `exports` 只暴露操作和类型，Web 无法导入 Zod schema**；Web 也没有直接依赖 `zod`。 |

`zod@4.6.5` 的 `z.toJSONSchema(zRoutingPolicy)` 可用（已在本地验证），但会丢失 `.refine()` 的长度约束和 OpenAPI 的 `description`。OpenAPI `components.schemas` 本身就是 JSON Schema 2020-12，带 `description`、`minLength/maxLength`，更适合补全和悬停帮助。

## 2. 访问语义提案（供 main / 用户决策，前端不冻结）

目标：访问回答"谁可以访问谁"；路由回答"走哪条路"；中转许可回答"谁愿意替谁转发"。三者取交集：

```
effective(src→dst) =
  route(src, dst) 产生路径 P = [src, h1..hn, dst]（或 reject）
  ∧ access(src, dst) = allow
  ∧ ∀ hi ∈ P 中间节点: transit(hi, src, dst) = allow
```

候选契约（命名待定）：

```jsonc
{
  "schema_version": 2,
  "default_action": "reject",            // "allow" | "reject"；初始 reject
  "tags": {                                // 与授权同 revision、同 CAS
    "edge": { "members": ["<node-id>", "..."] }
  },
  "rules": [                               // 有序 first-match（若采用 allow-only，可改为并集）
    {
      "id": "edge-to-core",
      "name": "边缘访问核心",               // 可选，仅显示
      "action": "allow",                   // "allow" | "reject"
      "src": { "all": false, "nodes": [], "tags": ["edge"] },
      "dst": { "all": false, "nodes": ["<node-id>"], "tags": [] },
      "bidirectional": false               // 若采用：编译期展开为两个独立方向
    }
  ],
  "transit": [                             // 显式中转许可，独立于端点授权
    { "id": "lax-relays-edge", "via": { "nodes": ["<lax>"] },
      "src": { "tags": ["edge"] }, "dst": { "all": true } }
  ]
}
```

需要决定的点（推荐值仅供讨论）：

1. **`*` / `all` 的含义**：本网络中、在该 revision 快照里**未撤销且已发布连接公钥**的已注册节点。不含互联网、未注册节点、子网。安全影响：`default_action: allow` 或 `src: all` 会让**之后注册的节点自动获得访问**。需要用户确认是否接受，或要求新节点注册后进入隔离标签。
2. **评估模型**：(a) 有序 first-match，allow/reject 都可用（与路由一致，易解释"命中了哪条"）；(b) allow-only 并集 + 默认 reject（Tailscale 风格，顺序无关）。默认 allow 时只有 (a) 能表达例外。推荐 (a)，UI 已按有序卡片设计。
3. **标签归属**：策略内定义 `tags.{name}.members`（随授权同 revision 原子保存，最简单）vs 节点属性（需新的节点 mutation 和 revision 语义）。推荐策略内定义；节点页只读显示派生成员关系。
4. **方向**：当前 DataPlane IPv4 无状态、两个方向分别检查。`bidirectional` 只能是编译期展开的语法糖，不能暗示有状态回程。没有决定前 UI 只提供"同时添加反向规则"这一显式操作。
5. **中转许可**：当前路径授权隐含"路径中每个中间节点同意转发"。拆分后必须有 `transit`（或节点级 relay 能力 + 选择器），否则 route via 会被静默放行或静默拒绝。拒绝原因需在预览中可见。
6. **迁移**：v1 路径授权需保留行为。建议 controller 同时接受 v1；v2 新增 `legacy_paths`（原样精确路径匹配，和 v2 规则并存，按 "legacy 精确命中 ∨ v2 三段交集"）或一次性转换工具；Web 在迁移前保留 legacy 编辑器。
7. **域名不确定**：沿用现状，域名上下文不确定时失败关闭；access 规则不使用域名。

## 3. 需要 main 提供的契约 / 生成产物

按优先级：

1. **导出 schema（阻塞 JSONC 编辑器接入真实校验）**：`@meshway/control-client` 增加子路径，例如 `"./schemas": "./src/schemas.ts"`，导出生成的 `zMeshAccessPolicy`、`zRoutingPolicy`、`zFlowQuery`（以及未来 v2 文档 schema）。不改生成器语义，只加导出。
2. **生成 JSON Schema**：生成器从 OpenAPI `components.schemas` 为策略文档输出 `jsonschema.gen.ts`（`$ref` 转 `$defs`，保留 `description`）。用于补全、枚举、悬停帮助；Zod 仍是唯一校验来源。
3. **节点引用注解**：在 Rust 契约中给 node ID 字段加 OpenAPI 扩展，例如 `x-meshway-ref: "node"`（`path[]`、`via[]`、`exit_node_id`、`source.node_ids[]`、`destination_any[kind=node].value`、`members[]`），让编辑器按 schema 而不是硬编码字段名补全节点。
4. **授权预览 API（拓扑"证明阶段"需要）**：`POST /api/v1/access/preview { flow, routing_policy?, access_policy? }` → 分阶段结果：`classify`（路由命中规则/路径）、`access`（命中的访问规则或 legacy grant）、`transit[]`（每跳）、`effective`、`revision`。只做计划，不做 trace。
5. **可选**：路由规则 `name`；访问规则 `name`；节点标签的派生视图（`GET` 返回每节点所属标签）。
6. **网络设置**：当前没有 DNS / 网段写 API。Web 只读显示 `MeshNetwork { ipv4_prefix, ipv6_prefix }`，不提供编辑。

## 4. 依赖请求（Web 不改 package.json / lockfile，由 main 统一添加）

`apps/web` `dependencies`：

```
"@codemirror/autocomplete": "6.20.3",
"@codemirror/commands": "6.11.1",
"@codemirror/lang-json": "6.0.2",
"@codemirror/language": "6.12.4",
"@codemirror/lint": "6.9.7",
"@codemirror/state": "6.7.6",
"@codemirror/view": "6.43.13",
"@lezer/highlight": "1.2.5",
"jsonc-parser": "3.3.1",
"zod": "4.6.5"            // 仅当 control-client 不直接导出 JSON Schema 时，用于 z.toJSONSchema
```

选 CodeMirror 6 而不是 Monaco：

- 体积：按需模块，约 150–200 KB gzip；Monaco 2 MB 以上。
- SSR：CM6 纯 DOM，可在 `"use client"` 组件内 `import()` 延迟加载；Monaco 依赖 `window`，常见做法要从 CDN loader 加载，违背"本地加载、无 CDN"。
- Worker / CSP：CM6 无需 Web Worker；Monaco JSON 语言服务需要 worker（`worker-src blob:`）。
- shadcn 集成：CM6 主题即 CSS，直接使用 `--background`、`--border`、`--ring` 等变量，暗色模式跟随 `next-themes`。
- 补全 / lint：`@codemirror/autocomplete` 的 completion source 与 `@codemirror/lint` 的 diagnostics 由我们基于 `jsonc-parser.getLocation` + JSON Schema 自己实现，不引入 `codemirror-json-schema`（额外依赖链）。
- 注意：`@lezer/json` 语法不认注释，只影响高亮；解析、校验全部走 `jsonc-parser`。

`jsonc-parser` 是零依赖包。本分支先实现了与其 `parseTree` 节点形状一致的内置解析器（`lib/jsonc.ts`），依赖批准后可替换为一行导入，测试保持不变。

## 5. 编辑器状态模型（本分支已实现基础）

一个策略只有**一个**规范草稿，可视化编辑器和 JSONC 编辑器都写它：

```
PolicyDocument<T> = {
  base: { revision, value }       // CAS 基础；只有成功保存才移动
  text: string                     // JSONC 缓冲区，永远保留原文（含注释）
  parsed: T | null                 // 最近一次"语法 + Zod 都通过"的值
  syntax: JsoncError[]             // 行 / 列 / 偏移
  schema: SchemaIssue[]            // Zod 路径 → 文本范围
  seq: number                      // 每次编辑 +1，用于丢弃迟到的异步结果
}
```

规则：

- 文本编辑：重新解析；`parsed` 只在全部通过时更新。无效缓冲区永不自动保存、永不覆盖。
- 可视化编辑：只在缓冲区有效时允许；把旧值与新值逐路径比较，对**最深的差异节点**做文本替换 / 数组插入 / 删除，保留其他位置的注释和格式。
- 缓冲区无效时切到图形视图：图形视图只读并显示错误列表，提供"跳到错误"和"放弃文本修改（回到最后有效值）"两个显式操作，不静默丢弃。
- 保存冲突（409）：base 与草稿都不动；显示远端版本差异，用户选择"在新版本上重新应用"或"放弃草稿"，不覆盖注释。
- 草稿注册表（`OwnedDraftRegistry`）保存 `{ base, text, seq }`；`parsed` 可重新计算，避免两份事实来源。现有 access / routing registry 的唯一写者仍是各自的已挂载编辑器。
- JSONC 注释**不会**被 controller 保存（契约是 JSON）。保存后成功：若草稿等于发送值，文本保留注释（只在本地会话内）；UI 必须明确提示"注释只保存在浏览器草稿中"。若需要服务端保存注释，需要新契约字段（例如原文 `source`），由 main 决定。

## 6. UI 流程

### 访问（当前契约可做）

- "谁 → 谁" 矩阵：行 = 来源，列 = 目标，单元格显示该方向的授权路径（直连 / 经由 N 个节点）。双向是否齐全一目了然；点击单元格打开该方向的路径编辑。
- 列表仍可用，按 (来源, 目标) 分组，替代 6 行平铺。
- 节点永远显示名称 + 短 ID，悬停 / 展开显示完整 ID、Mesh IP、公钥状态。
- 说明区明确：方向独立、默认拒绝、`direct` 不授予权限、授权不绕过路由 reject 或必经节点。

### 访问（v2，待契约）

- 默认动作开关（allow / reject），旁边写清 `*` = 本网络活动且已发布公钥的节点。
- 规则卡片：`SRC 标签芯片 → DST 标签芯片`，芯片可展开成员；legacy 路径授权放在单独折叠区。
- 标签管理：标签名 + 成员选择，节点页只读显示所属标签。

### 路由（当前契约可做）

- 已有：有序 first-match 列表、上下移按钮、规则构建器。补：命中摘要句式（"来自 X 前往 Y → 经 A、B 出口 C"）、默认动作在列表末尾作为最后一张固定卡片、遮蔽检测（前面规则完全覆盖后面规则时提示）。

### 拓扑 / 流程（当前契约可做）

- 选择来源 + 目标 → 调用现有 `previewPolicy` 得到计划路径；在图上高亮计划路径（虚线），叠加已观测邻接（实线），访问授权精确匹配与否（图例），三层互不混淆。
- 证明阶段条：`分类（路由预览）→ 访问（本地精确匹配现有 grant）→ 每跳邻接观测（nonce，新鲜度 / TTL）→ TUN / 业务流量：未知`。没有 API 的阶段显示"未知"，不推断。
- 每一阶段有跳转链接：路由规则 → `/routing#rule-id`，授权 → `/access#grant-id`。
- 不动画、不画流量、不推断未观测的直连边（例如 NYC–SHA）。

## 7. 所有权

| 范围 | 所有者 |
| --- | --- |
| `apps/web/**`（本分支） | 前端 agent |
| Rust 契约、OpenAPI、生成器、`packages/**`、根配置、lockfile、依赖添加 | main |
| 访问 / 中转 / 标签语义决定 | main + 用户 |
| 正式 `docs/` | main（本文件是交接，不进文档站） |
