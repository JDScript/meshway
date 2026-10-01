# Meshway 控制台设计（第一阶段）

> 状态：2026-10-01 第一阶段交付稿。已实现部分只接 controller 已有 API；标注「线框」的页面尚无后端，只作为后续阶段的设计依据，**不在控制台中出现可操作入口**。产品语义以 `docs/product-alignment.md` 为准，冲突时以该文件为准。

## 1. 设计原则

1. **以 Tailscale 管理控制台为基线。** 网络（tailnet）、节点（Machines）、访问控制（Access controls）、DNS、设置（Settings）的分区、命名和常规流程默认照搬；只有两项 Meshway 扩展需要时才偏离：可插拔 transport 承载、按域名 / IP 经有序指定路径的路由（含路由方案）。
2. **网络是一等公民。** 每个管理页面都在 `/<network_id>/…` 之下；页面、草稿、Server Action 都绑定 URL 中的网络，不依赖可变的选择 Cookie。
3. **只显示真实数据。** 没有上报的指标显示「未测量 / 未上报」；空状态说明真实原因；失败给出「哪里失败 / 下一步 / 技术详情（折叠）」。
4. **意图、回执与生效分开。** 「已保存」「节点已确认（ACK）」「数据面已生效」「实测」是四种不同状态，界面用不同措辞与颜色，绝不合并。
5. **默认简洁，细节可达。** 默认视图回答用户任务；ID、revision、digest、socket、generation 等放进「高级 / 技术详情」折叠区。折叠不卸载内容，草稿与计时不受影响。
6. **设备侧零配置。** 节点能力、listener、承载、出口、DNS 都在 Web 管理；页面不引导用户去设备上改配置（现有 `--dns-listen` 等参数只作为「当前实现」的帮助折叠出现）。

## 2. 信息架构

### 2.1 URL 结构

```
/                          → 记住的网络（切换器写入的 Cookie）或第一张网络
/login                     管理员令牌登录（用户账户上线前）
/authorize-device          Meshway App 浏览器授权（选择网络、批准 / 拒绝）

/<network_id>              概览
/<network_id>/machines     节点列表
/<network_id>/machines/add 添加节点（agent 接入令牌 / App 授权说明）
/<network_id>/machines/<node_id>[?tab=connections]  节点详情：概览 | 连接与承载
/<network_id>/access       访问控制 › 访问授权（v2：有序 allow/reject，默认动作）
/<network_id>/routing      访问控制 › 路由（v2：有序 first-match、经由与出口）
/<network_id>/tags         访问控制 › 标签
/<network_id>/connections  访问控制 › 手工连接（当前实现的高级入口）
/<network_id>/dns          DNS：节点 DNS 意图与节点名称
/<network_id>/diagnostics  诊断 › 诊断工作台
/<network_id>/topology     诊断 › 拓扑与流量检查
/<network_id>/settings     设置：网络名称、ID、地址前缀
```

- `network_id` 是 controller 的 32 位小写十六进制稳定 ID，不用名称（名称可改、可重名）。格式不对或不在 controller 网络列表中 → 404，**不回退到其他网络**。
- 节点详情只在本网络清单中查找节点；他网节点 ID → 404。
- 静态路由（`/login`、`/authorize-device`、`/api/v1/…`、旧 URL）优先于 `[network]` 动态段，App 依赖的路径不受影响。

### 2.2 导航

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ [M] Meshway  /  [N] 生产网络 ⇅                     [⌕ 搜索或跳转… ⌘K] [☾] (管) │
│ 概览   节点   访问控制•   DNS   诊断   设置                                     │
└──────────────────────────────────────────────────────────────────────────────┘
  访问控制： [访问授权] [路由 •未保存] [标签] [手工连接]
  诊断：     [诊断工作台] [拓扑与流量检查]
```

- **顶部一级 Tab**（对应 Tailscale 顶部导航）：概览 · 节点 · 访问控制 · DNS · 诊断 · 设置。窄屏横向滚动，不折叠成抽屉，保证任何宽度下一步可达。
- **二级 Tab**：一级 Tab 拥有多个页面时，在页面顶部用分段控件呈现。
- **网络切换器**（Tailscale tailnet 切换器）：列出全部网络（名称 + IPv4 前缀，未保存草稿的网络有琥珀色点）、创建网络、网络设置。切换 = 导航到另一网络的同一分区；详情页回到列表。
- **⌘K 命令面板**：跳转到任意页面、在节点中搜索、添加节点、切换主题；有其他模态窗口（如规则编辑 Sheet 有未应用修改）时不打开。
- **账户菜单**：当前为「管理员令牌会话」，外观、文档、退出登录（退出清空所有网络的草稿）。用户账户上线后显示邮箱、角色。
- **未保存提示**：一级 Tab 与二级 Tab 上的琥珀色点；整页刷新 / 关闭标签页触发浏览器离开确认。

### 2.3 页面清单

| 页面 | 状态 | 数据来源 | 用户任务 |
| --- | --- | --- | --- |
| 概览 | 已实现 | overview、nodes、access、policy | 看待处理事项，进入节点 / 策略 / 诊断 |
| 节点列表 | 已实现 | nodes、tags | 搜索（名称 / 地址 / 标签 / 国家）、筛选、进入详情、设置国家、撤销 |
| 节点详情 · 概览 | 已实现 | nodes、tags、connections、runtime、network | 地址复制、国家 / 标签、控制面联系与回执、连接摘要、DNS 名称 |
| 节点详情 · 连接与承载 | 已实现（手工） | connections、runtime、discovery | 运行报告、UDP 观测、手工监听 / 主动连接草稿 |
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

### 3.1 添加节点

1. 节点列表或概览右上「添加节点」。
2. 选择接入方式（单选卡片）：
   - **服务器 · meshway-agent**：选择有效期 → 生成一次性接入令牌（只显示一次）→ 填写节点名称 → 复制 enroll 命令 → 复制 run 命令。令牌只把节点加入当前网络。
   - **Meshway App**：说明 App → 系统浏览器 → `/authorize-device` 选网络批准 → App 兑换的流程；此页不伪造进度。
3. 底部说明「监听与承载」：当前在节点页手工配置；承载托管上线后这里出现可选 listener 配置（§6.4）。
4. 节点注册后出现在列表；「控制面联系」「配置确认」各自独立显示。

Tailscale 对照：Machines › Add device（Linux server 生成 auth key 命令；客户端登录）。差异：Meshway 不区分服务器 / 客户端节点类别，两种方式产出同一种节点。

### 3.2 配置 listener / 承载（当前：手工）

节点详情 › 连接与承载：运行报告（Agent 自报、带新鲜度）→ UDP 地址观测（折叠）→ 高级手工连接配置（草稿 + CAS，有草稿或无效配置时自动展开）。网络级跨节点视图在 访问控制 › 手工连接。目标形态见 §6.4。

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
| Machine › Edit route settings / exit node | 节点 › 连接与承载（手工）→ 未来「承载与出口」 | 出口由管理员在 Web 开启，设备不做通告（已确认问题 13） |
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

主操作用墨色而不是品牌色，品牌色只做指示，避免与状态色混淆。所有状态同时有文字，不靠颜色单独表达。

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

二级 Tab 变为：**连接 · 授权 · Mesh 路由 · 出口路由 · 标签**，每个 Tab 都有图形 / JSONC 双视图并共享一份 v3 草稿与 CAS。

```
授权（grants）                                     [图形] [JSONC]  [+ 授权]
  只写允许；未匹配即拒绝；规则取并集，顺序无关
  ┌ tag:edge  ⇄  tag:exit                                      ⋯ ┐
  ┌ tag:home  →  *                                             ⋯ ┐
  ┌ *  →  互联网                                               ⋯ ┐
```
- 连接：默认「可达即允许直连」模板 + node / tag 阻断规则（阻断优先）；显示按需建连说明，不要求逐对配置。
- Mesh 路由：必经节点规则按具体度分组（node > tag > *），同级冲突在保存前标红。
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

节点详情的「连接与承载」改为：

```
承载                                         平台默认：UDP › TCP › VLESS
  监听器   [+ 添加监听]   udp :41641 ✓ 已下发   tcp :443 ✓
  允许协议 ☑ UDP ☑ TCP ☑ VLESS   优先级 ⇅      逐段覆盖（高级）
可作为出口   ○ 关闭  ● 开启（转发 / NAT 由 Agent 按下发完成）
```
添加节点流程第 2 步增加「监听配置（可选）」，默认值由平台生成。

### 6.5 接入密钥（P2b）

设置 › 接入密钥：列表（ID、创建者、预分配 tag、一次性 / 可复用、过期、已用次数）与撤销；添加节点页的令牌生成复用同一表单。

## 7. 组件规范

| 组件 | 位置 | 规则 |
| --- | --- | --- |
| `ConsoleShell` | `components/shell/console-shell.tsx` | 顶栏 + 一级 Tab；包裹 `NetworkWorkspace` |
| `NetworkWorkspace` / `DraftVault` | `components/network/network-workspace.tsx` | 每网络一组草稿 registry；Vault 在网络段之上，切换网络不丢草稿 |
| `PageHeader` / `PageBody` / `SectionTitle` | `components/shell/page-header.tsx` | 返回链接、标题 + 状态、说明、meta（读取时间）、右侧操作 |
| `SectionTabs` | `components/shell/section-tabs.tsx` | 二级导航，显示未保存 |
| `Panel` / `Fields` / `Field` | `components/common/panel.tsx` | 唯一卡片面；标签 / 值行 |
| `StatusBadge` / `StatusDot` | `components/common/status-badge.tsx` | 五种语义色调，见 §5.1 |
| `StatStrip` | `components/dashboard/stat-strip.tsx` | 关键数字横条，可跳转 |
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
