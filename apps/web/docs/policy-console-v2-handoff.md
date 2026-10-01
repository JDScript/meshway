# Policy console v2：前端交接

分支 `feat/policy-console-v2`（基线 `a02a6f4`，已 cherry-pick 协调者提交 `18abb80`：CodeMirror 6 / `jsonc-parser` 依赖与 `@meshway/control-client/schemas`、`/json-schema` 子路径）。本文件只在 `apps/web/docs`，不进文档站。

## 1. 已确认的语义（用户 2026-10-01 确认，main 冻结）

- **Access 与 routing 分离**：routing 选路径（含 via / exit）；access 只回答端点 A 能否访问 B。controller 从 ACL + 路由 + tag 成员编译内部精确路径授权（`NodeDataplane.access_policy` 仍是 v1 `{grants}` 形状，只是编译产物）。没有单独的 transit 列表，也没有 legacy 路径授权编辑器。
- **Wire**：`MeshAccessPolicy { default_action: "allow"|"reject", rules: [{ id, source: string[], destination: string[], action: "allow"|"reject", bidirectional: boolean }] }`。
- **选择器**：`*`、`node:<64 位小写 hex>`、`tag:<name>`，name 为 `[a-z0-9][a-z0-9_-]{0,47}`；列表 1..32 项，`*` 必须单独出现，重复项在语义校验中拒绝。规则 ID 为 1..64 位 ASCII（`[A-Za-z0-9._-]`），可手写或自动生成。
- **`*`**：当前 network 中已注册、未撤销的节点，**包括之后注册的节点**；未发布公钥的节点也是 ACL 主体（公钥只是数据面前提）。显式引用已撤销 / 未知节点或未知 tag 使授权源无效并失败关闭。
- **有序 first-match**，allow / reject 都可用。`bidirectional: true` 的规则在**同一位置**同时匹配交换 source / destination 后的反向流量；UI 中是一张卡片，不拆两行。没有状态型回包跟踪。
- **Tags**：管理员维护的目录 + 节点绑定，独立快照 `{ revision, tags: [{ tag: "tag:prod", description }], assignments: [{ node_id, tags }] }`，`getTags` / `saveTags` / `setNodeTags`，全局 CAS。删除被引用或已分配的 tag 返回 422；空成员 tag 合法。`NodeSummary` 与 agent payload 形状不变，节点 tag 单独读取。
- **迁移**：现有 6 条路径授权迁移为 3 条双向 endpoint allow 规则 + 默认 reject。
- **组合预览**：`previewFlow(base, admin, { flow })` → `{ revision, valid, authorization: { action: allow|reject|invalid, rule_id, matched_direction: forward|reverse|null, reason }, routing: RouteDecision, diagnostics, execution_supported, execution_reason }`。只是计划，不是 trace。
- **JSONC 原文**：routing 与 access 的 Save 请求都可选 `source_jsonc`，Snapshot 返回 nullable `source_jsonc`；服务端解析后必须与 `policy` 完全相等，拒绝重复 key / 无效 JSONC；只改注释也提升全局 revision。

## 2. 已在本分支实现

| 范围 | 文件 |
| --- | --- |
| JSONC 解析 / 定位 / 注释保留的补丁（`jsonc-parser` + 重复 key 检查 + 不删注释的移除） | `lib/jsonc.ts` |
| 共享草稿：CAS 基准（含 `source`）、文本、最后有效值、问题（行 / 列 / 路径）、编辑序号、显式放弃 | `lib/policy-document.ts` |
| Schema 驱动补全：容错定位、`$ref` / `anyOf` / `oneOf` / `items` / `enum` / nullable，`x-meshway-ref` 引用（节点名称） | `lib/jsonc-complete.ts` |
| 路由工作区：同一草稿驱动图形与 JSONC；规则 key 跨重排 / 文本编辑稳定；编辑器检查定位到 `rules[i]` | `lib/routing-workspace.ts` |
| 导航保留：草稿注册表保存文本、注释、CAS 基准、规则 key、构建器、当前视图；孤儿保存只结算发出的那一版 | `lib/routing-draft-registry.ts` |
| CodeMirror 6 编辑器（客户端懒加载，无 worker / CDN，shadcn 变量主题，注释着色，lint 下划线，补全） | `components/policy/code-editor*.tsx` |
| 问题列表（可键盘访问，点击跳转）与 JSONC 面板 | `components/policy/problem-list.tsx`、`jsonc-pane.tsx` |
| 路由编辑器：图形 / JSONC 切换；文本无效时图形只读 + "回到 JSONC 修复 / 放弃无效的文本修改" | `components/routing/policy-editor.tsx` |
| 流量证据：路由规划 → 访问授权（未知）→ 逐跳邻接（agent nonce 探测）→ 数据面（未知）；规则深链 `/routing?rule=` | `lib/flow-stages.ts`、`components/topology/flow-stages.tsx` |
| 选择器解析与预检（模式按 §1 更新） | `lib/access-selector.ts` |

删除了中间版本的 path-grant 矩阵视图（`c0d1e14` 已 revert）。

## 3. 等待生成契约后接入（不手写 DTO）

1. **路由 `source_jsonc`**：`lib/routing-workspace.ts#routingBase` 是唯一读取点（当前返回 `source: null`）；`savePolicyAction` 增加可选 `source_jsonc: document.text`（仅当文本有效）。之后 SaveBar 中"controller 当前不保存 JSONC 原文"提示删除。
2. **Access v2**：`components/access/*` 重写为有序规则卡片（source → destination 选择器芯片、allow/reject、双向开关、上下移）+ 默认动作开关，JSONC 视图复用 `JsoncPane` 与 `PolicyDocument<MeshAccessPolicy>`；`lib/access-model.ts` 的路径授权模型与 `grant-builder` 删除。
3. **Tags**：节点页 tag 面板（`setNodeTags`）与 tag 目录页（`saveTags`），选择器芯片展开显示成员。
4. **组合预览**：`lib/flow-stages.ts` 的 access 阶段改为读取 `authorization`（规则深链 `/access?rule=`、`matched_direction`），并显示 `execution_supported / execution_reason`；拓扑与路由页的规划改用 `previewFlow`。
5. **`x-meshway-ref`**：生成的 JSON Schema 带上注解后，节点 / tag 补全自动生效（`references.node` 已传入，tag 需要 tags 快照）。

## 4. 需要 main 确认 / 提供

- 生成的 OpenAPI 中选择器 `pattern`、`minItems: 1`、`maxItems: 32`，以及 `x-meshway-ref`（`"node"`、`"tag"`、`"selector"`）。`*` 单独出现若无法用 schema 表达，请保证诊断 `field` 为 `rules[i].source[j]` 形式（编辑器已按此定位）。
- Zod 报错信息为英文；如需中文，可在 web 侧按 `code` 映射（不改生成物）。
- 浏览器验证：本分支未运行 `next build` / dev server / e2e。请在合并验证中跑 routing e2e，并手动检查 JSONC 视图（补全、错误跳转、切换视图、导航后恢复）。
- 网络设置：仍是单例 IPAM、无 DNS 管理，Web 不提供编辑表单。

## 5. 所有权

`apps/web/**` 由前端 agent 维护；Rust 契约、OpenAPI、生成器、`packages/**`、根配置、lockfile 与依赖由 main 维护。
