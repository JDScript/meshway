# 开发路线与验收

本页记录已验证的注册基线、配置 / 路由切片及后续网络里程碑。后续顺序是推荐推进路径，不代表发布时间承诺。

## 已验证基线：注册与持久化切片

Rust controller / Agent、生成客户端与真实 API 驱动的 Web 已交付控制面凭据身份闭环。该切片属于里程碑 1 的已完成部分，下表记录其实现范围。

| 范围 | 当前边界 |
| --- | --- |
| Rust controller | 单实例 SQLite；管理员 bearer、限时一次性注册令牌、节点清单、心跳与撤销 |
| Rust Agent | 通过生成客户端注册 / 心跳；私有原子状态文件复用身份，支持响应丢失后的恢复 |
| 协议链 | Rust DTO + 实际路由 + 认证元数据 → OpenAPI → Rust / TS 客户端；严格运行时验证、类型化 HTTP 401 与漂移检查 |
| Next.js + shadcn | 管理员登录 / 登出、HttpOnly cookie、受保护 server action；真实清单、令牌创建与撤销 |
| VitePress | 中文架构、约束、研究依据与开发说明 |
| 数据面 | 尚未实现 TUN、DNS 接管、多跳转发、多传输、E2E 或 trace |

2026-09-30 注册基线集成结果：**17 项 Rust 测试（8 项 Agent、7 项 controller、2 项生成适配器）、4 项 SDK 测试及浏览器端到端测试通过**，Rust fmt / Clippy、TypeScript 检查和代码生成漂移检查通过。端到端测试使用生产构建的 Next.js、真实 controller 与 Rust Agent，数据目录和端口独立创建。这是该基线的历史证据，不是本轮扩展后的测试数量或通过声明。

这一阶段验证 HTTP 控制面凭据身份及持久化，不建立 mesh 公钥认证或 E2E 会话。具体语义见[注册与持久化](/enrollment)。

## 注册与持久化验收矩阵

以下是注册基线已通过的行为覆盖。证据来源：**Controller** 为 `apps/controller/tests/api.rs`，**Agent** 为 `apps/agent/tests/roundtrip.rs`，**SDK** 为 `packages/control-client/test/client.test.ts`，**E2E** 为 `e2e/control-plane.spec.ts`。本轮配置扩展后的回归结果另列于下节。

| 场景 | 已通过的结果 | 证据 |
| --- | --- | --- |
| controller / Agent 重启 | 同数据目录与状态文件继续使用相同节点 ID；节点、已消费令牌和撤销状态保留，原管理员凭据仍有效 | Controller、Agent、SDK；E2E 验证管理员与节点身份 |
| 非法 / 过期注册 | 无效令牌、未消费的过期令牌、非法 TTL、请求格式 / 大小和字段被拒绝；失败请求不误消费有效令牌 | Controller |
| 重复与并发消费 | 16 个不同凭据竞争同一令牌只创建一个节点；16 个相同绑定重试返回同一节点，总数仍为 1 | Controller |
| 同身份幂等重试 | 同令牌、凭据及原元数据返回同一有效节点，包括已消费令牌过期后的恢复；改变绑定或撤销后不能恢复 | Controller、Agent、SDK |
| 丢失成功响应 | 模拟服务端提交、客户端未保存响应后，用持久凭据心跳恢复节点 ID，再次注册仍是同一身份 | Agent |
| 权限范围 | 节点 / 注册凭据不能读取概览或清单、创建令牌、撤销节点；心跳拒绝调用方提交的其他节点 ID | Controller、SDK |
| 撤销与重启 | 已撤销节点心跳 / 原注册重试返回 403，重启后仍生效；再次撤销保留原时间，总数仍包含该节点 | Controller、SDK；E2E 验证 UI 撤销到 Agent 拒绝 |
| 私有文件及失败路径 | 0700 / 0600 权限、独占锁、部分初始化、错误权限和链接被检查；损坏、不同绑定及请求期间被替换的状态不会被覆盖 | Controller、Agent |
| 心跳进程 | `run` 立即心跳、临时故障退避、成功后重置；401 / 403 使进程退出；`status` 不输出凭据 | Agent |
| Web 登录 / 登出 | 匿名访问跳转登录，错误令牌不产生登录 cookie；正确令牌产生 HttpOnly / Strict cookie，登出后受保护页面被阻止 | E2E |
| Web mutation | 创建真实注册令牌；匿名重放捕获到的同一个 Next.js action 请求被拒绝并跳转登录；UI 可撤销真实 Agent | E2E |
| 注册命令与名称 | 浏览器完整接收 128 个 emoji，拒绝 129 个；实际展示的命令经 shell 解析后，让真实 Agent 保留以 `-` 开头、包含单引号、空格和 Unicode 的名称 | E2E |
| 故障与显示 | controller 停止后显示读取错误，恢复后原会话可继续读取；390×844 视口无水平溢出，完整流程无浏览器 JS 错误 | E2E |
| 错误与运行时 schema | 类型化 HTTP 401 / 403 / 503；缺失 / 额外字段、非法时间、可空性及数值错误被拒绝；Unicode 长度与 Rust 一致 | SDK、Controller、Agent |
| 必填可空字段 | 源 DTO 与生成 Rust 客户端拒绝缺失的 `revoked_at`，接受显式 `null`；`None` 序列化保留 `"revoked_at": null` | Agent、生成适配器测试 |
| HTTP transport | Rust 客户端不跟随重定向；TS 响应媒体类型、大小、JSON 及 HTTP 错误分类有效 | Agent、SDK |

生成契约另外通过 `pnpm protocol:check` 验证 Rust DTO、路由、认证 metadata 与全部生成产物一致。文档通过 VitePress 构建及死链接检查。

撤销验收仅覆盖控制面凭据拒绝，不宣称切断活动加密流。`last_seen` 与可选服务存活检查均不充当隧道或 mesh 在线状态。

## 配置与路由切片

**当前状态：已实现并通过集成验收。** 首版域名 suffix-only、first-match、可配置 direct / reject 默认动作（初始 reject）已贯通 Rust 编译器、控制面、生成客户端、Agent 和 Web；exact / prefix 不在首版范围。源类型与编译器分别位于 `crates/protocol` 和 `crates/policy`。

2026-09-30 集成及审查复验记录：

| 检查批次 | 已通过结果 |
| --- | --- |
| 初轮 Rust workspace | 48 项测试 |
| 独立审查修复后 | policy 26 项测试；受影响核心 30 项测试，分别记录，不与初轮数量相加 |
| 当前 SDK | 5 项测试，`packages/control-client/test/client.test.ts` |
| 当前 TypeScript | 完整工作区与 E2E 类型检查 |
| 真实浏览器端到端 | 2 项 Playwright 测试：`e2e/control-plane.spec.ts` 与 `e2e/routing.spec.ts`；生产构建 Next.js、真实 controller 与 Rust Agent |

两项独立审查问题均已修复并重测：IPv4-mapped IPv6 在分类前拒绝；文件写入明确区分 `NotCommitted` 与 `CommittedNotDurable`，每次 ACK 前重新确认文件和父目录持久性。

### 新切片验收清单

以下行为覆盖已通过；路径均为仓库内的实际测试证据：

| 场景 | 已通过的行为 | 证据 |
| --- | --- | --- |
| 双栈分配与开发数据库升级 | 构造的 v1 数据库保留管理员、节点 / 凭据、令牌绑定及撤销；按稳定 ID 补地址，容量不足回滚，撤销不复用，换池拒绝 | `apps/controller/src/store/tests.rs` |
| revision、策略 CAS 与回执 | 清单 / 国家 / 策略变化增加全局版本；幂等操作、心跳 / ACK 不增加；节点绑定、旧 / 未来版本及错误 digest 拒绝；无效策略不保存 | `apps/controller/src/store/tests.rs`、`apps/controller/tests/api.rs` |
| 配置校验与缓存 | schema / 绑定 / digest / 网络 / 策略校验，拒绝旧版本与同版本变更；先保存再 ACK，409 竞争重试，重启后缓存稳定 | `apps/agent/tests/roundtrip.rs` |
| 发布前文件失败 | `0300` 父目录在发布前打开失败；保留旧缓存且不 ACK，恢复权限后可重试 | `apps/agent/tests/roundtrip.rs` 的 `unreadable_private_parent_fails_before_publication_and_never_acks` |
| 发布后持久性失败 | 测试专用故障注入产生 `CommittedNotDurable`：新文件已可见，内存保留已发布值；相同缓存重试及重启仍不 ACK，直到文件 / 目录同步恢复；凭据使用前也重新确认 | `crates/protocol/src/local/write.rs`、`apps/agent/src/state/durability_tests.rs` |
| 策略规范化与求值 | suffix 根域 / 子域 / 标签边界、IDNA / 大小写 / 末尾点、双栈 CIDR；来源 OR / 跨维 AND、目标 OR、first-match、两种默认动作 | `crates/policy/src/tests.rs` |
| 目标与必经路径 | IP 归属解析为 mesh，池内无有效归属不得互联网回退；节点 / IP / 国家一致性；非法引用、重复 hop、循环及非法出口拒绝，不跳过规则或 hop | `crates/policy/src/tests.rs` |
| IPv4-mapped IPv6 审查修复 | 点分及十六进制形式在分类前均为 `invalid`；已分配、未分配、外部地址和显式节点上下文不能绕过拒绝或默认 direct | `crates/policy/src/tests.rs`；`e2e/routing.spec.ts` 覆盖三类映射目标 |
| 类型与容量边界 | 策略 wire 的必填可空及不支持的域名种类；规则 / selector / via / 字节限制；地址耗尽及配置发布约束 | `crates/policy/src/tests.rs`、`apps/controller/src/store/tests.rs`、`apps/agent/tests/roundtrip.rs` |
| 生成 SDK 真实往返 | 管理读取、策略 / 国家操作、预览与条件保存；运行时响应校验及类型化错误 | `packages/control-client/test/client.test.ts` |

`e2e/routing.spec.ts` 使用独立数据目录和端口，验证以下真实服务闭环：

1. 注册 NYC、LAX、SHA 三个 Agent，双栈地址各自唯一，管理员设置国家标签。
2. 预览得到逻辑 `NYC → LAX → SHA`，IPv6 字面 mesh 地址解析到相同目标；域名 suffix 边界生效，IPv4-mapped IPv6 在默认 direct 前拒绝；预览不持久化。
3. Web `/routing` 编辑、预览和保存草稿；匿名重放捕获到的同一保存 action 被拒绝；并发写入导致 `409` 时保留草稿。
4. Agent 私有配置缓存存在后 controller 才接受回执；重复同步不改缓存 mtime，心跳和 ACK 不增加 revision。
5. controller 重启后地址、策略与回执稳定。
6. 撤销必经 LAX 后保留失效源策略，分发 deny-all 与诊断，Agent 保存并 ACK；UI 如实呈现失效状态，移动视口无水平溢出，流程无浏览器 JavaScript 错误。

Web 编辑基准 revision 仅在成功保存或显式重新读取后更新，预览不自动推进 CAS；国家编辑也使用 CAS。“已校验保存”回执标签不表示在线或网络生效。上述升级证据来自测试数据库，不代表既有运行实例已执行升级。

这些证据只能证明控制意图、落盘和回执。它们不证明 ACL 已授权、OS 网络已应用、物理路径可达、真实多跳或 trace。

## 里程碑 1：身份与策略基础

连接控制意图切片新增公钥关联、编译能力声明、管理连接计划及独立存储 ACK。覆盖 SQLite v2→v3 保留身份、不可替换 / 不可复用公钥、全局 CAS、节点局部投影、未知响应字段拒绝、发布前 / 后持久性故障及真实 Web 编辑；见[公钥与连接配置](/connections)。`e2e/connections.spec.ts` 验证 SHA→LAX 的建连方向、回执独立性、匿名重放拒绝、原始编辑基准及未应用表单的跨页恢复。

本里程碑的**控制意图切片已完成并验收**：持久清单、控制面注册 / 凭据权限 / 心跳 / 撤销、双栈地址、版本化配置与回执、逻辑策略和管理编辑闭环。整个身份与网络基础里程碑仍为**部分完成**，后续仍需：

- 控制面公钥关联已交付；运行层对等认证与受管理身份的连接、密钥生命周期仍需推进，继续分离身份和观测标签。
- 数据面会话的配置应用、撤销传播和密钥更新。
- 独立授权及传输能力模型，将逻辑意图连接到实际数据面。
- 物理邻接、传输能力和不可满足路径的运行期诊断。

**验收边界：** 已通过的配置回执与逻辑预览不完成 mesh 公钥身份、ACL 或网络会话；这些能力需独立实现与验收。

## 里程碑 2：真实三节点转发与追踪

托管邻接已在隔离 Docker 环境通过真实注册、DNS 入口、UDP / VLESS-TLS 双向认证探测、配置撤回、控制面断联后有效期截止、原身份恢复与撤销验收；节点页显示独立的应用 / 观测状态。该能力只建立邻接，不把连接许可转换成 IP / 中转授权，完整业务转发与真实逐跳 trace 仍按下列目标继续推进。见[连接应用与地址](/connection-runtime)。

独立 `transport-lab` 切片开始交付真实数据面：采用静态私有身份、精确有序路径授权、Noise E2E，以及原生 UDP / TCP 与 VLESS 适配。SHA 主动 dial LAX 后，LAX 复用已接受通道向 SHA 转发；Linux TUN 与 Docker / iperf3 工具用于实测。范围和可运行命令见 [Linux 传输实验](/transport-lab)。该实验尚未绑定 controller 节点身份，也不提供逐跳 trace。

建立可重复的 `NYC → LAX → SHA` 测试环境，先选一种合适承载打通真实数据面，再扩展候选。

- 业务流量实际经过 LAX，由 SHA 作为 mesh 目标或外部出口。
- 按明确选择的安全模式工作，优先实现 NYC → SHA 的 E2E。
- `meshway trace` 发送真实探测，输出计划和观测，不能只打印配置路径。
- 必经 LAX 失效时请求失败，并报告失败段；不能直连 SHA 绕过约束。
- 覆盖回程、MTU、大包、IPv4 / IPv6 以及拒绝访问。

**验收重点：** 用真实转发和流量观测证明中间 hop 被使用；以故障注入证明必经约束；以协议与抓包检查加密边界。基础 ping 成功不够。

## 里程碑 3：DNS 与域名策略

- 节点清单 → 本地解析器 → A / AAAA / PTR / 搜索域。
- Split DNS 及首批操作系统集成与退出恢复。
- 选择 DNS 关联 / FakeIP / 显式代理的初始覆盖范围。
- 域名规范化、标签级 suffix、TTL、缓存歧义与未知上下文行为。
- 解析出口与数据路径一致；控制 socket 与 bootstrap 连接不被自身 TUN 捕获。

**验收重点：** 共享 IP、应用 DoH、直接 IP、DNS 过期、网络切换及系统 DNS 恢复。`.cn`、中国域名集合与中国 IP 集合分别测试。

## 里程碑 4：多传输与故障行为

- 相同逻辑相邻节点间配置多个有优先级的候选，首版按已确认的主备方式工作。
- 能力过滤、真实健康检查、按流固定、冷却与回切防抖。
- 同一安全模式内进行传输回退，输出选择与失败原因。
- 验证 stream 断开语义、数据报丢包、乱序与 MTU 差异。

**验收重点：** 传输失败时路径不变、必经节点不变、加密语义不变。路径级故障切换需要独立政策，不能混入候选选择。

按流负载分担、同流多路径、逐包条带化、bonding 与原生 traceroute 兼容放在后续评估，当前均未实现。

## 开发检查

在仓库根目录执行，根脚本由工作区统一维护：

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
pnpm protocol:check
pnpm test:protocol
pnpm typecheck
pnpm lint
pnpm build
pnpm test:e2e
```

修改 Rust 契约后先执行 `pnpm protocol:generate` 更新产物，再执行上述检查。生成操作不能替代 `protocol:check` 的漂移比较；根脚本别名以根 `package.json` 为准。

首次运行 E2E 前安装浏览器：

```sh
pnpm exec playwright install chromium
```

`pnpm test:e2e` 先执行 `cargo build --workspace --features meshway-agent/transport-lab --locked` 和 `pnpm build:web`，再运行 Playwright。承载 feature 用于验证 Agent 发布实际已编译能力，连接配置测试不会因此启动 TUN 或隧道。测试自动启动真实 controller、Rust Agent 和生产 Next.js 服务，使用临时数据目录及独立端口，并在结束后清理。可通过 `MESHWAY_E2E_TMPDIR` 指定临时目录的父路径。

`pnpm typecheck` 同时覆盖工作区包与根目录 E2E TypeScript；单独检查测试代码可执行 `pnpm typecheck:e2e`。

文档构建保留 VitePress 默认死链接检查。参考源码使用固定提交的 GitHub 链接，不引用构建产物之外的本地 `references/` 路径。

仅修改文档时可执行 `pnpm build:docs`，无需反复运行完整工作区构建。协议与整体验证由协调集成统一执行。

## 仍需冻结的设计决策

1. mesh 公钥认证、密钥生命周期和 E2E 封装；首版是否额外交付 trusted-routing。
2. 由原生 UDP / TCP 和 VLESS 实验验证的传输接口如何接入生产配置、能力协商与多候选生命周期。
3. 真实数据面域名上下文的来源、可信度、过期与歧义处理；首版 suffix-only、first-match 和默认动作已确定。
4. DNS 平台顺序、FakeIP 是否首版必需、上下文关联策略。
5. 路径与回程对象、既有流在更新 / 撤销 / 故障时的处理。
6. 后续配置兼容窗口、协商 / 废弃流程与高可用部署模型；本轮单 controller SQLite 增量升级仅覆盖现有开发基线，远程 TLS 仍由部署层负责。

每个决定都应更新对应设计页、契约与验收，不应仅在 UI 中添加一个尚无实现的开关。
