# 首版交付与后续里程碑

2026-10-01 文档基线。此页是交付依赖，不是完成声明或发布时间承诺；已确认要求以[产品对齐记录](/product-alignment)为准，验证入口见[开发与验收](/development)。

## 首版主线

**Mesh 私网互访与按域名 / IP 经指定路径出网同为首版核心。** 共同基础是多网络、用户身份、平台托管、四层策略及按需自动建连。互联网出口与域名分流不再推迟到旧 M4 之后。

| 阶段 | 内容与实现边界 | 依赖 |
| --- | --- | --- |
| P1a 多网络（已实现） | v8→v9 无损迁移；成员、接入、IPAM、DNS、策略、连接与报告隔离；管理 API 显式选网、Agent 由凭据推导；Web 最小选网 / 创建与批准选网，逐网络草稿。客户端多接入切换另行实现 | — |
| P1b 策略引擎 v3（已合入） | 独立 Rust 四层校验 / 编译：连接 allow/block、grants 并集、Mesh / 出口有序 first-match 与遮挡提示、路由方案、规则集展开及迁移报告；controller 接入在 P3a | — |
| P1c 出口数据面（原型已验收） | Linux NAT、状态型回包、撤回终止存量流与方案切换；12 个隔离 Docker 场景通过；控制面下发、出口 DNS 和正式产品接入仍待 P3c | — |
| P2a 承载托管（推进中） | Web 可选 / 可修改 listener，VLESS TCP/TLS 与 WebSocket 配置；平台生成下发参数与凭据，迁移本地 profiles | P1a |
| P2b 用户与登录（领域切片已合入） | 本地账户、邀请、会话、按网络权限与统一设备授权模块已实现；生产迁移、管理鉴权、Web 和 Agent/App 登录尚待接线，用户 / 组策略随契约落地 | P1a；共享迁移 / 接口由协调者整合 |
| P3a 策略上线 | controller / Web 接入四层模型，图形 / JSONC 共用草稿 / CAS，既有策略迁移 | P1a、P1b |
| P3b 自动建连 | 按授权流量及路径按需连接 / 自动中转，手工候选收敛为高级覆盖 | P2a、P3a |
| P3c 路由方案与分流 | 公共规则 + 方案规则 + 兜底；仅 Mesh / 全局经 X / 自动按 STUN 地区；规则集与可信域名上下文 | P1c、P3a |
| P3d 打洞 | 获准候选分发、认证探测与打洞，失败经允许中转并保持 E2E | P3b |

控制台整理、文档、性能定位并行推进，随真实契约接入。根 manifest、lockfile、生成产物与整体验证由协调者维护。

控制台重做及侧边栏调整已合入：网络作用域 URL、可折叠侧边栏、核心管理页面与逐网络草稿。
它接入当前真实多网络 API；用户账户、托管承载和 v3 编辑器随对应后端阶段交付。
重复说明已精简；监听与承载页区分配置、实际绑定、候选来源与逐对端验证，过期记录仅作历史。
shadcn `sidebar-07` 用作布局参考，按用户最新要求可进一步定制现代化视觉与交互；
侧栏/顶栏/内容区与移动端适配继续优化中。
概览进一步转向点状地图、真实节点/连接和精简总体状态，带进入 sidebar 工作区的过渡；
节点配置聚合与移除手工地区入口同步推进。实际 GeoIP/位置尚未上报，待定位状态须真实表达。
可观测性后续补齐节点间延迟检测与历史趋势的数据链路；邻接 RTT、端到端路径 RTT、
未测量与失败分别表达，历史接口未接通前不展示伪造曲线。
P2a 继续补齐真实承载配置入口，VLESS/WebSocket 表单尚待托管契约接通。
性能复测与 Rust 代码审查独立并行，输出证据与可验证的改动；共享 Docker 实验使用同一性能锁。
系统保护通路的[独立 Linux socket/策略路由实验](/underlay-routing)已完成双栈 148 项
检查与 10 项 host-only 测试；正式 Agent socket 工厂、HTTP connector、DNS 与生命周期
接入仍待交付，实验不等于现网已经采用 socket 级保护。

## 已有基础及验收入口

| 能力 | 已交付边界 |
| --- | --- |
| 控制面与配置 | 多网络稳定 ID、SQLite、注册 / 心跳 / 撤销、网络内 IPAM、公钥绑定、revision / digest / ACK、网络内 CAS 与逐网络双编辑器草稿 |
| 托管 IPv4 | Linux TUN、路径绑定 Noise E2E、旧版端点授权与有序路由、Native UDP / TCP / TLS-VLESS、反向 dial |
| 节点 DNS | loopback UDP/TCP A/AAAA/PTR；隔离容器 resolver / search、真实 glibc / curl、退出恢复 |
| CLI | 保存状态与 live IPC 分开，whois、邻接 ping、DNS status/query |
| Endpoint 观测 | 独立 netcheck 的 kernel NAT 场景；真实 carrier socket STUN、身份 / 代际 / 期限上报 |
| App 接入 | controller PKCE 兑换与管理员批准、Apple Keychain / 库存显示；App 隧道未实现 |

这些能力不等于用户体系、grants 模型、全平台配置托管或公网出口已完成。多网络控制面与快照隔离已实现；客户端多接入切换仍待交付。行为证据集中见[开发验收](/development#数据面与实验)，具体边界见各功能页。

## 后续验证边界

- **打洞 / 主备**：验证 NAT、UDP blocked、反向 stream、endpoint 重绑定、失败 / 回切、防重放与 lease。承载变化不得跳过必经节点，不交付首版同流聚合。
- **DNS / 出口**：域名上下文来源仍待确认；解析路径与出口一致、回包 / NAT / 授权撤回及无泄漏均须真实隔离验收。设备不再新增手工 DNS / STUN 配置流程。
- **IPv6**：需真实双栈 TUN、ACL、MTU / ICMP 与 fail-closed，AAAA / 底层 IPv6 观测不是完成证据。
- **Trace**：需受认证的实际逐跳探测，规划路径和邻接 nonce 不冒充 trace；原生 traceroute 兼容另行评估。
- **身份 / 平台**：密钥轮换、通用 resolver 安装与恢复、Apple 真机 NE（待签名条件）、兼容 / HA 独立推进。OIDC、邮件发送不阻塞首版本地邀请账户。
