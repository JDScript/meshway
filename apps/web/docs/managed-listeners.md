# 平台托管 listener：控制台实现交接

状态：2026-10-01，分支 `feat/managed-listeners-web`。只消费已冻结的 P2a 生成 SDK
（`getManagedListeners` / `saveManagedListener` / `createManagedEnrollmentToken`），
契约见 [`docs/managed-transports-api.md`](../../../docs/managed-transports-api.md)。
本文件是交接说明，`console-design.md` §3.1 / §3.2 / §6.4 由设计文档作者按此更新。

## 已实现

| 位置 | 内容 |
| --- | --- |
| 节点详情 ›「配置」› 监听 | 每个监听标明来源：**平台托管**（协议 · SNI · WS 路径 / Host、材料版本、证书 SHA-256 前后缀、「承载能力可用 / 当前承载能力不可用」、编辑）或 **手工配置（旧）**（只读；旧 VLESS 显示其本地 profile 引用，不提供编辑或私钥导入）。已停用的托管监听单列，直接显示仍引用它的拨号候选与「连接配置无效」影响。面板底部「添加监听」。 |
| 监听表单（Sheet） | 协议：Native UDP / Native TCP / VLESS · TLS / VLESS · WebSocket；启用开关；本机监听地址 + 端口；对外地址列表（可空，最多 16）；WS 路径。高级：TLS 名称（SNI）、WS Host、ID。已创建的监听 ID、节点、协议固定（VLESS 可在 TLS / WS 间切换）。保存前列出影响：引用的拨号候选、停用后计划失效、对外地址删除导致候选失效、SNI 变化会重新签发证书与 UUID。 |
| 添加节点 › 服务器 | 「同时配置监听（可选）」：关闭时仍调用 `createEnrollmentToken`；开启时调用 `createManagedEnrollmentToken`，`initial_listener.node_id` 为空，由注册事务绑定新节点。结果中显示绑定的监听摘要，运行命令改为 `--features managed-connections … run --apply-connections`。 |
| 高级手工连接（全局与节点内） | 托管监听行显示「平台托管 · 在节点配置中编辑」，整行链接到节点配置，无删除按钮；仍可为它「允许连接」添加拨号候选。候选不会被静默删除。 |
| 拨号 socket | TCP / VLESS 拨号 socket 的本地地址显示「未知 · 暂不上报」，不从 UDP 观测或配置补造。 |

## 状态来源（不合并）

- 已保存意图：`getManagedListeners`（页面读取、每次保存 / 重新读取的响应、节点页轮询）。
- 承载能力：`compiled_capability_available`。false 可能是未报告，也可能是节点明确不支持该承载
  （如只支持 VLESS TCP），因此统一显示中性的「当前承载能力不可用」，不说成只是等待上报。
- 读取状态：托管监听的读取结果单独记录（`lib/managed-read-state.ts`）：显示的快照与其读取时间、最近一次读取
  结果分开，按读取时间排序，迟到的旧成功不能清除较新的失败，迟到的旧失败也不会标记已刷新的数据。
  任一读取失败而有旧快照时为「缓存」：面板显示失败原因、缓存时间与 revision 及重试；托管行带
  「缓存 · 托管设置 rN」。托管快照不是当前读取或与连接配置 revision 不同时，不在快照中的监听显示
  「未确定」（`managed.<id>` 引用的显示为平台托管），不会被标成手工配置。没有任何快照时禁止添加 / 编辑。
  基于缓存可以打开编辑和创建，表单提示缓存并以其 revision 做 CAS，版本变化只会冲突。
- 节点回执、实际绑定、对端验证：沿用 `getConnections` / `getConnectionRuntimes` /
  `getEndpointDiscoveries` 的既有显示；创建监听不显示为已组网。
- 证书：平台自签、精确信任；指纹只是公有保存状态。控制台永不接收 UUID / PEM / 私钥。

## 草稿、CAS 与鉴权

- 每个网络一个 `ManagedListenerDraftRegistry`（`DraftVault.managedListeners`），只保存有修改的表单；键为
  `node:<id>:create`、`node:<id>:edit:<listener>`、`enroll`。切网络、切 Tab、关闭表单时选「稍后继续」都保留；
  保存成功只清除与发送时完全相同的草稿对象；退出登录清空。离开页面前由 vault 的 beforeunload 提示。
- 表单打开时记录网络 revision 作为 `expected_revision`；后台读取变新时只提示，不覆盖表单。
  409 显示冲突并保留输入，「读取最新版本 / 以最新版本为基础」是显式操作，并列出已保存版本的变化字段。
- 是否「有修改」以当前明确采用的保存版本为准（编辑：`original`，rebase 后为最新保存值；创建：建议值），
  不是首次打开时的值：rebase 到 9443 后改回最初的 443 仍可保存，填 9443 才是没有修改。
- Server Actions（`lib/managed-listener-actions.ts`）每次 `requireNetworkActionContext(渲染网络)`，再用生成 Zod
  （`zSaveManagedListenerRequest` / `zManagedListenerSettings`）校验浏览器参数；401 走既有登录跳转。
- 字段级错误来自本地校验（`lib/managed-listener-model.ts`，对照 controller `validate`）。SDK 的
  `ControlApiError` 只保留状态码，422 时只能给出整体原因，输入保持。

## 验证

- `pnpm test`（含 `managed-listener-model.test.ts`、vault 测试）、`pnpm check`、`pnpm build:web`、
  `pnpm typecheck:e2e`。
- `e2e/managed-listeners.spec.ts`：独立临时 controller + 真实 `meshway-agent enroll`；带 WS 监听的令牌注册、
  能力由节点凭据报告前后两种状态、编辑 / 字段错误 / 稍后继续、CAS 冲突与重新基于、停用影响与恢复、
  rebase 后以新基准判定修改且只发送一次保存、只让托管读取失败（loopback 代理）时的缓存提示 / 重试恢复、
  手工编辑器只读、Native TCP 创建与 ID 冲突、跨网络 404 与隔离、匿名 Server Action 重放、HTML/Flight 中无令牌
  以外的秘密（含节点专属 UUID / 私钥 / 证书）、绕过 UI 的 422、退出清空草稿；截图设
  `MESHWAY_SHOTS_DIR` 时另存。

## 剩余

- 422 的具体原因需要 SDK 暴露错误 body（`ErrorResponse.message`）；当前不改生成 / 传输层。
- 拨号候选仍在高级手工连接中维护；按需自动建连、承载优先级与「可作为出口」不在本切片。
- 注册令牌列表 / 撤销、初始监听在令牌消费前的查看与修改没有接口。
- TCP / VLESS 拨号 socket 的 M2 明细、GeoIP 与 RTT 历史由后端独立推进。
