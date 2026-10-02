# 首版交付与后续里程碑

此页记录最新已确认的推进顺序及实际交付边界；产品语义见[产品对齐记录](/product-alignment)，验证入口见[开发与验收](/development)。

## 首版主线

**Mesh 私网互访与按域名 / IP 经指定路径出网同为首版核心。** 共同基础是多网络、用户身份、平台托管、四层策略及按需自动建连。互联网出口与域名分流不再推迟到旧 M4 之后。

用户已确认下一大交付点为四地 Linux 服务器的真实 Docker 组网。工程顺序：
**本轮收尾 / Demo 更新 → P3a 与系统保护通路 → P3b 自动组网及 P3d 打洞 → 跨地域 Mesh 验收 → P3c 完整出网验收**。
账户管理和 Web 产品重整同时推进；Apple 暂停新增开发，保留现有接入接口兼容。

| 阶段 | 内容与实现边界 | 依赖 |
| --- | --- | --- |
| P1a 多网络（已实现） | v8→v9 无损迁移；成员、接入、IPAM、DNS、策略、连接与报告隔离；管理 API 显式选网、Agent 由凭据推导；Web 选网 / 创建与逐网络草稿。Apple 已保存隔离多接入及活动网络切换，CLI 多接入仍待实现 | — |
| P1b 策略引擎 v3（已合入） | 独立 Rust 四层校验 / 编译：连接 allow/block、grants 并集、Mesh / 出口有序 first-match 与遮挡提示、路由方案、规则集展开及迁移报告；controller 接入在 P3a | — |
| P1c 出口数据面（原型已验收） | Linux NAT、状态型回包、撤回终止存量流与方案切换；12 个隔离 Docker 场景通过；控制面下发、出口 DNS 和正式产品接入仍待 P3c | — |
| P2a 承载托管（已合入） | 平台材料生成/下发、VLESS TCP/TLS 与 WebSocket/TLS、v10 迁移和私有 profiles 等价导入已完成隔离验收；WS 退休晚写、错误脱敏与能力投影问题已修复并独立复核。节点 listener 与添加节点的可选配置表单已部署，Demo Agent 已原位更新；旧 profiles 保留，托管导入另行推进 | P1a |
| P2b 用户与登录（Web/App 闭环已接入） | Controller API、事务内网络 RBAC、设备归属及 App PKCE 已合入；v11 正式迁移、Web 邮箱登录/首 Owner/单邀请及用户批准已接通。会话过期、撤成员 tag 清理、schema gate 和跨标签页会话问题已修复。CLI 链接登录、完整成员/账户管理 UI 仍待交付，用户 / 组策略随契约落地 | P1a；共享迁移 / 接口已统一 |
| P3a 策略上线 | controller / Web 接入四层模型，图形 / JSONC 共用草稿 / CAS，既有策略迁移 | P1a、P1b |
| P3b 自动建连 | 按授权流量及路径按需连接 / 自动中转，手工候选收敛为高级覆盖 | P2a、P3a |
| P3c 路由方案与分流 | 公共规则 + 方案规则 + 兜底；仅 Mesh / 全局经 X / 自动按 STUN 地区；规则集与可信域名上下文 | P1c、P3a |
| P3d 打洞 | 获准候选分发、认证探测与打洞，失败经允许中转并保持 E2E | P3b |
| Linux 跨地域联测（未开始） | 东京 / 纽约 / 首尔 / 洛杉矶独立 Docker 部署；先单台安装、两地互通，再四地多跳、重启、撤回与主备恢复。完成后在同一环境验收出口 / 分流 | P3a、系统保护通路、P3b、P3d |

### 账户与控制台并行主线

已部署地图概览、网络作用域、侧栏、托管监听、账户登录和邀请接入。用户认可地图方向，
其余页面继续重整产品逻辑与视觉。目标入口为概览 / 节点 / 成员 / 访问控制 / 路由 / 设置；
先交付成员管理、节点列表→详情→配置、登录→初始化→进入网络三组真实页面供用户手工验证。
成员 / 邀请 / 角色 / 设备归属、我的账户是第一批功能，策略界面随 P3a/P3c 扩展。

日常只有用户账户登录，所有者 / 管理员 / 成员是网络角色；实例凭据留在专门初始化 / 恢复入口，
不授予网络 Owner 额外跨网权限。账户后端已有基础接口，完整管理界面仍待本轮交付。
地图保留有来源的位置与真实连接证据，缺少 GeoIP 的节点仍明确待定位；不伪造健康与流量。
真实邻接延迟采样、幂等上报、滚动存储、查询与 Web 曲线已合入；正常 Agent 生命周期在
新鲜 Applied 报告获接受后启动唯一采样任务，撤回/lease/版本变化取消旧任务。
容量停写和 schema gate 问题已独立复核关闭，查询显示保留范围与容量裁剪。
三个本地 Demo 已运行新版 Agent，四个有方向的直接邻接开始产生真实历史；端节点之间没有直邻接，因此不生成它们的邻接样本。不补旧数据、不当作端到端路径 RTT，也没有主动检测命令。
性能复测与 Rust 代码审查独立并行，输出证据与可验证的改动；共享 Docker 实验使用同一性能锁。
系统保护通路的[独立 Linux socket/策略路由实验](/underlay-routing)已完成双栈 148 项
检查与 10 项 host-only 测试。受保护 socket 工厂、HTTP/TLS connector、bootstrap DNS 与
代际取消的代码切片也已交付；实际 Linux namespace fixture 通过 137 次 mark/readback
审计与 4 个 listener 的 premark 检查。命名路由表漏检 block 外捕获表引用、DNS SERVFAIL
未尝试备用上游两项问题已修复并通过独立复核；基础模块及修复已合入并完成主线组合检查。
R1 心跳发现更高 revision 时立即撤旧执行的独立修复也已合入，原阻塞 HTTP 反例通过。
正式主循环、Native UDP/TCP、
捕获规则 ownership 与恢复仍待接通，现网尚未采用 socket 级保护。

### 已完成整合与验证

用户最新要求移除浏览器自动化检查：套件、依赖、运行脚本及 CI 入口已清理，Web 默认保留
类型、lint、模型单测和生产构建，UI 在开发页面手工验证。此前浏览器验收数字仅为历史证据，
不再作为日常检查要求；原生 Apple 联测的公共进程 helper 已解除浏览器框架依赖。

第一批整合已完成：R1、underlay 基础及 UR1/UR2 修复通过全工作区 **475 passed / 7 ignored**、
Clippy、生成 drift、SDK 12 / Web 150 tests、各 TypeScript 消费者与文档检查。
随后独立合入邻接探测原期限内的有界重试，增量 dataplane 74、Agent managed 28 / dataplane 16 /
packet 7 与 workspace Clippy 通过；它不延长超时或 lease，RTT 包含可能的重试等待。
已移除 underlay 实现/审查及邻接重试审查工作树，保留分支与证据；本地 Controller 已更新。
Docker demo 已从已审定主线 `bdbd13f` 构建并原位更新，三个容器 ID、可写层工具、身份、密钥和旧 profiles 均保留。
端节点双向 ICMP 各 3 包通过；部署回执与包验证位于
`.meshway/services/binary-update-b1188f16dea2/`。本次没有修改策略，也没有导入实验 offload。

最终整合已包含账户、托管承载、历史与 Apple 多接入改动：Rust **548 passed / 7 ignored**、
SDK 14 tests、Web 176 tests、Clippy、drift、各消费者类型检查与生产构建通过。
最后一次浏览器回归 19 项通过后，按用户要求删除整个浏览器套件和相关依赖/CI。
现网已从 v9 向前迁到 v11，4 个旧节点、身份和配置内容保留，迁移前后网络 revision 为 **43**；
未自动创建用户或认领旧节点。升级前私有备份位于 `.meshway/services/db-backups/`。
此前独立真实副本演练还逐项验证了 15 张原表及重启不变，见[统一迁移](/schema-v11)。

性能本批最终报告已完成，见 `perf/tunnel-next` 分支的
`experiments/tunnel-throughput/HANDOFF.md`（`dba5f49`）。邻接丢包恢复修复已审合；
offload 有局部收益但仍有完整性 / 延迟问题，NEON 和 worker 调整无稳定收益，
没有无保留可上线的新吞吐候选。保留原始产物，不整分支部署、不扩展实验矩阵。
已核对性能工作树无未提交内容或进程引用并移除，分支与外部测量产物保留。

### 后续并行与发布边界

后端策略 / underlay 与前端账户 / 控制台按文件所有权并行；Rust 关键改动独立审查。
远端验收前补齐正式配置链路和 Docker 部署流程。容器只保留接入和私有持久化，
listener、承载与业务策略由 Web 管理。独立 namespace 内开放必要 TUN / 网络权限，
不修改宿主管理路由、DNS、防火墙或 Docker daemon，不使用 host network / privileged。
区分公网端口映射与真实打洞，受限 NAT 仍由本地隔离场景验证，远端不代替该覆盖。

用户最新要求的[承载扩展架构审查](/transport-registry)已完成源码核对：统一强类型目录、
控制面权威验证、Agent 编译/实际能力报告及混合版本协商是下一轮收敛方向。
先统一现有三协议及生成器，再用一个现有库级协议验证注册扩展流程；不继续复制 VLESS 特判。

各任务分别拥有领域模块、协议文件与测试；中心 Store / 路由注册、最终 schema 版本、
生成器与产物整合、根依赖锁和跨消费者验收由协调者统一处理。独立任务先交可运行的
隔离验证与小范围接线提交，合入依赖不代表开发必须串行；尚未合入或部署的能力不计为上线。

## 已有基础及验收入口

| 能力 | 已交付边界 |
| --- | --- |
| 控制面与配置 | 多网络稳定 ID、SQLite、注册 / 心跳 / 撤销、网络内 IPAM、公钥绑定、revision / digest / ACK、网络内 CAS 与逐网络双编辑器草稿 |
| 托管 IPv4 | Linux TUN、路径绑定 Noise E2E、旧版端点授权与有序路由、Native UDP / TCP / TLS-VLESS、反向 dial |
| 节点 DNS | loopback UDP/TCP A/AAAA/PTR；隔离容器 resolver / search、真实 glibc / curl、退出恢复 |
| CLI | 保存状态与 live IPC 分开，whois、邻接 ping、DNS status/query |
| Endpoint 观测 | 独立 netcheck 的 kernel NAT 场景；真实 carrier socket STUN、身份 / 代际 / 期限上报 |
| App 接入（暂停新增） | 用户浏览器授权 / PKCE、设备归属、Apple Keychain、多接入与库存显示；App 隧道未实现 |

用户体系基础已接通，完整用户管理界面、正式 grants 下发、全平台配置托管与公网出口仍待上述阶段交付。行为证据集中见[开发验收](/development#数据面与实验)，具体边界见各功能页。

## 后续验证边界

- **打洞 / 主备**：验证 NAT、UDP blocked、反向 stream、endpoint 重绑定、失败 / 回切、防重放与 lease。承载变化不得跳过必经节点，不交付首版同流聚合。
- **DNS / 出口**：域名上下文来源仍待确认；解析路径与出口一致、回包 / NAT / 授权撤回及无泄漏均须真实隔离验收。设备不再新增手工 DNS / STUN 配置流程。
- **IPv6**：需真实双栈 TUN、ACL、MTU / ICMP 与 fail-closed，AAAA / 底层 IPv6 观测不是完成证据。
- **Trace**：需受认证的实际逐跳探测，规划路径和邻接 nonce 不冒充 trace；原生 traceroute 兼容另行评估。
- **身份 / 平台**：密钥轮换、通用 resolver 安装与恢复、Apple 真机 NE（待签名条件）、兼容 / HA 独立推进。OIDC、邮件发送不阻塞首版本地邀请账户。
