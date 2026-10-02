# 首版交付与后续里程碑

此页记录最新已确认的推进顺序及实际交付边界；产品语义见[产品对齐记录](/product-alignment)，验证入口见[开发与验收](/development)。

## 首版主线

**Mesh 私网互访与按域名 / IP 经指定路径出网同为首版核心。** 共同基础是多网络、用户身份、平台托管、四层策略及按需自动建连。互联网出口与域名分流不再推迟到旧 M4 之后。

用户已确认下一大交付点为四地 Linux 服务器的真实 Docker 组网。工程顺序：
**本轮收尾 / Demo 更新 → P3a 与系统保护通路 → P3b 自动组网及 P3d 打洞 → 跨地域 Mesh 验收 → P3c 完整出网验收**。
用户最新决定暂缓进一步前端重整，后续可自行另派 agent；Apple 同样暂停新增开发。
当前集中完成正常 Controller 下发 → Agent 生命周期 → 真实 TUN 包流的 Linux 组网闭环。
已部署 Web 用于联调，协议变化仍须维护生成客户端兼容并检查全部消费者。

| 阶段 | 内容与实现边界 | 依赖 |
| --- | --- | --- |
| P1a 多网络（已实现） | v8→v9 无损迁移；成员、接入、IPAM、DNS、策略、连接与报告隔离；管理 API 显式选网、Agent 由凭据推导；Web 选网 / 创建与逐网络草稿。Apple 已保存隔离多接入及活动网络切换，CLI 多接入仍待实现 | — |
| P1b 策略引擎 v3（已合入） | 独立 Rust 四层校验 / 编译：连接 allow/block、grants 并集、Mesh / 出口有序 first-match 与遮挡提示、路由方案、规则集展开及迁移报告；controller 接入在 P3a | — |
| P1c 出口数据面（原型已验收） | Linux NAT、状态型回包、撤回终止存量流与方案切换；12 个隔离 Docker 场景通过；控制面下发、出口 DNS 和正式产品接入仍待 P3c | — |
| P2a 承载托管（已合入） | 平台材料生成/下发、VLESS TCP/TLS 与 WebSocket/TLS、v10 迁移和私有 profiles 等价导入已完成隔离验收；WS 退休晚写、错误脱敏与能力投影问题已修复并独立复核。节点 listener 与添加节点的可选配置表单已部署，Demo Agent 已原位更新；旧 profiles 保留，托管导入另行推进 | P1a |
| P2b 用户与登录（首批管理界面已部署） | Controller API、事务内网络 RBAC、设备归属及 App PKCE 已合入；Web 邮箱登录、首 Owner、成员 / 邀请 / 角色 / 归属管理、我的账户、实例初始化 / 恢复已接通。CLI 链接登录尚待交付；当前没有全局无网络成员关系账户清单与逐会话清单，用户 / 组策略随契约落地 | P1a；共享迁移 / 接口已统一 |
| P3a 策略上线 | 优先 controller / Agent 接通四层模型、正常下发和既有策略迁移；生成管理接口供联调，Web v3 编辑体验另行推进 | P1a、P1b |
| P3b 自动建连 | 按授权流量及路径按需连接 / 自动中转，手工候选收敛为高级覆盖 | P2a、P3a |
| P3c 路由方案与分流 | 公共规则 + 方案规则 + 兜底；仅 Mesh / 全局经 X / 自动按 STUN 地区；规则集与可信域名上下文 | P1c、P3a |
| P3d 打洞 | 获准候选分发、认证探测与打洞，失败经允许中转并保持 E2E | P3b |
| Linux 跨地域联测（未开始） | 东京 / 纽约 / 首尔 / 洛杉矶独立 Docker 部署；先单台安装、两地互通，再四地多跳、重启、撤回与主备恢复。完成后在同一环境验收出口 / 分流 | P3a、系统保护通路、P3b、P3d |

### 已部署账户与控制台（后续重整暂缓）

首批管理页面已合入并更新本地 3000：主入口为概览 / 节点 / 成员 / 访问控制 / 路由 / 设置；
成员 / 邀请 / 角色 / 设备归属、我的账户、节点列表→详情→配置，以及登录→初始化→进入网络
流程已接真实 API。地图延续已有方向；用户仍不满意其余前端，决定先完成组网，后续独立安排 UI / 产品逻辑工作。

日常只有用户账户登录，所有者 / 管理员 / 成员是网络角色；实例凭据留在专门初始化 / 恢复入口，
不授予网络 Owner 额外跨网权限。`/login` 为日常邮箱登录，`/setup` 为实例初始化 / 恢复；
旧 bootstrap 深链保留既有兼容权限。没有逐会话清单时只提供真实的全部会话撤销；
实例恢复页目前按各网络 membership 汇总账户，不冒充完整全局账户清单。
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
Native UDP/TCP owning factory 与 Manager prepared owner 接点已合入 `46a7b42` / `94c65ea`。
原复核发现的 rebind 返回后旧 carrier 入站交付问题已修复并独立关闭；UDP/TCP 的 endpoint / relay
四种交错及新 carrier E2E 恢复通过。正式主循环调用、捕获规则 ownership / 恢复与系统 split-DNS
仍待接通；默认仍为 unmarked，现网尚未启用完整 socket 级系统保护。
该批主线组合检查完成：Rust **562 passed / 7 ignored**、fmt 与 workspace all-features / all-targets
Clippy 通过，日志在批准 TMP 的 `native-underlay-main-validation/`；作者及审查工作树已清理。

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
随后 `d9e8b41` 发布上下文与已审定 native 修复更新到本地 Controller / 三个 Demo Agent；
保留同一容器、身份和当前配置，现有配置下正常 Agent/TUN 的端节点双向 ICMP 各 3 包通过。
最新回执为 `.meshway/services/binary-update-4120ea711460/`；不将此项记为自动组网或打洞完成。

最终整合已包含账户、托管承载、历史与 Apple 多接入改动：Rust **548 passed / 7 ignored**、
SDK 14 tests、Web 176 tests、Clippy、drift、各消费者类型检查与生产构建通过。
最后一次浏览器回归 19 项通过后，按用户要求删除整个浏览器套件和相关依赖/CI。
现网已从 v9 向前迁到 v11，4 个旧节点、身份和配置内容保留，迁移前后网络 revision 为 **43**；
未自动创建用户或认领旧节点。升级前私有备份位于 `.meshway/services/db-backups/`。
此前独立真实副本演练还逐项验证了 15 张原表及重启不变，见[统一迁移](/schema-v11)。

账户 / 控制台首批 `4939fa2` 至 `5cc3ecf` 已完成聚焦权限复核，并修复邀请错误处理、
密码操作提示、缺少连接配置的状态文案和退出全部会话失败时的草稿保留。
合入后主线 Web typecheck / lint、**184 单测**与生产构建通过；开发服务 ready，
`/login`、`/setup`、`/reset-password` HTTP 读取和旧 onboarding 重定向通过。
该只读部署检查未创建账户、网络或修改配置，不代替用户手工 UI 验证。

性能本批最终报告已完成，见 `perf/tunnel-next` 分支的
`experiments/tunnel-throughput/HANDOFF.md`（`dba5f49`）。邻接丢包恢复修复已审合；
offload 有局部收益但仍有完整性 / 延迟问题，NEON 和 worker 调整无稳定收益，
没有无保留可上线的新吞吐候选。保留原始产物，不整分支部署、不扩展实验矩阵。
已核对性能工作树无未提交内容或进程引用并移除，分支与外部测量产物保留。

### 后续并行与发布边界

[代码架构审查](/architecture-review)已对冻结主线完成 Rust / Web / 工具链核对。
建议先收敛 Controller publication、policy 到执行合同、Agent control / session 和 Web 共享编辑会话等
module 边界，随 P3a / P3b 和控制台重整逐步落地；当前未实施大规模拆 crate 或目录搬迁。

后端策略与 underlay 按文件所有权并行，Rust 关键改动独立审查；不再主动开启前端体验或结构重构。
先完成当前 native 入站代际修复及复核，再接正式保护生命周期与 v3 下发；生成器只扩展真实契约所需形状。
P3a 前置发布边界修复已合入 `ddaee7c` / `a1b992f`：实际安装前缀预算、完整 global 方案 ID
与 Agent 消费者校验、移除编译对象直接 Deserialize。独立行为复核、fmt / Clippy 和主线
workspace 全 feature / target 编译通过；正式 Controller / Agent v3 合同和发布接线仍待完成。
Controller 正常发布链路已按独立复核合入 `d9e8b41`，见[发布上下文](/controller-publication)：
同次事务共享事实、编译与节点投影，保留现有 v2 / CAS / 鉴权和预算回滚语义；主线 101 单测、
6 codegen、39 HTTP、14 SDK 测试及 fmt / Clippy / drift 通过，日志在批准 TMP 的
`controller-publication-main-validation/`，本地 Controller 已更新。
v3 契约生成前置已合入 `889c752` / `b38b8ff` / `fa0f1d9`：五项 typed map / nullable 复核问题关闭，
完整真实源 schema 直接生成验证；`pnpm test:codegen` 已纳入 CI。尚未接入 v3 正式下发。
隔离固定依赖快照的组合验收通过：Rust **566 passed / 7 ignored**、生成器 **7**、SDK **14**、
既有 Web **184**，fmt / Clippy / drift、既有 TS 消费者与五个实验 tsconfig、Web / 文档构建均通过。
SDK fixture 的绝对 Cargo target 路径拼接问题已修为 `9160f26` 并重跑剩余检查；未重复无变化的 Rust 全套。
日志及初次失败保留在批准 TMP 的 `backend-integrated-validation/`。本次排除由用户独立维护的
`apps/console`，未替换该工作流的 workspace / lockfile 或源文件。
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

### 发现与地区信息的闭环核对

STUN 核心通过不表示默认发现、GeoIP 或自动路由方案已完成。当前明确逐项跟踪：

| 子项 | 当前状态 | 交付 / 验收边界 |
| --- | --- | --- |
| 真实 UDP socket 的 STUN 及有期限上报 | 已实现，区分 listener / outgoing、IPv4 / IPv6、节点 / epoch / revision / socket generation | 新采样才更新时间，退出 / 换代 / lease / 过期撤销资格；UDP 映射不代表 TCP/VLESS 可达 |
| 平台托管发现与默认观察点 | 未完成；当前仍需设备 `--stun-server`，无参数则 disabled | P3b/P3d 前置：正常接入后从控制面取得观察点与发现意图，设备不手工维护；所有探测走系统保护通路 |
| 公网 IP → 国家 / 地区与有来源的位置数据 | 未实现；`StunLocation` 仅是纯引擎输入 | 与发现数据链路独立跟踪，作为 P3c 自动方案前置；保留观测 IP / 来源 / 时间、GeoIP 数据源 / 版本，位置是近似地区而非 GPS |
| 根据实测地区选择自动方案 | 纯引擎选择 / 兜底已测试，真实数据和执行未接通 | P3c：有效底层 IP → GeoIP → 允许的方案 → 实际执行 / 回执；缺测、过期、无数据或多源地区冲突使用兜底 |

本轮只读查询三个本地 Demo 的发现状态均为 `disabled`，socket 与 observation 数量均为 0；
现网已经有的邻接 RTT 历史与 STUN / 地区观测是不同数据，不相互替代。

容易遗漏的场景一并纳入后续联测：仅使用 TCP/VLESS、没有 UDP carrier；所有 UDP 观察点失败；
网络 / IP 变化与旧样本过期；IPv4/IPv6 或多网卡观测不一致；GeoIP 数据缺失 / 更新失败；
Docker 多层 NAT 与端口暴露。独立探测 socket 若用于地区依据，须有独立来源，不能当 listener
映射或 peer 打洞候选。端到端业务测试还须区分单向 grant 的状态型回包与反向主动访问，
不能只用全部双向允许的模板掩盖回包能力缺口。

- **打洞 / 主备**：验证 NAT、UDP blocked、反向 stream、endpoint 重绑定、失败 / 回切、防重放与 lease。承载变化不得跳过必经节点，不交付首版同流聚合。
- **DNS / 出口**：域名上下文来源仍待确认；解析路径与出口一致、回包 / NAT / 授权撤回及无泄漏均须真实隔离验收。设备不再新增手工 DNS / STUN 配置流程。
- **IPv6**：需真实双栈 TUN、ACL、MTU / ICMP 与 fail-closed，AAAA / 底层 IPv6 观测不是完成证据。
- **Trace**：需受认证的实际逐跳探测，规划路径和邻接 nonce 不冒充 trace；原生 traceroute 兼容另行评估。
- **身份 / 平台**：密钥轮换、通用 resolver 安装与恢复、Apple 真机 NE（待签名条件）、兼容 / HA 独立推进。OIDC、邮件发送不阻塞首版本地邀请账户。
