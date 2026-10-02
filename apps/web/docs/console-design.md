# Meshway 控制台设计

> 状态：2026-10-01。第三轮：shadcn `sidebar-07` 骨架（§2.2）、四个主入口「概览 / 节点 / 路由 / 设置」、沉浸式网络地图概览（§3.0）、节点「配置」聚合与删除日常国家入口；延迟检测 / 历史的目标流程与后端交接见 §6.6。已实现部分只接 controller 已有 API；标注「线框」的页面尚无后端，只作为后续阶段的设计依据，**不在控制台中出现可操作入口**。产品语义以 `docs/product-alignment.md` 为准，冲突时以该文件为准。

## 1. 设计原则

1. **以 Tailscale 管理控制台为基线。** 网络（tailnet）、节点（Machines）、访问控制（Access controls）、DNS、设置（Settings）的分区、命名和常规流程默认照搬；只有两项 Meshway 扩展需要时才偏离：可插拔 transport 承载、按域名 / IP 经有序指定路径的路由（含路由方案）。
2. **网络是一等公民。** 每个管理页面都在 `/<network_id>/…` 之下；页面、草稿、Server Action 都绑定 URL 中的网络，不依赖可变的选择 Cookie。
3. **只显示真实数据。** 没有上报的指标显示「未测量 / 未上报」；空状态说明真实原因；失败给出「哪里失败 / 下一步 / 技术详情（折叠）」。
4. **意图、回执与生效分开。** 「已保存」「节点已确认（ACK）」「数据面已生效」「实测」是四种不同状态，界面用不同措辞与颜色，绝不合并。
5. **默认简洁，细节可达。** 默认视图回答用户任务；ID、revision、digest、socket、generation 等放进「高级 / 技术详情」折叠区。折叠不卸载内容，草稿与计时不受影响。
6. **布局参考 sidebar-07，视觉不受模板限制。** Base UI / shadcn 负责菜单、表单、焦点与无障碍；网络画布、排版层级、悬浮控件和过渡按产品需要定制，不把每页都做成灰框卡片集合。
7. **设备侧零配置。** 节点能力、listener、承载、出口、DNS 都在 Web 管理；页面不引导用户去设备上改配置（现有 `--dns-listen` 等参数只作为「当前实现」的帮助折叠出现）。

## 2. 信息架构

### 2.1 URL 结构

```
/                          → 记住的网络（切换器写入的 Cookie）或第一张网络
/login                     管理员令牌登录（用户账户上线前）
/authorize-device          Meshway App 浏览器授权（选择网络、批准 / 拒绝）

/<network_id>              概览：网络地图（登录后首屏沉浸式）
/<network_id>/machines     节点 › 全部节点
/<network_id>/machines/add 添加节点（agent 接入令牌 / App 授权说明）
/<network_id>/machines/<node_id>[?tab=connections]  节点详情：概览 | 配置
/<network_id>/tags         节点 › 标签
/<network_id>/connections  节点 › 手工连接（当前实现的高级入口）
/<network_id>/routing      路由（v2：有序 first-match、经由与出口）
/<network_id>/settings     设置 › 网络：名称、ID、地址前缀
/<network_id>/dns          设置 › DNS：节点 DNS 意图与节点名称
/<network_id>/access       设置 › 访问授权（v2：有序 allow/reject，默认动作）
/<network_id>/diagnostics  诊断工作台（从节点 / 连线上下文与 ⌘K 进入，不在侧栏）
/<network_id>/topology     拓扑与流量检查（同上）
```

- `network_id` 是 controller 的 32 位小写十六进制稳定 ID，不用名称（名称可改、可重名）。格式不对或不在 controller 网络列表中 → 404，**不回退到其他网络**。
- 节点详情只在本网络清单中查找节点；他网节点 ID → 404。
- 静态路由（`/login`、`/authorize-device`、`/api/v1/…`、旧 URL）优先于 `[network]` 动态段，App 依赖的路径不受影响。

### 2.2 布局与导航（shadcn sidebar-07）

来源：`npx shadcn@latest add sidebar-07 --yes --overwrite`，shadcn CLI 4.21.1，`components.json` 为 `base-mira` / Base UI（registry 有该 block，未改用其他模板）。CLI 生成 `app/dashboard/page.tsx`、`components/{app-sidebar,nav-main,nav-projects,nav-user,team-switcher}.tsx` 并覆盖 `ui/{sidebar,button,input,dropdown-menu}.tsx` 与 `hooks/use-mobile.ts`；审查后保留 registry 版 `ui/sidebar.tsx`（16rem / 3rem，去掉被 lint 禁止的类型断言，中文化读屏文案），回退 button / input / dropdown-menu（保留控制台较大的控件与字号）与 use-mobile（保留 `useSyncExternalStore` 实现）。未新增依赖。示例数据、`/dashboard` 与 `nav-projects` 已删除，block 结构适配为：

```
SidebarProvider ─ AppSidebar (collapsible="icon"; 沉浸概览时 "offcanvas")
                │   SidebarHeader  NetworkSwitcher（team-switcher 位置，真实网络）
                │   SidebarContent NavMain「网络」：概览 · 节点▸ · 路由 · 设置▸
                │   SidebarFooter  NavUser（管理员令牌会话，无虚构用户 / 头像）
                │   SidebarRail
                └ SidebarInset (main)
                    header 4rem（图标栏时 3rem）：Trigger | Separator | Breadcrumb … ⌘K 主题
                    #main 页面内容
```

- **主入口**：概览、节点（全部节点 · 标签 · 手工连接〔高级〕）、路由、设置（网络 · DNS · 访问授权）。诊断工作台与拓扑不在侧栏：从节点详情、地图节点 / 连线、路由预览与 ⌘K 进入，面包屑归在「节点」下。分组仍在讨论中，`nav.ts` 是唯一数据源，调整不影响 URL。
- **可折叠分组**：与 block 相同，当前页所在分组默认展开，其他折叠；用户可手动开合，再次进入该分组时重新展开。折叠的分组若含未保存草稿，在分组名后显示琥珀点。图标栏中分组图标直接打开其第一页，tooltip 写出名称。
- **内容列**：顶栏内容与页面共用一列，`--console-column: 80rem` 居中，左右留白对称：手机 1rem，≥768px 1.5rem；侧栏展开、图标栏、抽屉三态都对齐。页面距顶栏 1.5rem。
- **收起**：⌘B / Ctrl+B、顶栏按钮或 rail；桌面状态写入 `sidebar_state` Cookie（布局偏好），服务端渲染读取。
- **移动端**（< 768px）：抽屉（网络切换器 + 导航 + 账户），任何导航后自动关闭；顶栏面包屑为「网络 › 页面」，分组名只在 ≥1024px 显示，⌘K 在 <1024px 收为图标。
- **网络切换器 / ⌘K / 账户菜单**：行为同前；账户菜单在侧栏底部，退出登录清空全部网络草稿。

### 2.3 页面清单

| 页面 | 状态 | 数据来源 | 用户任务 |
| --- | --- | --- | --- |
| 概览 | 已实现 | nodes、connections、runtime、policy、access | 网络地图：节点、真实连线、待处理；进入节点配置 / 诊断（§3.0） |
| 节点列表 | 已实现 | nodes、tags | 搜索（名称 / 地址 / 标签）、筛选、进入详情、撤销 |
| 节点详情 · 概览 | 已实现 | nodes、tags、connections、runtime、network | 地址复制、标签、控制面联系与回执、承载摘要、DNS 名称 |
| 节点详情 · 配置 | 已实现（承载托管待接） | connections、runtime、discovery | 每个监听 / 拨号 socket 的配置、实际绑定、对外候选与来源、对端验证；运行报告；手工草稿（高级） |
| 添加节点 | 已实现 | enrollment-tokens | 选择接入方式；生成一次性令牌与命令；App 授权说明 |
| 访问授权 | 已实现（v2） | access、connections、tags | 图形 / JSONC 双编辑器、默认动作、规则卡片 |
| 路由 | 已实现（v2） | policy、nodes | 图形 / JSONC 双编辑器、草稿预览 |
| 标签 | 已实现 | tags、connections、access | 标签目录、成员 |
| 手工连接 | 已实现（高级） | connections、runtime | 跨节点检查监听与主动连接 |
| DNS | 已实现 | network、nodes | 节点 DNS 意图、zone/TTL、节点完整名称 |
| 诊断工作台 | 已实现 | nodes、network、discovery | 按问题生成本地命令、查看承载观测 |
| 拓扑与流量检查 | 已实现 | policy、nodes、connections、runtime、flows/preview | 邻接图、规划路径、流量检查 |
| 设置 | 已实现 | network | 名称、ID、地址前缀 |
| 用户 / 邀请 | 线框 | P2b | 见 §6.1 |
| 访问控制 v3（连接 / 授权 / Mesh 路由） | 线框 | P3a | 见 §6.2 |
| 出口路由与路由方案、规则集 | 线框 | P3c | 见 §6.3 |
| 节点承载托管（listener、可作为出口） | 线框 | P2a / P3c | 见 §6.4 |
| 接入密钥列表（含 tag 预分配） | 线框 | P2b | 见 §6.5 |

### 2.4 旧 URL

旧 URL 全部保留为服务端重定向到「记住的网络」，查询参数按新页面语义转发：

| 旧 URL | 新位置 |
| --- | --- |
| `/` | `/<n>` |
| `/nodes?q=` | `/<n>/machines?q=` |
| `/nodes/<id>?tab=connections` | `/<n>/machines/<id>?tab=connections` |
| `/enrollment` | `/<n>/machines/add` |
| `/access?rule=`、`/routing?rule=`、`/tags?node=` | 同名页面，保留参数 |
| `/connections?node=<id>` | `/<n>/machines/<id>?tab=connections`；无参数 → `/<n>/connections` |
| `/network`（含 `#dns-*`） | `/<n>/settings`；`#dns-*` 片段在客户端转到 `/<n>/dns#…` |
| `/monitoring?source=` | `/<n>/diagnostics?source=` |
| `/topology?src=&dst=&…` | `/<n>/topology?…` |

## 3. 核心任务流程

### 3.0 概览：网络地图

```
┌───────────────────────────────────────────────────────────────────────┐
│ [M] 生产网络 100.96.0.0/16                               [管理网络]     │
│        · · · · ·  · ·  点状世界地图（装饰底图）  · · · ·               │
│              ╭──── 4 个节点待定位 · 按连接关系排布，不代表地理位置 ────╮  │
│              │        hk-relay ○                                  │  │
│              │       ┊                                            │  │
│              │  sg-edge ◉━━━━━━━━◉ laptop        ○ home-gw          │  │
│              ╰────────────────────────────────────────────────────╯  │
│ 4 节点 · 1/2 连接双向确认 · 1 未验证 · ⚠ 1 项待处理 · 更新于 16:40 ⟳   图例 │
└───────────────────────────────────────────────────────────────────────┘
```

- **沉浸式首屏**：会话中第一次落在概览（登录后）时，侧栏与顶栏隐藏，地图铺满视口。「管理网络」或 ⌘B 进入管理：侧栏滑入、顶栏展开，同一张地图缩进 `SidebarInset`（高度与侧栏间隙同 300ms 过渡），选中的节点 / 连线保持。离开概览即结束沉浸模式，之后回到概览不再重播；地图右上角「专注地图」可回到沉浸。手机上「管理网络」打开导航抽屉。`prefers-reduced-motion` 时无过渡。
- **底图**：`lib/world-dots.gen.ts`，由 `scripts/generate-world-dots.ts` 从 Natural Earth 1:110m Land（公有领域，v5.1.2，sha256 `9e0729ee…`）生成 1.5° 等距圆柱格点，仅作装饰，不参与任何节点定位。
- **位置**：节点只有带来源的 `GeoPlacement`（地区级经纬度、来源、观测时间）才画在地图上；控制面目前没有 GeoIP / 位置接口，所以全部节点进入「待定位」关系视图。节点名、Mesh IP 和旧管理员 country 永远不当位置。关系视图是名称排序的环，独立的虚线圆框并写明「不代表地理位置」。未来接入时地区级聚合显示，不宣称精确坐标。
- **连线**：只画已保存连接配置中的通道，按两端 agent 自己的新鲜报告分为：双向确认（实线，success）、单侧报告（warning）、未认证（红色虚线）、仅配置无报告（灰色点线）。授权允许、规划路径都不是连线，不画全连接图。
- **角落状态**：活动节点数；连接「x / y 双向确认 · 单侧 · 未验证」；连接配置或运行报告从未读到时写「连接未知」；之后的刷新失败时保留上次报告但标为「缓存 HH:MM」与「上次成功 HH:MM」（报告仍按原有效期过期，不续期），并直接显示错误与「重试」，连线卡同样标注；待处理项（访问授权 / 路由无效或读取失败、未确认配置、超过 1 小时无控制面联系）展开为列表；运行报告读取时间与刷新。没有任何「健康度」分数。
- **节点卡**：控制面联系、配置确认、运行报告新鲜度、连接计数、位置（待定位）；进入「节点配置」、详情、诊断。
- **连线卡**：每个通道的承载、发起方、两端各自的认证结果与 RTT（只取 `probe_fresh` 的认证探测，分别显示，不平均、不相加）；下方是有方向的延迟历史（§6.6）；进入访问与路径检查、拓扑；候选 / listener ID 折叠在技术详情。
- **键盘**：节点、连线是可聚焦按钮（Enter / Space 选中，Esc 关闭卡片）；Tab 顺序先地图后状态条。
- **沉浸时的隐藏界面**：桌面上移出屏幕的侧栏与收起的顶栏设为 `inert`，不在 Tab 顺序与辅助技术中；进入管理立即恢复，焦点落到替换「管理网络」的「专注地图」按钮（⌘B 进入时同理）。手机抽屉只在打开时挂载，不受影响。
- **概览读取失败**：节点清单读不到时显示真实错误与「重新读取」，并结束沉浸模式，侧栏、顶栏与 ⌘K 可用。


### 3.1 添加节点

1. 节点列表或概览右上「添加节点」。
2. 选择接入方式（单选卡片）：
   - **服务器 · meshway-agent**：选择有效期 → 可选「同时配置监听」→ 生成一次性接入令牌（只显示一次）→ 填写节点名称 → 复制 enroll 命令 → 复制 run 命令。令牌只把节点加入当前网络。不配置监听时用普通接入令牌；配置时用托管接入令牌与 `initial_listener`（节点 ID 为空，由注册事务绑定新节点），结果中显示绑定的监听摘要（`feat/managed-listeners-web` 已实现，见 §6.4）。
   - **Meshway App**：说明 App → 系统浏览器 → `/authorize-device` 选网络批准 → App 兑换的流程；此页不伪造进度。
3. 注册后在节点「配置」中查看与调整监听（§3.2、§6.4）。
4. 节点注册后出现在列表；「控制面联系」「配置确认」各自独立显示。

Tailscale 对照：Machines › Add device（Linux server 生成 auth key 命令；客户端登录）。差异：Meshway 不区分服务器 / 客户端节点类别，两种方式产出同一种节点。

### 3.2 节点配置（节点详情「配置」Tab，`?tab=connections`）

已实现（只用已有生成 API：connections、connection runtimes、endpoint discovery）：

```
已保存 2 个监听 · 0 个主动连接   节点回执 [已保存当前版本]              诊断此节点
监听                                        UDP 地址观测：未验证候选观测 ⓘ ⟳
 [Native UDP] sg-udp                                           ● 已监听
   绑定配置  0.0.0.0:41641          实际绑定  0.0.0.0:41641
   对外候选  203.0.113.10:41641  管理员发布 [已保存]
             198.51.100.20:41641 STUN 观测 [未验证候选] 18 秒后过期
   对端验证  laptop 经 203.0.113.10:41641 拨入 [● 已认证] 38 ms
   ▸ socket 技术详情（generation / socket 序号 / observer / STUN RTT）
 [Native TCP] sg-tcp                              ● 监听失败：本机监听失败
   检查监听地址属于本节点、端口未被占用……（失败原因直接可见）
   对外候选  sg.example.net:8443 管理员发布；Native TCP 不做 UDP 地址观测。
主动连接
 [Native UDP] laptop-to-sg → sg-edge                            优先级 1
   拨号目标 203.0.113.10:41641     本地 socket 0.0.0.0:53122
   出站映射 198.51.100.77:61001 STUN 观测 [未验证候选]（只属于该拨号 socket，不是监听）
   对端验证 sg-edge 拨向 … [● 已认证]
▸ 地址观测报告与启用帮助（原始报告：state / epoch / revision / digest、当前 STUN 启用参数）
Agent 运行报告（运行阶段、全部邻居认证探测）
▸ 连接身份与当前启用帮助    ▸ 高级手工连接配置（草稿 + CAS）
```

规则（`lib/carrier-view-model.ts`，有单测）：

- 「绑定配置」（已保存）、「实际绑定」（Agent 报告；只有 UDP socket 报告具体地址）、「对外候选」（按来源：管理员发布 = 已保存意图；STUN 观测 = 未验证候选，带过期倒计时）、「对端验证」（按已保存拨入 / 拨出关系逐个对端列出邻居认证探测）四类事实分开显示，不合并成一个「可达」。
- STUN 映射只标「未验证候选 / 已过期 / 历史观测 / 无映射」，从不标可达；过期或非当前代际的地址不可复制。UDP 观测从不出现在 TCP / VLESS 监听上（显示「不做 UDP 地址观测」）。
- 拨号 socket 的观测归属其 outgoing candidate，显示为「出站映射」，不当作监听；没有监听的节点仍显示拨号 socket 的地址。报告里有但已不在配置中的 socket 进入「其他已上报 socket（历史）」。
- 运行报告只有在「最近报告且对应当前已保存 revision」时才标注到行上；否则行上显示「未报告」，完整历史在下方运行报告里看。
- 地址观测与运行报告分别读取，本地再做一次栅栏：观测报告须属于本节点、对应当前已保存 revision、服务端标为当前，且与一份最近的同 revision 运行报告 epoch / digest 一致，才是当前证据；否则其中的实际绑定与映射全部显示为「历史报告」，不可复制。socket 的新鲜度与单次 STUN 结果分开：实际绑定只在该 socket 自己至少一条测量（任意结果，超时也算）的有效期覆盖当前时间时才是当前，新的运行报告不会延长旧 socket 的观测期限。不在当前配置中、或已改为 TCP / VLESS 的资源，其旧 UDP socket 一律是历史。所有有效期共用一个只前进的时钟。对端「已认证」但最近没有新鲜探测时不显示 RTT。
- epoch、digest、generation、socket 序号、observer、STUN RTT 只在折叠区。

托管监听的创建 / 编辑 / 停用表单在 `feat/managed-listeners-web` 实现（§6.4），每个监听标明「平台托管」或「手工配置（旧）」来源。仍不出现入口：NAT 打洞、UPnP / NAT-PMP / PCP 映射开关、承载优先级与「可作为出口」。

### 3.3 授权

访问控制 › 访问授权。当前 v2：默认动作卡片 + 有序规则卡片（来源 → / ⇄ 目标、动作、标签成员），JSONC 视图共用同一草稿与 CAS 基准；右侧「检查已保存的授权」跳到拓扑流量检查。目标 v3 grants 见 §6.2。

### 3.4 路由与路由方案

访问控制 › 路由（v2 有序规则）。v3 后拆为「Mesh 内路由」（具体度匹配）与「出口路由 / 路由方案」（有序 first-match），见 §6.2、§6.3。

### 3.5 DNS

DNS 页：开启节点 DNS 意图、zone、TTL（与名称共享网络 CAS）；下方列出每个活动节点的完整名称（由节点 ID 推导，可复制）；「DNS 诊断与当前启用帮助」折叠区给出本地命令。保存只表示控制意图。

### 3.6 诊断

- 诊断工作台：选源 / 目标节点，四个检查分区（已保存 flow、邻接与身份、DNS、底层网络 netcheck），每个分区标注数据来源（配置预览 / Agent 运行观测 / 本地执行未上报）。
- 拓扑与流量检查：已认证邻接、配置意图、规划路径三层；流量检查调用 `flows/preview`，结果是计划不是 trace。
- 未测量的项目（流量、吞吐、端到端延迟、真实 trace）明确写「未测量」。

## 4. 与 Tailscale 控制台对照

| Tailscale | Meshway 现在 | 差异与理由 |
| --- | --- | --- |
| Tailnet 切换器 | 网络切换器（`/<network_id>` URL） | URL 绑定网络，避免多标签页写错网络；Tailscale 用会话选择 |
| Machines | 节点 | 同一种节点，无服务器 / 客户端类别；「控制面联系」不叫 online |
| Machine › Edit route settings / exit node | 节点 › 配置（观测事实、托管监听表单〔分支〕、手工草稿）→「可作为出口」 | 出口由管理员在 Web 开启，设备不做通告（已确认问题 13） |
| Add device | 添加节点 | 同 |
| Users | （线框） | 本地邮箱 + 密码、仅邀请（P2b） |
| Access controls（Visual / JSON editor） | 访问控制 › 访问授权 / 路由 / 标签（图形 + JSONC 共享草稿） | Meshway 有四层策略；路由独立于授权，不合并为 `via` |
| — | 访问控制 › 手工连接 | Meshway 扩展（承载）；自动建连上线后收敛为连接约束 |
| DNS（MagicDNS、nameservers） | DNS | 首版只做节点 DNS 意图与名称；全局 override / split DNS 未确认 |
| Logs | 诊断 | 还没有审计 / 流量日志；诊断只呈现已有观测与本地命令 |
| Settings › Keys | 添加节点（一次性令牌） | 令牌列表 / 撤销需后端（§6.5） |
| Settings › General | 设置 | 名称、ID、地址前缀（只读） |
| — | 概览 | Tailscale 无仪表盘；Meshway 用它汇总待处理事项（只基于控制面数据） |

## 5. 视觉语言

### 5.1 色彩（`app/globals.css`）

| Token | 浅色 | 深色 | 用途 |
| --- | --- | --- | --- |
| `--background` / canvas | oklch(0.982 0.003 247) | oklch(0.158 0.009 258) | 页面底色（冷灰） |
| `--card` | 白 | oklch(0.198 0.011 258) | 工作面板 |
| `--foreground` | oklch(0.215 0.018 258) 墨色 | oklch(0.955 0.005 250) | 正文 |
| `--primary` | 墨色 | 近白 | 主操作按钮（每屏最多一个） |
| `--brand` | oklch(0.56 0.105 189) 青绿 | oklch(0.79 0.115 185) | 品牌、当前 Tab 指示线、选中卡片、焦点 |
| `--success` | 绿 | 绿 | 已确认、有效、当前 |
| `--warning` | 琥珀 | 琥珀 | 待确认、未保存、需注意 |
| `--destructive` | 红 | 红 | 失败、无效、已撤销、危险操作 |
| `--info` | 蓝 | 蓝 | 信息说明、「意图已开启」 |
| `--neutral` | 灰 | 灰 | 未知、未测量、未上报、不适用 |

主操作用墨色而不是品牌色，品牌色只做指示，避免与状态色混淆。所有状态同时有文字，不靠颜色单独表达。侧边栏用 `--sidebar`（浅色为白、深色同卡片），与冷灰画布区分。

### 5.2 排版

- 字体：Inter（拉丁）+ 系统中文字体（PingFang SC / 微软雅黑）；等宽 Geist Mono 用于地址、ID、命令、revision。
- 字号：页面标题 22px/600；面板标题 14px/600；正文与控件 13px；辅助说明 12px；标签与徽章 11px。数字统一 `tabular-nums`。
- 行宽：说明文字最大 48rem，避免长行。

### 5.3 间距与密度

- 内容列最大 76rem，居中；桌面边距 32px，移动 16px。
- 页面块之间 24px；面板内边距 16–20px；表格行高约 52px（两行信息）。
- 控件：默认 32px 高，小号 28px，大号 36px；圆角 8px（面板）/ 6px（控件）/ 全圆（徽章）。
- 面板只有一级：不在卡片里再套卡片；分组用面板内的分隔线。

### 5.4 状态与措辞

| 场景 | 组件 | 措辞 |
| --- | --- | --- |
| 心跳 | 圆点 + 相对时间 | 「n 分钟前」，标题写「控制面联系」，不写在线 |
| 配置回执 | `StatusBadge` 成功 / 警告 | 已确认 / 待确认 / 从未确认（含 r 版本） |
| 策略 | 成功 / 危险 | 有效 / 无效（附诊断数） |
| 意图 | 信息 / 中性 | 「发布意图：开启」「未发布」「运行状态未上报」 |
| 观测 | 来源徽章 | 「Agent 运行观测」「配置预览」「本地执行 · 未上报」 |

### 5.5 图标

lucide，统一 16px（小控件 14px），线宽默认。一级导航每项一个图标；状态不只用图标表达。

### 5.6 暗色模式

跟随系统，可在账户菜单 / 顶部按钮 / 快捷键 D 切换。暗色下背景低饱和蓝灰、卡片略亮于背景，状态色提高亮度保持对比度；不使用纯黑。

### 5.7 说明文字

- 页面标题下最多一行任务说明，或直接省去；面板说明同样一行以内。
- 反复出现的边界（「控制面联系不代表在线」「配置确认不代表生效」「规划不是 trace」等）用清晰的状态名称表达，补充说明放在列名 / 字段名旁的 ⓘ（`components/common/hint.tsx`，悬停、聚焦或点按都能打开）。
- 规则释义、执行范围、当前启用参数等长说明收进折叠区（`DiagnosticDetails`）；错误与失败原因始终直接可见。
- 正文 13px、导航 14px，不为压缩空间缩小正文。

## 6. 尚无后端的页面（线框）

以下页面**不在当前控制台中提供入口**，后端契约落地后按此实现。

### 6.1 用户与邀请（P2b）

```
用户                                                      [邀请用户]
┌─────────────────────────────────────────────────────────────────────┐
│ 用户            角色     设备数   加入时间     最近登录      ⋯      │
│ alice@corp      Owner    3       2026-10-02   5 分钟前              │
│ bob@corp        成员     1       …                                 │
├─────────────────────────────────────────────────────────────────────┤
│ 待接受邀请  bob2@corp · 成员 · 24 小时后过期   [复制链接] [撤销]      │
└─────────────────────────────────────────────────────────────────────┘
```
- 邀请：输入邮箱 + 角色 → 生成链接（首版不发邮件，复制手动转交）。
- 用户详情：所属设备、角色、重置密码链接。
- 节点列表增加「所有者」列（用户或 tag）。登录页改为邮箱 + 密码，管理员令牌入口收进「其他登录方式」。

### 6.2 访问控制 v3（P3a）

侧边栏「访问控制」组变为：**连接 · 授权 · Mesh 路由 · 出口路由 · 标签**，每个 Tab 都有图形 / JSONC 双视图并共享一份 v3 草稿与 CAS。

```
授权（grants）                                     [图形] [JSONC]  [+ 授权]
  只写允许；未匹配即拒绝；规则取并集，顺序无关
  ┌ tag:edge  ⇄  tag:exit                                      ⋯ ┐
  ┌ tag:home  →  *                                             ⋯ ┐
  ┌ *  →  互联网                                               ⋯ ┐
```
- 「互联网」按 Tailscale 基线只含公网地址（排除 RFC1918、ULA、CGNAT、链路本地）；局域网须单独授权具体网段，选择器中分开列出并在规则卡片上注明。
- 连接：默认「可达即允许直连」模板 + node / tag 阻断规则（阻断优先）；显示按需建连说明，不要求逐对配置。
- Mesh 路由：只表达「必须经过某节点」，规则从上到下 first-match，可拖动排序；无规则时平台自动选路。编译结果标出被完全遮挡的规则，并给出被前序规则部分抢先的流量示例，提示调整顺序。权限类（连接、授权）与顺序无关，界面不提供排序。
- 迁移横幅：v2 → v3 无损迁移结果与无法等价转换的条目列表。

### 6.3 出口路由、路由方案与规则集（P3c）

```
出口路由
  公共规则（所有方案生效，排在最前）                         [拖动排序]
   1  192.168.1.0/24          → home-gw
  方案   [仅 Mesh] [全局经 sg-exit] [自动] [CN] [HK] [+ 方案]
   CN: 1 openai.com           → 经 hk-relay → 出口 sg-exit
       2 规则集 geoip-cn       → direct
       兜底                    → 出口 hk-relay
  [命中测试: 域名 / IP ____ ]  → 命中「CN #1」；被遮蔽规则提示
规则集         名称 · 来源 · 版本 · 更新时间 · 无法转换条目 n   [更新]
分配           用户 / 设备 → 允许的方案、是否锁定
```

### 6.4 节点承载与出口（P2a / P3c）

状态：托管监听表单已在分支 `feat/managed-listeners-web`（`5dc2f24`）实现，只消费冻结的 P2a 生成 SDK（`getManagedListeners` / `saveManagedListener` / `createManagedEnrollmentToken`，契约见 `docs/managed-transports-api.md`）；与 P2a WebSocket 后端修复一同组合合入后才部署，**尚未上线**。实现细节与验证见该分支的 `apps/web/docs/managed-listeners.md`。

已实现（分支）：

- **节点详情 ›「配置」› 监听**：每个监听标明来源。平台托管：协议 · SNI · WS 路径 / Host、材料版本、证书 SHA-256 前后缀、承载能力是否已报告（未报告显示「等待节点报告承载能力」）、编辑；手工配置（旧）：只读，旧 VLESS 显示本地 profile 引用，不提供编辑或私钥导入。已停用的托管监听单列，并列出仍引用它的拨号候选与「连接配置无效」影响。底部「添加监听」。
- **监听表单（Sheet）**：协议 Native UDP / Native TCP / VLESS · TLS / VLESS · WebSocket；启用；本机监听地址与端口；对外地址列表（可空，最多 16）；WS 路径；高级：TLS 名称（SNI）、WS Host、ID。创建后 ID、节点与协议固定（VLESS 可在 TLS / WS 间切换）。保存前列出影响（引用的拨号候选、停用后计划失效、删除对外地址使候选失效、SNI 变化重新签发证书与 UUID）。
- **添加节点 › 服务器**：可选「同时配置监听」，见 §3.1。
- **高级手工连接**：托管监听行只读并链接到节点配置；拨号候选不被静默删除。TCP / VLESS 拨号 socket 的本地地址显示「未知 · 暂不上报」。
- **状态来源不合并**：已保存意图来自 `getManagedListeners`；节点回执、实际绑定、对端验证沿用 §3.2；证书为平台自签、精确信任，控制台永不接收 UUID / PEM / 私钥。
- **草稿与 CAS**：每网络 `DraftVault.managedListeners`；表单打开时记录网络 revision 为 `expected_revision`，后台变新只提示；409 保留输入并提供显式「以最新版本为基础」；退出登录清空。

仍未实现、不出现入口：承载偏好（允许协议 / 优先级 / 逐段覆盖）、按需自动建连、「可作为出口」（P3c）、接入令牌列表 / 撤销、NAT 打洞与 UPnP / NAT-PMP / PCP（UPnP 范围未经用户确认）。422 的具体原因需 SDK 暴露错误 body 后再显示。WebSocket / TLS 是 VLESS 承载参数，不是新的节点类别。

#### 自动路由方案的地区依据（P3c，预留）

方案「自动」由控制面按设备底层公网 IP 观测推导地区。控制台在节点详情与方案分配处预留解释卡：

```
自动方案依据   观测 IP 198.51.100.20（STUN · 实际 UDP socket · 12 秒前）
               地区 SG（控制面 GeoIP，数据集版本 …）   选中方案「HK」
               缺少有效观测时：使用兜底方案「…」（并写明原因）
```

- 只用底层公网 IP 观测；不使用 Mesh IP、所选出口 IP、控制面连接来源 IP，也不把管理员「国家」标注当自动地区。当前没有 GeoIP 下发，界面不显示任何自动地区结果。日常「设置国家」入口已删除；旧 v2 country 值只在节点技术详情中只读显示（路由国家条件可能仍引用），路由规则与 JSONC 中的国家条件保持原样以便审阅。

### 6.5 接入密钥（P2b）

设置 › 接入密钥：列表（ID、创建者、预分配 tag、一次性 / 可复用、过期、已用次数）与撤销；添加节点页的令牌生成复用同一表单。

### 6.6 链路延迟历史（已接入生成接口 `queryLatencyHistory`；契约见 `docs/latency-history.md`）

**状态**：在分支 `feat/latency-web` 实现，基于历史后端 `13af9c5`；后端仍在评审，Agent 主循环的正式采样启动 hook 尚未启用，也**尚未部署**。未启用采样时连线卡如实显示「这段时间没有记录」。

**已实现（分支）**（地图连线卡 → 延迟历史；`components/latency/`，模型 `lib/latency-history-model.ts`，读取 `lib/latency-actions.ts`）：

```
连线卡
  当前   Native UDP  hk 发起   hk：已认证 2 ms · sg：已认证 2 ms        ← 运行报告，当前状态
  延迟历史 · 已认证邻接 RTT · 近 1 小时                       [⤢ 展开]
         [hk-relay → sg-edge | sg-edge → hk-relay]               ← 有方向，测量端 → 对端
         ▁▂▂▃  ▂▁ ╎ ▁▂   ▮                                       ← 分段折线、超时 / 失败刻度
         成功 3（中位 1.5 ms，0.42–1.6 ms）· 超时 1 · 失败 1 · 2 段
展开面板  方向 · 时间范围（15 分钟 / 1 小时 / 6 小时 / 24 小时）· 重新读取
         大图（←/→/Home/End 逐条读出，aria-live）+ 时间刻度
         分段图例（承载 · 监听 · 运行代际 · 配置版本 · 采样代际）
         事件（超时、失败原因、分段变化，均为真实记录）· 全部记录（按测量时间，最多 200 条）
```

- **读取**：Server Action 每次校验管理员会话与固定网络作用域，用生成的 operation 与 Zod 校验；查询虽是 POST 但只读。窗口为半开 `[from, until)`、整秒、≤ 24 小时（默认 1 小时）。按接收顺序 cursor 分页，每页 1,000 条，每次读取最多 6 页；仍有 `next` 时显示「只读取了前 N 条…」与「继续读取」，绝不把截断说成完整。
- **围栏**：网络、方向、节点对或时间窗改变即清空并以序号丢弃旧响应（Server Action 串行执行，旧响应仍会返回）；记录还按网络 / source / peer / 方向 / 窗口在本地过滤。刷新失败保留上次记录并显示读取时间、错误与「重试」。只在连线卡打开（组件挂载）且页面可见时每 60 秒有界刷新，不使整个概览重绘。
- **图表**：按 `sample.measured_at` 排序，按 `measurement_id` 去重；分段键 = 网络 · source · peer · 方向 · 运行代际 · revision · digest · 采样代际 · 承载 · 发起方 · listener · candidate，键变化断线并画淡色分隔；超过 `max(4 × 期望间隔, 120 s)` 无记录时断线（未采样 / 未上报，不叫离线）。timeout / failed 没有通道关联，画在节点对的事件带上，不归入任何承载折线。统计只取成功值的最小 / 中位 / 最大，空值不计 0；µs 转 ms 按量级保留精度，<0.01 ms 显示「<0.01 ms」。
- **状态**：读取中、无记录（「未采样或未上报，不代表断开」）、错误（可重试）、缓存（上次读取时间）、截断（继续读取）。历史与连线卡上方的当前 RTT 分开显示。
- **不做**：「检测一次」（无命令接口）、端到端路径 RTT、丢包率、断开 / 重连事件推断、「已覆盖 7 天」等覆盖率说法。后端正在为容量裁剪增加实际保留范围字段，生成后再显示，不以最早记录推断连续覆盖。

## 7. 组件规范

| 组件 | 位置 | 规则 |
| --- | --- | --- |
| `ConsoleShell` | `components/shell/console-shell.tsx` | `SidebarProvider`（受控 open，沉浸概览时 offcanvas）+ `AppSidebar` + `SidebarInset` 与 block 顶栏 |
| `ConsoleFocusProvider` | `components/shell/console-focus.tsx` | 沉浸概览状态；位于网络段之上 |
| `NetworkOverview` | `components/overview/network-overview.tsx` | 地图、关系视图、连线、状态条、选中卡；模型 `lib/overview-map-model.ts` |
| `LatencyHistory` / `LatencyChart` | `components/latency/` | 连线卡中的有方向延迟历史与展开面板；模型 `lib/latency-history-model.ts` |
| `AppSidebar` / `NavMain` / `NavUser` / `nav.ts` | `components/shell/` | sidebar-07 结构；`navMain` 是侧栏与 ⌘K 的唯一定义 |
| `NetworkWorkspace` / `DraftVault` | `components/network/network-workspace.tsx` | 每网络一组草稿 registry；Vault 在网络段之上，切换网络不丢草稿 |
| `PageHeader` / `PageBody` / `SectionTitle` | `components/shell/page-header.tsx` | 返回链接、标题 + 状态、说明、meta（读取时间）、右侧操作 |
| `Hint` / `HintLabel` | `components/common/hint.tsx` | 列名 / 字段名旁的说明弹层 |
| `NodeCarriers` | `components/nodes/node-carriers.tsx` | 节点监听与拨号 socket 的四类事实（模型 `lib/carrier-view-model.ts`） |
| `Panel` / `Fields` / `Field` | `components/common/panel.tsx` | 唯一卡片面；标签 / 值行 |
| `StatusBadge` / `StatusDot` | `components/common/status-badge.tsx` | 五种语义色调，见 §5.1 |
| `ControllerUnavailable` / `NoNodes` | `components/common/controller-states.tsx` | 读取失败 / 真实空网络 |
| `FailureNotice` | `components/common/failure-notice.tsx` | 失败原因：标题、下一步、折叠的原始错误 |
| `DiagnosticDetails` | `components/common/diagnostic-details.tsx` | 原生 `<details>`，内容常驻挂载 |
| `DraftActionBar` | `components/common/draft-action-bar.tsx` | 未保存草稿浮栏：吸底、琥珀描边、保留焦点与 live region |
| `StandaloneFrame` | `components/shell/standalone-frame.tsx` | 登录、授权、404、落地失败 |

## 8. 数据与安全边界

- 读取：页面用 `requireNetwork(params)`（`lib/network-scope.ts`）校验 URL 中的网络，再以显式 `networkId` 调用 `lib/controller.ts` 的 `fetch*`。
- 写入：Server Action 第一个参数是渲染时的网络 ID，内部 `requireNetworkActionContext` 先校验登录再校验 ID；controller 再检查网络存在性。
- 草稿：每网络独立的 registry；JSONC 原文（含注释、无效文本）、未应用表单与原始 CAS 基准一起保存；登出全部清空；不写 localStorage。
- 管理员令牌只在 HttpOnly / SameSite=Strict Cookie；App 同源代理（`/api/v1/agents/heartbeat`、`/devices/exchange`、`/agents/configuration`、`/agent/dns`）不读控制台 Cookie、不注入管理员凭据。

## 9. 第二阶段建议

1. 合入 P1a 后：去掉重复的 controller 访问层差异，按 main 的最终契约 rebase；补「网络删除 / 改名」若后端提供。
2. P2b：登录改邮箱 + 密码；用户页、邀请、接入密钥列表；节点所有者列；`/authorize-device` 由用户登录批准。
3. P3a：访问控制 v3 四个 Tab 与迁移报告；复用现有 `policy-document` / `jsonc` / draft registry。
4. P2a / P3c：节点「承载与出口」、添加节点的可选 listener；出口路由、路由方案、规则集与命中测试。
5. 运行数据：节点间 RTT / 流量 / 实际 transport 接入后，在节点详情与拓扑显示，并区分未测量、过期与零值。
