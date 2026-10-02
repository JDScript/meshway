# Web 产品与交互

Web 控制台以 Tailscale 管理控制台为产品基线：在明确的网络（类似 tailnet）中管理节点、访问控制、DNS 与设置，并为 Meshway 的两项扩展（可插拔 transport 承载、按域名 / IP 经有序路径的路由）提供入口。页面只呈现 controller 的真实数据；尚无后端的功能不出现可操作入口。完整的信息架构、视觉语言与后续线框见 `apps/web/docs/console-design.md`。

## 网络作用域

每个管理页面都在 `/<network_id>/…` 之下，`network_id` 是 controller 的稳定 ID。页面读取、草稿与 Server Action 都绑定 URL 中的网络：多标签页里各自保存到自己渲染时的网络，选择 Cookie 只决定 `/` 落到哪张网络。不存在或格式不对的网络、他网节点均为 404，不回退到其他网络。侧边栏顶部的网络切换器列出全部网络、可创建网络；切换会导航到另一网络的同一分区，每张网络的未保存草稿各自保留。

## 导航

布局以 shadcn `sidebar-07` 为骨架（视觉可定制）：侧栏顶部是网络切换器，主入口为「概览 · 节点（全部节点、标签、手工连接）· 路由 · 设置（网络、DNS、访问授权）」，底部是账户菜单。诊断工作台与拓扑从节点、地图连线、路由预览与 ⌘K 进入。桌面可收起为图标栏（⌘B），状态记在布局 Cookie；手机上为抽屉。顶栏是 block 的触发器 + 面包屑，加 ⌘K 与主题切换。页面标题下最多一行说明，状态边界通过状态名称与字段旁的 ⓘ 说明表达，长说明折叠，错误原因直接可见。

## 页面与工作流

| 页面 | 路径 | 当前能力 |
| --- | --- | --- |
| 概览 | `/<n>` | 点状世界地图上的网络：登录后沉浸式全屏，「管理网络」缩入管理布局；节点在明确标注的「待定位」关系视图中（尚无位置来源），连线只来自配置通道的两端新鲜报告；角落状态（节点、连接确认 / 未验证、待处理、读取时间）；节点卡与连线卡（各端当前 RTT、历史空状态） |
| 节点 | `/<n>/machines` | 搜索（名称 / 地址 / 标签）、状态与配置筛选、排序、撤销 |
| 节点详情 | `/<n>/machines/<id>` | 概览：身份、地址、标签、控制面联系与回执、承载摘要、DNS 名称；配置：每个监听 / 拨号 socket 的绑定配置、实际绑定、对外候选（管理员发布 / STUN 未验证候选及过期）与对端验证，Agent 运行报告，手工草稿（高级） |
| 添加节点 | `/<n>/machines/add` | agent 一次性接入令牌与命令；Meshway App 浏览器授权说明 |
| 标签 / 手工连接 | `/<n>/tags`、`/connections` | 节点分组；跨节点监听与主动连接（高级） |
| 路由 | `/<n>/routing` | 图形 / JSONC 双编辑器、草稿预览 |
| 设置 | `/<n>/settings`、`/dns`、`/access` | 网络名称 / ID / 前缀；节点 DNS 意图与名称；访问授权（图形 / JSONC 双编辑器） |
| 诊断 | `/<n>/diagnostics`、`/topology` | 诊断工作台；拓扑、规划路径与流量检查（上下文入口） |
| 登录 | `/login` | 管理员令牌（用户账户上线前） |
| App 授权 | `/authorize-device` | 选择网络后批准 / 拒绝 Meshway App 接入 |

承载托管（VLESS / WebSocket、TLS、监听覆盖）、自动路由方案的地区依据、节点位置与延迟历史尚无已冻结的接口，控制台不提供入口或伪造数据；目标设计见 `apps/web/docs/console-design.md` §6.4、§6.6。日常「设置国家」入口已删除，旧 country 值只读保留在节点技术详情，路由中的国家条件不改动。

旧 URL（`/nodes`、`/enrollment`、`/access`、`/routing`、`/tags`、`/connections`、`/network`、`/monitoring`、`/topology`）保留为重定向，查询参数按新页面转发，`/network#dns-*` 转到 DNS 页。

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

管理员令牌只保存在 HttpOnly、SameSite=Strict 的服务端 Cookie 中；每个页面与 Server Action 在服务端检查登录，网络 action 显式携带渲染时的网络 ID。controller 响应由生成的运行时 schema 校验。App 依赖的同源接口 `/api/v1/devices/exchange`、`/api/v1/agents/heartbeat`、`/api/v1/agents/configuration`、`/api/v1/agent/dns` 只做透明转发，不读取控制台 Cookie、不注入管理员凭据。

## 验证入口

- `pnpm --filter @meshway/web check`、`pnpm test:web`：严格类型、lint，以及路由构造、旧 URL 映射、草稿 registry、JSONC 与模型单测。
- `pnpm build:web`：生产构建。浏览器自动化套件已移除；登录、网络切换、草稿保存、地图与响应式交互由用户在开发页面手工验证。
