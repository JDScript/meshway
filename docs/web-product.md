# Web 产品与交互

Web 控制台以 Tailscale 管理控制台为产品基线：在明确的网络（类似 tailnet）中管理节点、访问控制、DNS 与设置，并为 Meshway 的两项扩展（可插拔 transport 承载、按域名 / IP 经有序路径的路由）提供入口。页面只呈现 controller 的真实数据；尚无后端的功能不出现可操作入口。完整的信息架构、视觉语言与后续线框见 `apps/web/docs/console-design.md`。

## 网络作用域

网络管理页面都在 `/<network_id>/…` 之下，`network_id` 是 controller 的稳定 ID。页面读取、草稿与 Server Action 都绑定 URL 中的网络：多标签页里各自保存到自己渲染时的网络，选择 Cookie 只决定 `/` 落到哪张网络。不存在或格式不对的网络、他网节点均为 404，不回退到其他网络。侧栏切换器只列当前账户有权访问的网络；切换到另一网络的同一分区，各网络未保存草稿分别保留。创建网络使用实例初始化入口，不把网络 Owner 当作实例管理员。

## 导航

侧栏以 shadcn `sidebar-07` 为基础，主入口为「概览 · 节点 · 成员 · 访问控制 · 路由 · 设置」。
节点区内以 tabs 切换全部节点、标签、手工连接（高级）；设置区内切换网络与 DNS。
诊断工作台、拓扑和我的账户从上下文、账户菜单与 ⌘K 进入。桌面可收起为图标栏（⌘B），
状态记在布局 Cookie；手机使用抽屉。地图延续沉浸 / 管理过渡；管理页面强调表格和信息层级，
技术字段收进详情，错误与实际状态直接可查。

## 页面与工作流

| 页面 | 路径 | 当前能力 |
| --- | --- | --- |
| 概览 | `/<n>` | 沉浸式点状地图与管理布局过渡；缺定位节点明确待定位；真实连接与当前 RTT，连线内查询有方向的历史 |
| 节点 | `/<n>/machines` | 搜索、近期联系 / 待处理 / 未认领筛选；归属与已保存承载摘要；撤销节点折叠；读取失败明确显示未知 |
| 节点详情 | `/<n>/machines/<id>` | 归属、最近联系、配置确认、运行报告和承载分别显示；概览聚焦邻接与报告，配置页提供 listener / 承载编辑及实际绑定、候选来源、验证 / 过期和错误 |
| 添加节点 | `/<n>/machines/add` | Agent 接入令牌与可选初始 listener；App 浏览器授权说明 |
| 成员 | `/<n>/members` | 成员、网络角色、所属设备；邀请创建 / 一次复制 / 撤销；角色调整 / 移除、离开网络、Owner 认领旧节点 |
| 我的账户 | `/<n>/account` | 本人网络及角色、当前网络设备、改密码、退出当前或全部会话 |
| 标签 / 手工连接 | `/<n>/tags`、`/connections` | 节点分组；跨节点监听与主动连接（高级） |
| 路由 | `/<n>/routing` | 图形 / JSONC 双编辑器、草稿预览 |
| 访问控制 | `/<n>/access` | 当前 v2 授权的图形 / JSONC 双编辑器，v3 grants 随正式协议接线 |
| 设置 | `/<n>/settings`、`/dns` | 网络名称 / ID / 前缀；节点 DNS 意图与名称 |
| 诊断 | `/<n>/diagnostics`、`/topology` | 诊断工作台；拓扑、规划路径与流量检查（上下文入口） |
| 登录 | `/login` | 邮箱密码账户登录；实例凭据使用专门入口 |
| 初始化 / 恢复 | `/setup` | 验证实例凭据后创建网络、认领首个 Owner 并以该账户进入网络、创建 / 撤销重置链接与撤销用户会话 |
| 重置密码 | `/reset-password` | 消费一次性重置链接；秘密在 fragment 中，读入后清除地址栏 fragment，仅随 POST 提交 |
| App 授权 | `/authorize-device` | 用当前用户身份选择网络，批准 / 拒绝本人设备接入；App 只取得 PKCE 结果 |

VLESS TCP/TLS 与 WebSocket/TLS 托管监听、材料分发和邻接延迟历史已接入真实接口。
自动路由方案、实际 GeoIP 位置和 v3 策略尚待后续；不把当前页面或纯预览当作这些能力已生效。
日常「设置国家」入口已删除，旧 country 仅在技术详情保留，已有路由条件不随 UI 重整改写。

角色与移除的最终判断由 Controller 完成，包括最后 Owner 保护和成员移除时撤回设备。
目前没有逐会话清单 API，“退出所有会话”不显示虚构清单；实例恢复页从各网络成员汇总账户，
尚不能枚举没有任何网络成员关系的账户。

旧 URL（`/nodes`、`/enrollment`、`/access`、`/routing`、`/tags`、`/connections`、`/network`、`/monitoring`、`/topology`）保留为重定向，查询参数按新页面转发，`/network#dns-*` 转到 DNS 页。`/onboarding` 转到 `/setup`，`/invitations/new` 转到对应网络成员页的邀请入口。

## 状态与数据

| 信息 | 数据依据 | 呈现含义 |
| --- | --- | --- |
| 已注册 / 撤销 | `NodeSummary.revoked_at` | 控制面凭据状态 |
| 控制面联系 | `NodeSummary.last_seen` | 最近一次注册或心跳，不代表隧道在线 |
| 配置确认 | `configuration_revision` 与 `acknowledged_revision` | Agent 已校验并保存，不代表数据面已生效 |
| 授权 / 路由 | `MeshAccessSnapshot`、`PolicySnapshot` | 有效性、规则数、默认动作与版本 |
| 运行观测 | 连接运行报告、端点发现 | Agent 自报、带新鲜度与来源 |

节点间 RTT 之外的流量、吞吐、端到端延迟与真实 trace 尚无数据，界面写「未测量」。规划路径来自 controller 编译结果，是计划而不是 trace。

## 草稿、预览与保存

- Access、Routing、手工连接的修改先形成草稿；Access / Routing 的图形与 JSONC 编辑器共享同一份草稿与 CAS 基准，无效文本、注释、未应用的表单不会被切换视图或页面导航覆盖。
- 草稿按网络保存在当前登录界面的内存中，跨页面、⌘K、浏览器前后导航和网络切换保留；登出清空，整页刷新或关闭标签页前浏览器会提示。不写 localStorage / sessionStorage。
- 保存使用载入时的 revision；`409` 冲突与 `422` 校验失败保留草稿，只有成功保存或明确重新读取才更新基准。

## 安全边界

用户会话或实例凭据只保存在 HttpOnly、SameSite=Strict Cookie 中，每次登录有独立凭据 Cookie
和 active 指针，迟到清理只作用于原登录。每个页面和 Server Action 在服务端校验身份；
mutation 还核对渲染时的 sign-in marker，网络 action 显式携带渲染网络。403 保留会话，
401 进入登录复核；Controller 响应由生成 schema 校验。`/setup` 操作额外要求实例凭据，
网络 Owner 无法调用；旧实例凭据的管理深链保留兼容权限，不是普通账户的权限升级入口。
App 同源接口 `/api/v1/devices/exchange`、`/api/v1/agents/heartbeat`、`/api/v1/agents/configuration`、
`/api/v1/agent/dns` 仅透明转发，不读取 Cookie、不注入管理员凭据。

## 验证入口

- `pnpm --filter @meshway/web check`、`pnpm test:web`：严格类型、lint，以及路由构造、旧 URL 映射、草稿 registry、JSONC 与模型单测。
- `pnpm build:web`：生产构建。浏览器自动化套件已移除；登录、网络切换、草稿保存、地图与响应式交互由用户在开发页面手工验证。
