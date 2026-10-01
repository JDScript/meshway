# Policy console v2：前端交接

分支 `feat/policy-console-v2`（基线 `a02a6f4`）。经协调者许可 cherry-pick 了 `18abb80`（CodeMirror 6 / `jsonc-parser` 依赖、`/schemas` 与 `/json-schema` 子路径）与 `7239e24`（access v2 后端与生成契约）。其余提交只改 `apps/web/**`。本文件不进文档站。

## 1. 已确认的语义（生成契约为准：`crates/protocol/src/access.rs`、`tags.rs`）

- Access 与 routing 分离：routing 选路径（via / exit），access 决定端点能否访问。controller 由 ACL + 路由 + tag 成员编译内部精确路径授权；Web 不再有任何路径授权编辑器。
- `MeshAccessPolicy { default_action: allow|reject, rules: [{ id, source[], destination[], action, bidirectional }] }`，有序 first-match。双向规则在同一位置匹配交换后的方向（`matched_direction`），无状态型回包跟踪。
- 选择器 `*`、`node:<64 hex>`、`tag:<[a-z0-9][a-z0-9_-]{0,47}>`，列表 1..32，`*` 单独出现。`*` = 当前 network 中已注册、未撤销的节点（含之后注册的节点、含未发布公钥的节点；公钥只是数据面执行前提）。引用未知 / 已撤销节点或目录中没有的标签 → 授权源无效，生效授权为空。
- Tags：管理员目录 + 节点绑定，全局 CAS；删除被分配 / 被引用的 tag 返回 422；空标签合法。
- `source_jsonc`：routing 与 access 的 Save 请求可选、Snapshot 返回；controller 验证其解析结果与 `policy` 完全相等后原样保存（含注释）。
- `previewFlow({ flow })`：按已保存的 access + routing 返回 `authorization`、`routing`、`execution_supported / execution_reason`。是计划，不是 trace。

## 2. 本分支实现

| 范围 | 文件 |
| --- | --- |
| Access v2 规则卡片：默认动作卡片（醒目）、有序 first-match 卡片（来源 → / ⇄ 目标、动作、标签成员、待公钥、变更 / 问题 / 被覆盖提示）、上移 / 下移 / 复制 / 删除（可撤销）、末尾"其余流量：默认 X" | `components/access/access-editor.tsx`、`rule-card.tsx`、`selector-chip.tsx` |
| 规则构建器：动作、来源 / 目标选择（`*` 或标签 + 节点，成员可见）、双向开关、可读的自动 ID（`allow-edge-to-core`），输入后不再自动更新 | `components/access/rule-builder.tsx`、`selector-picker.tsx`、`lib/access-rule-form.ts` |
| 校验（字段路径与 controller 诊断一致）、按 ID 判断变更、仅按选择器文本判断"被完全覆盖"（不展开成员、不评估流量） | `lib/access-model.ts`、`lib/access-selector.ts` |
| Access JSONC 视图：与卡片同一草稿；补全 `*` / `tag:` / `node:`（显示节点名）；保存时发送 `source_jsonc` | `components/access/access-editor.tsx`、`lib/access-draft-registry.ts` |
| 流量检查（access 页）：调用 `previewFlow`，显示访问决定、命中规则及方向、路由规则与路径、执行条件 | `components/access/flow-check.tsx` |
| 标签页：目录增删 / 说明、成员列表、删除前说明阻止原因；节点标签逐节点编辑（`setNodeTags`）；节点详情显示标签并跳转 | `app/(console)/tags/page.tsx`、`components/tags/tags-workspace.tsx`、`lib/tags-model.ts`、`lib/tags-actions.ts`、`components/nodes/node-detail.tsx` |
| Routing `source_jsonc`：读取快照原文为基准，保存时发送；只改注释也可保存 | `lib/routing-workspace.ts`、`lib/policy-actions.ts`、`components/routing/policy-editor.tsx` |
| 拓扑流量预览改用 `previewFlow`：访问 → 路由 → 逐跳邻接（agent nonce 探测、新鲜度）→ 执行条件；规则深链 `/access?rule=`、`/routing?rule=` | `lib/flow-stages.ts`、`components/topology/*` |
| Schema 报错的常见 code（type / enum / 长度 / 格式）显示中文；其他保留原文（纯文本） | `lib/policy-document.ts#schemaMessage` |

所有 server action 都先 `requireActionAdminToken()`，请求体由生成 SDK 的 Zod 校验。没有客户端规则评估器；`matched_direction` 用比较而不是类型断言收窄。

## 3. 给 main 的验证清单（本分支未运行 next build / dev / e2e）

- `e2e/access.spec.ts` 需按 v2 重写。可用 testid：`access-editor`、`access-default-action`、`access-add-rule`、`access-rule-list`、`access-rule-card`（`data-rule-id`）、`access-rule-action`、`access-rule-direction`、`access-rule-builder`、`access-rule-source`、`access-rule-destination`、`access-rule-bidirectional`、`access-rule-id`、`access-rule-apply`、`access-save`、`access-save-status`、`access-base-revision`、`access-base-invalid`、`access-behind`、`access-text-invalid`、`access-flow-check`、`flow-stages`、`flow-stage-<access|route|neighbors|execution>`（`data-status`）、`jsonc-pane`；标签页 `tags-workspace`、`tags-base-revision`、`tag-catalog`、`tag-row`、`tag-new-name`、`tag-add`、`tag-catalog-save`、`tag-assignments`、`node-tags-row`、`node-tags-save`、`tags-save-status`；节点详情 `node-tags`。
- 路由页拓扑相关 e2e 中"规划路径"按钮名保持不变；拓扑面板标题改为"流量预览"。
- 手动检查 CodeMirror（懒加载、补全、错误跳转、切换视图保留无效文本、导航后恢复）。
- 保存标签或节点标签会推进全局版本，access / routing 未保存草稿随后会提示"controller 已有更新版本"，这是预期的 CAS 行为。

## 4. 所有权

`apps/web/**` 由前端 agent 维护；Rust、OpenAPI、生成器、`packages/**`、根配置与 lockfile 由 main 维护（根 `Cargo.lock` 的 Rust jsonc 依赖由 main 处理）。
