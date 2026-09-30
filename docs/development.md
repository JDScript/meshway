# 开发路线与验收

本页区分当前基础切片与后续网络里程碑。顺序是推荐推进路径，不是已完成清单，也不代表发布时间承诺。

## 当前：注册与持久化切片（已实现）

Rust controller / Agent、生成客户端与真实 API 驱动的 Web 已交付控制面凭据身份闭环。该切片属于里程碑 1 的已完成部分，下表记录其实现范围。

| 范围 | 当前边界 |
| --- | --- |
| Rust controller | 单实例 SQLite；管理员 bearer、限时一次性注册令牌、节点清单、心跳与撤销 |
| Rust Agent | 通过生成客户端注册 / 心跳；私有原子状态文件复用身份，支持响应丢失后的恢复 |
| 协议链 | Rust DTO + 实际路由 + 认证元数据 → OpenAPI → Rust / TS 客户端；严格运行时验证、类型化 HTTP 401 与漂移检查 |
| Next.js + shadcn | 管理员登录 / 登出、HttpOnly cookie、受保护 server action；真实清单、令牌创建与撤销 |
| VitePress | 中文架构、约束、研究依据与开发说明 |
| 数据面 | 尚未实现 TUN、DNS 接管、多跳转发、多传输、E2E 或 trace |

2026-09-30 集成结果：**17 项 Rust 测试（8 项 Agent、7 项 controller、2 项生成适配器）、4 项 SDK 测试及浏览器端到端测试通过**，Rust fmt / Clippy、TypeScript 检查和代码生成漂移检查通过。端到端测试使用生产构建的 Next.js、真实 controller 与 Rust Agent，数据目录和端口独立创建。

这一阶段验证 HTTP 控制面凭据身份及持久化，不建立 mesh 公钥认证或 E2E 会话。具体语义见[注册与持久化](/enrollment)。

## 注册与持久化验收矩阵

以下是本次已通过的行为覆盖。证据来源：**Controller** 为 `apps/controller/tests/api.rs`，**Agent** 为 `apps/agent/tests/roundtrip.rs`，**SDK** 为 `packages/control-client/test/client.test.ts`，**E2E** 为 `e2e/control-plane.spec.ts`。

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

## 里程碑 1：身份与策略基础

本里程碑**部分完成**：持久清单、控制面注册、凭据权限、心跳与撤销已实现并验收。仍待实现：

- mesh 公钥绑定、对等节点认证与密钥生命周期，分离可信身份和观测标签。
- mesh 地址、版本化配置及分发，以及未来数据面会话的撤销传播。
- 分类规则、独立授权、有序路径、出口与传输能力的类型模型。
- 编译期拒绝循环、非法节点、缺失能力与不可满足路径。

**后续验收重点：** mesh 身份、地址 / 配置分发及策略验证，无效策略产生可解释的拒绝，Web 与 Agent 使用同一契约。

## 里程碑 2：真实三节点转发与追踪

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

`pnpm test:e2e` 先执行 `cargo build --workspace --locked` 和 `pnpm build:web`，再运行 Playwright。测试自动启动真实 controller、Rust Agent 和生产 Next.js 服务，使用临时数据目录及独立端口，并在结束后清理。可通过 `MESHWAY_E2E_TMPDIR` 指定临时目录的父路径。

`pnpm typecheck` 同时覆盖工作区包与根目录 E2E TypeScript；单独检查测试代码可执行 `pnpm typecheck:e2e`。

文档构建保留 VitePress 默认死链接检查。参考源码使用固定提交的 GitHub 链接，不引用构建产物之外的本地 `references/` 路径。

仅修改文档时可执行 `pnpm build:docs`，无需反复运行完整工作区构建。协议与整体验证由协调集成统一执行。

## 仍需冻结的设计决策

1. mesh 公钥认证、密钥生命周期和 E2E 封装；首版是否额外交付 trusted-routing。
2. 第一种数据面承载与传输能力接口。
3. 域名 prefix 精确语义、无匹配 / 域名未知时的行为。
4. DNS 平台顺序、FakeIP 是否首版必需、上下文关联策略。
5. 路径与回程对象、既有流在更新 / 撤销 / 故障时的处理。
6. 版本化配置分发、后续兼容窗口与高可用部署模型；本切片已选择单 controller SQLite，远程 TLS 由部署层负责。

每个决定都应更新对应设计页、契约与验收，不应仅在 UI 中添加一个尚无实现的开关。
