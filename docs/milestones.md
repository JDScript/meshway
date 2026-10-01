# 首版交付与后续里程碑

2026-10-01 文档基线。此页是交付依赖，不是完成声明或发布时间承诺；已确认要求以[产品对齐记录](/product-alignment)为准，验证入口见[开发与验收](/development)。

## 首版主线

**Mesh 私网互访与按域名 / IP 经指定路径出网同为首版核心。** 共同基础是多网络、用户身份、平台托管、四层策略及按需自动建连。互联网出口与域名分流不再推迟到旧 M4 之后。

| 阶段 | 待交付内容 | 依赖 |
| --- | --- | --- |
| P1a 多网络 | 成员、接入、IPAM、DNS、策略、连接与报告真实隔离，原网络无损迁移；设备一次只接入一网 | — |
| P1b 策略引擎 v3 | 纯 Rust 四层校验 / 编译：连接 allow/block、grants 并集、Mesh 具体度与同级冲突、出口方案有序 first-match；迁移报告 | — |
| P1c 出口数据面 | NAT、状态型回包、撤回终止存量流、DNS 泄漏边界设计与 Linux 隔离原型 | — |
| P2a 承载托管 | Web 可选 / 可修改 listener，平台生成下发参数与凭据，迁移本地 profiles | P1a |
| P2b 用户与登录 | 邮箱密码安全哈希、仅邀请、Owner、设备归属、用户 / 组授权、Agent 链接登录与 App SSO | P1a |
| P3a 策略上线 | controller / Web 接入四层模型，图形 / JSONC 共用草稿 / CAS，既有策略迁移 | P1a、P1b |
| P3b 自动建连 | 按授权流量及路径按需连接 / 自动中转，手工候选收敛为高级覆盖 | P2a、P3a |
| P3c 路由方案与分流 | 公共规则 + 方案规则 + 兜底；仅 Mesh / 全局经 X / 自动按 STUN 地区；规则集与可信域名上下文 | P1c、P3a |
| P3d 打洞 | 获准候选分发、认证探测与打洞，失败经允许中转并保持 E2E | P3b |

控制台整理、文档、性能定位并行推进，随真实契约接入。根 manifest、lockfile、生成产物与整体验证由协调者维护。

## 已有基础及验收入口

| 能力 | 已交付边界 |
| --- | --- |
| 控制面与配置 | 单网络稳定 ID、SQLite、注册 / 心跳 / 撤销、IPAM、公钥绑定、revision / digest / ACK、CAS 与双编辑器 |
| 托管 IPv4 | Linux TUN、路径绑定 Noise E2E、旧版端点授权与有序路由、Native UDP / TCP / TLS-VLESS、反向 dial |
| 节点 DNS | loopback UDP/TCP A/AAAA/PTR；隔离容器 resolver / search、真实 glibc / curl、退出恢复 |
| CLI | 保存状态与 live IPC 分开，whois、邻接 ping、DNS status/query |
| Endpoint 观测 | 独立 netcheck 的 kernel NAT 场景；真实 carrier socket STUN、身份 / 代际 / 期限上报 |
| App 接入 | controller PKCE 兑换与管理员批准、Apple Keychain / 库存显示；App 隧道未实现 |

这些能力不等于多网络 / 用户体系、grants 模型、全平台配置托管或公网出口已完成。行为证据集中见[开发验收](/development#数据面与实验)，具体边界见各功能页。

## 后续验证边界

- **打洞 / 主备**：验证 NAT、UDP blocked、反向 stream、endpoint 重绑定、失败 / 回切、防重放与 lease。承载变化不得跳过必经节点，不交付首版同流聚合。
- **DNS / 出口**：域名上下文来源仍待确认；解析路径与出口一致、回包 / NAT / 授权撤回及无泄漏均须真实隔离验收。设备不再新增手工 DNS / STUN 配置流程。
- **IPv6**：需真实双栈 TUN、ACL、MTU / ICMP 与 fail-closed，AAAA / 底层 IPv6 观测不是完成证据。
- **Trace**：需受认证的实际逐跳探测，规划路径和邻接 nonce 不冒充 trace；原生 traceroute 兼容另行评估。
- **身份 / 平台**：密钥轮换、通用 resolver 安装与恢复、Apple 真机 NE（待签名条件）、兼容 / HA 独立推进。OIDC、邮件发送不阻塞首版本地邀请账户。
