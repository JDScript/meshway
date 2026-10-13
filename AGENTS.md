# Meshway

## 当前边界

- `apps/meshway-server`：Rust Server；`apps/meshway-web-console`：pure SPA（`@meshway/web-console`）；`apps/meshway-web-site`：对外站点（landing、用户文档、API 参考；Next.js + Fumadocs 静态导出，`@meshway/web-site`）。
- `crates/meshway-*`：当前共享模型、配置／编译、协议、生成客户端与严格序列化。
- Mesh routing 由 Policy 按每段实际起终点递归 first-match；via 的段边界视为已满足，真实循环／重复节点拒绝。原业务 Access／身份与原路径回复不改成分段业务；compilerVersion 为 `meshway-netmap-2`（边级承载顺序；dialer 无 priority；规则集按 digest 引用）。topology 定义 links（存在性＋承载顺序），routing 定义 paths（必经点＋出口），access 定义权限；listeners／dialers 是节点能力声明，不承载策略。配置结构归 Configuration；Server 用 `meshway-policy` 把配置投影成每节点 netmap（配置子集，不是编译结果），节点用同一 Policy 在投影上按首包懒编译准入与中继判定，投影等价测试防漏规则，见 `docs/v2/node/netmap-design.md`、`docs/v2/configuration-reference/routing.md`。规则集匹配、转换与格式只有 `meshway-rules` 一份实现；artifact 全局内容寻址、netmap 只带 digest，节点经 `GET /agent-api/v1/artifacts/{digest}` 取回并由 Control 缓存，缺失时命名它的规则不可用（fail closed），见 `docs/v2/configuration-reference/rule-sets.md`。
- 节点授权只有一份状态：`meshway-engine` 的 `Engine::reconfig(Option<netmap>)` 算出 `Desired`，connectivity／dataplane／underlay／dns／diagnostics 各暴露一个幂等 `apply(desired)`，`apply(empty)` 即全撤（underlay 保留负向边界）；没有 guard／generation／lineage／fence／epoch。Server 对每份 netmap 租约 ed25519 签名（钥在 enrollment 时钉住），节点验签一次后缓存 `netmap.json`，租约（默认 24 h，下限 60 s）内离线可用，到期 fail closed。`crates/` 不得出现第二份授权状态或签发实现，基准／fixture 的静态 desired 只放 `tools/`，`meshwayd` 只装配。
- `packages/`：生成 OpenAPI／TypeScript SDK，以及 Console 与站点共享的主题和基础组件（`@meshway/ui`）；`docs/v2/`：当前唯一开发者规范与使用入口。
- Rust 包根目录和 Cargo 包名统一 `meshway-xxx`。现行 HTTP 根仍是 `/api/v1`／`/agent-api/v1`，目录整理不自行改 wire。
- Node 已有 `meshwayd`／`meshway`、IPC 10（Status 含 netmap ref／租约／sync 状态、本节点方案选择与本节点／peers、`SelectProfile`（`meshway exit`，经 `POST /agent-api/v1/routing-selection`；选择只归 Server，本机只读 netmap 中的选择）、`RotateKey`、`Logout`（经 `POST /agent-api/v1/leave` 自助注销；持久意图归 Control，未获回答时 phase `leaving` 重试）、`Netcheck`、`Up.enrollmentKey`、localEndpoint 诊断来源）和唯一 `meshway-control` owner；Node 以单一长轮询 `POST /agent-api/v1/sync` 取 netmap 与 peer 事实（无 Node SSE；用户 SSE 保留），hold 超时回 Unchanged 即兜底，且不越过节点持有的租约（至少余 15 s）。Linux 默认本机 `--network-mode tun`，可显式选择 `control-only`，失败不回退。Mesh-only TUN capture、Underlay 资源／socket 保护、认证 stream／Native UDP 邻接（按需建立、空闲 120 s 关闭；topology `keepalive` 边和只有一方能拨出的边常驻）、SecureNode 包流、首包本地判定（零 Server 往返）、租约续租和真实安装回执已接线。默认 DNS 支持经验证的动态 stub resolved 后端及显式归属的 resolv.conf 代理，独立 DNS 锚点保留至 down；resolver 使用精确执行许可，任何必需组件失败均阻断整体 applied。隔离 Linux 验证覆盖 A→R→B、内层双栈 UDP／TCP、Native TCP／Native UDP、125 秒持续／仅接收流、DNS 实际系统解析／重启恢复、撤销／恢复、租约到期、节点重启离线载入、Server 停机 10 分钟离线运行和离线精确清理；不同承载／DNS 组合的验收状态以 `docs/v2/node/validation.md` 为唯一定义。owner-only socket 仅允许 root／daemon 同 UID，业务配置归 Server；策略归 policy，唯一流表／Noise／逐跳证明归 dataplane，desired 计算归 engine，sync／验签／缓存归 Control。Exit/NAT 首版已支持（Linux IPv4 内核转发＋masquerade、独立出口 TUN、nft 批事务、conntrack 精确清理、两阶段 ACK、出口投影随租约存续、租约到期 Retire＋负向边界；`exit/off` 为默认验收行）；出口 DNS 已支持（`meshway_policy::compile::dns_resolution` 唯一判定：方案含域名规则或 fallback 为 exit 时普通查询经该出口的 `Purpose::Dns` circuit 交出口系统 resolver，纯 IP 规则本机解析、多出口可配；域名规则分布在多个出口为 `dns_exit_plan_required`，Server 保存时 422 拒绝；出口 resolver 不可用只对源 SERVFAIL 并报 `exit.dnsUnavailable`／`NodeState.exitDns`，见 `docs/v2/node/dataplane.md` 出口 resolver）；每通道路径 MTU 由 Manager 认证探测，数据面对超限本机包合成 Frag Needed／Packet Too Big，underlay 按通道上限做 TCP MSS clamp（`meshway_mss_*` 表）；可信域名关联已支持（出口 netmap 的 `ServeExitResolver` service 为 Server 预签常驻任务，出口回答名称规则决定的查询后经 `POST /agent-api/v1/dns-associations` 完成一次，Server 校验后为源与出口各签 `DnsAssociationOverlay`（内存、按条目 TTL 截止、随 `sync` 交付、任一方 netmap 变化即重验），Engine 把关联并入 `Desired`，源节点在关联生效前不放行该回答；唯一实现见 `docs/v2/node/exit-design.md` 可信域名关联）；显式多出口 DNS plan、IPv6 出口、SOCKS5/netstack、切网和插件仍不支持；系统服务：`dist/systemd/meshwayd.service`（`Type=notify`、最小 capability、`ProtectSystem=strict`）＋`dist/install.sh`，tag `v*` 经 `release.yml` 发布；实测整机重启（含 Server 不可达）无需 `up` 即从 `netmap.json` 恢复 running；resolved 归属由 namespace＋锚点＋标记链路设置证明，systemd-resolved 重启后 5 s 内、停机再启动后 10 s 内恢复（停机期间 fail closed），常驻验收 VM 为 `meshway-vm`，见 `docs/v2/install.md`。行为定义见 `docs/v2/node/dataplane.md`、`docs/v2/node/runtime.md`，验收状态见 `docs/v2/node/validation.md`。
- STUN、State 唯一候选归属、双边接受的短时协调 UDP 穿透已接入同一 Manager；Control 合法直接交接保留 Native TCP／UDP 物理通道，旧操作 scope 仍立即失效。双 IPv4 端口保持型 SNAT、空 UDP 发布端点、125 秒包流／仅接收续租与原路径 TCP 退路已完成隔离验收。映射轮换、端点相关映射、外层 IPv6 NAT 与 NAT／系统 DNS 组合分别验收；精确范围见 `docs/v2/node/connectivity.md`。
- Node 控制联系失败按真实错误分类恢复；网络／暂态响应、运行 session／instance 失效不永久卡入 Contract，当前身份和目标绑定错误仍阻断。静态 TCP／UDP、两种 DNS、双 NAT 与 TCP 退路均已验证 HTTP 中断至实际 instance／execution 到期、无第二次 up 的新资格／State／新包流恢复及旧输出拒绝；验收状态见 `docs/v2/node/validation.md`。
- `meshway ping` 接受节点名／Mesh IP、公网 IP 或域名：先输出本地判定（Policy 的方案／规则／动作与完整路径，域名经本机 DNS 路径解析并显示来源与 X6 关联），再按动作探测——Mesh 与 exit 目标由本节点在 Engine `Desired` 上按首包同一判定（`icmp:*` 服务、完整 routing／必经路径且全程 observable）本地决定，无 Server 往返，复用唯一 SecureNode、Noise 与原路径回复；`meshway_dataplane::diagnostics` 仅拥有有界测量调度／计时。Node 实例与用户订阅共用 ObservationService，真实 channel／STUN acquisition 和单调 RTT 分开于 preparing；订阅确认截止独立于 HTTP，旧输出不换绑新权限。Native TCP／UDP、双 NAT、退路与系统 DNS 场景已有真实 CLI 证据，精确范围见 `docs/v2/node/diagnostics.md`。Underlay／Manager 按资源差量续接，descriptor 取消不可逆；旧 discovery／attempt／输出 guard 仍立即失效。
- 常驻 Node lab 使用锁定构建的可写 Ubuntu 工具镜像，含 ping／traceroute／iperf／iperf3 和独立 D-Bus／resolved；二进制只读、身份独立持久化。显式 image-build 与 deploy 分离，按真实 daemon PID／hash 核验；不得写 Docker 管理的宿主 DNS bind，详见 `docs/v2/node/docker-lab.md`。
- 数据面当前 wire 为 v6：握手 transcript 绑路径授权摘要（grant 键、完整路径、节点身份／地址／密钥、承载候选、出口 selection；不含服务集合）与 INIT 时刻（±2 s 窗口），稳态为定长 86 B 开销的相邻 MAC；中继按会话转发并以 `admit_transit` 在自己的投影上判定，业务流仅在端点准入；circuit 空闲 90 s／无回复 15 s（端点 10 s 被动 keepalive）退役，相邻对端换 boot（重启）即退役并由中继逐跳回传 `Close`。包处理采用每轮最多 32 包／事件和独立 guard 的有界发送流水线。历史／当前重验边界与共享观察恢复缺口以 `docs/v2/node/validation.md` 为准。
- 旧 Node、Apple、运行库与实验退出活跃树，保存在冻结 Git 分支及必要证据中。新实现只按明确白名单复用，不恢复长期 legacy 目录或旧 HTTP 兼容轨。Meshing 项目不是本项目依据。
- 冻结参考：`archive/pre-consolidation-main-007e9deb`、`archive/node-runtime-8da83a7f`、`archive/apple-007e9deb`。未完成配置补全与旧 review WIP 保存在 `archive/wip-*-20261003` 分支，归档提交不代表通过验收；私有证据保存在 `.meshway/worktree-archives/20261003/`。

## 信息来源与文档

- 产品、领域边界、字段和真实未完成项维护在 `docs/v2/`；每个主题只有一个定义位置，其他页面引用它。
- 文档采用对外参考格式：结构、字段、行为、示例、限制。移除逐轮讨论、修复日志、过时 handoff 和重复 dump；追溯使用 Git 与私有原始证据。
- HTTP 类型和实际路由生成 OpenAPI／SDK，文档原样发布同一契约。不能把未定设计写成可用 API，不能以文档描述替代真实行为验收。
- 新产品语义不确定时使用 question 集中询问。构建缓存、进度或测试数量不代表业务功能完成。

## 类型、权限与数据完整性

- Server↔Node／Console 使用生成客户端；外部响应使用生成运行时 schema。禁止 `any`、未验证断言或宽 JSON 掩盖协议不一致。
- 修改协议／路由后重生成并通过 drift，不手改生成文件。可省略、null、未知、过期和不适用分别表达。
- 逐请求／最终事务检查真实主体与网络权限；类型化 NetworkId 不代替授权。Node 凭据不能作为用户管理凭据。
- 配置、身份／材料绑定和执行发布保持一致；CAS、永久吊销、成员生命周期、lease、取消与回执排序不可削弱。普通启动不因数据库错误清库。
- 配置原文／规范值和必要秘密私有保存，公开视图脱敏；密码安全哈希，私有文件原子持久化，凭据不进普通日志。
- State、saved、applied、认证通道和真实包流分别验收。缺测不填零，重放／读取不制造历史样本；状态不会自授权限或延长 lease。
- 新 Node 保留端到端保护与原完整路径回复，承载切换不绕必经点；打洞、反向拨号、中继与真正直连分别证明。当前穿透实现目标限 UDP；Native TCP／VLESS／Trojan 要求拨号取得 TCP stream，不实现 TCP simultaneous-open。连接获取方式与协议封装分层，双向承载不等于双向业务授权。

## 源码组织

- App 做装配和用例协调，库按真实职责和消费者划分，不按表／临时阶段机械拆包，不让库反向依赖 App。
- 同一业务逻辑只有一个实现归属和状态 owner；多入口、平台适配、插件导出与测试复用它。并行任务先明确共享接口及所有权，缺少依赖时不另写一份替代实现；整合时检查重复实现、循环／反向依赖和遗留过渡路径。Node 跨入口边界见 `docs/v2/architecture.md`。
- 单文件模块 `foo.rs`；有子模块用 `foo/mod.rs`。私有单测为所属模块的 cfg(test) 子模块；黑盒测试遵守 Cargo `tests/*.rs` 或 `tests/<suite>/main.rs`，共享 helper 放子目录。
- 不为测试扩大生产可见性、复制实现或用临时 path/autotests 配置绕过边界。生成／vendor 由其权威来源维护。
- Rust 业务代码 forbid unsafe；确需 native/FFI 的桥接隔离审计，不静默放开应用 lint 或 TLS 默认 features。
- Rust 程序入口复用 `meshway-logging`，库只发 `tracing` 事件、不安装全局 subscriber；字段与 stdout／stderr 边界见 `docs/v2/logging.md`。
- 取消后的已接纳事务仍由数据库 owner 完成；异步生产者有明确停止／join，缓冲和任务有界。

## 验证与环境

- 工具链版本和 workspace 任务统一在 `mise.toml`，下载锁定在 `mise.lock`；使用 `mise run`／`mise exec`，本地与 CI 共用 `mise run check`。包内脚本由所属包维护；根目录不另维护一套 pnpm 任务别名。
- Rust：fmt、严格 Clippy、必要行为测试；协议改动还检查 workspace/all-features、drift、Rust／TS SDK 消费者。当前 workspace 含 Server、共享库和 Node control；控制流程不代替 Node 包流／Apple 接通验收。
- 性能测量由独立脚本显式运行并出报告，不进入常规测试或 `mise run check`；共用正式实现，分层口径与报告契约见 `docs/v2/node/performance.md`。
- Console：严格 TypeScript、格式／lint、领域测试和构建；UI 人工验证，不维护浏览器自动化套件。
- 文档：`mise run check:docs` 检查构建、链接／锚点、公开 OpenAPI 一致性及标注完整示例；外部链接按需额外核对。
- 运行服务、身份数据与源码合并分开处理。先核对实际进程／接口；不把重新编译当作部署，不覆盖另一工作流账户／配置。
- 不改未知归属的宿主路由／DNS／防火墙／Docker daemon，不用 host network 或 privileged（含构建器）。测试资源隔离并可精确清理。
- 大型构建前检查磁盘，低于 50 GiB 先处理已结束且无进程／挂载引用的缓存。`CARGO_INCREMENTAL=0`，相同任务复用指定 target；不同 feature 的可执行测试不并发共用 target。不全量 prune 未知 volumes。

## 协作

- 并行工作明确文件所有权，manifest／lock／生成链及最终整合由协调者维护。跨组件行为一起验证，不把类型、存储、HTTP 分别当业务交付。
- agent 沟通只用原生 subagent 回执并标明来源；不改已有历史。跨会话交接（如 Console 会话 `ses_6b3d4dd249b14747b880d95546790add`）可经 OpenCode `session.prompt` 投递，正文首行必须标明来源会话、经用户授权、非用户本人输入；不得伪装成 user。子任务默认使用用户指定的 `openai/gpt-6-astra-ultrafast`。
- 工作树统一放仓库 `.worktree/`。完成、整合、验证且无 WIP／进程／挂载引用后回收，保留分支和必要证据。遇到未知变化先调查，不重置或删除。
- 独立完成的工作形成清楚提交；主线合并前核对源码、生成物、文档和实际运行命令一致。
