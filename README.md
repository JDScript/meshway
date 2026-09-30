# Meshway

基于 Rust 的可编程 mesh 网络：身份与访问控制、域名/IP 策略路由、有序多跳，以及可替换的链路承载。

## 工作区

| 目录 | 职责 |
| --- | --- |
| `apps/controller` | Rust 控制面 |
| `apps/agent` | Rust 节点进程 |
| `crates/protocol` | 共享 Rust 协议定义；Agent 生成客户端位于 `apps/agent/src/generated` |
| `crates/policy` | 纯 Rust 路由策略规范化、校验与逻辑路径预览 |
| `apps/web` | Next.js + shadcn/ui 控制台 |
| `packages/control-client` | 生成的 TypeScript 协议、SDK 与运行时校验 |
| `docs` | VitePress 产品与架构文档 |
| `references` | 独立的上游参考 checkout；版本见 [研究索引](references/README.md) |

## 开发环境

- Rust 1.98.0（由 `rust-toolchain.toml` 固定）
- Node.js 24
- pnpm 11.24.0

```sh
pnpm install
pnpm dev:docs
```

文档站：<http://localhost:5173>。控制台通过 `pnpm dev:web` 启动于 <http://localhost:3000>。

Rust 启动命令和契约生成流程见文档站的开发指南。

分别在终端中运行控制面和 Web：

```sh
cargo run --locked -p meshway-controller -- serve --data-dir .meshway/controller
pnpm dev:web
```

首次启动会创建 SQLite 状态及私有的 `.meshway/controller/admin.token`。在另一终端显式读取管理员令牌，并在 Web 登录：

```sh
cargo run --locked -p meshway-controller -- admin-token --data-dir .meshway/controller
```

在 Web 创建一次性注册令牌，然后注册并运行 Agent：

```sh
cargo run --locked -p meshway-agent -- enroll \
  --controller http://127.0.0.1:8420 \
  --token '<Web 创建的一次性注册令牌>' \
  --name nyc --state .meshway/agent.json
cargo run --locked -p meshway-agent -- run --state .meshway/agent.json
```

保留同一数据目录与 Agent 状态文件，即可在重启后继续使用原身份和已分配的 mesh 地址。`.meshway/` 不进入 Git。

`run` 持续发送心跳并同步配置；也可以单次拉取、校验、保存并确认当前配置：

```sh
cargo run --locked -p meshway-agent -- sync --state .meshway/agent.json
cargo run --locked -p meshway-agent -- status --state .meshway/agent.json
```

首次初始化默认使用 `100.96.0.0/16`，并生成和保存 IPv6 ULA `/64`。自定义前缀通过 controller 的 `--ipv4-prefix` / `--ipv6-prefix` 指定；现有数据目录不会隐式改址。路由编辑器位于 `/routing`。

Web 默认从 `http://127.0.0.1:8420` 读取 controller；可通过 `MESHWAY_CONTROLLER_URL` 配置。

控制台提供 dashboard、节点管理、路由规则编辑、逻辑拓扑与监控视图。规则在 Sheet 中编辑并应用到草稿，再统一预览 / 保存；草稿及其版本基准可在已登录界面的页面间恢复。团队入口和未测量指标的边界见 [Web 产品与交互](docs/web-product.md)。

## 当前范围

当前控制面包含一次性令牌注册、SQLite 持久清单、双栈 mesh 地址分配、持续心跳、版本化配置、节点撤销，以及路由策略编辑、编译预览和条件保存。Agent 校验配置后私有、原子地保存，再发送版本与摘要确认。`last_seen` 表示最近联系 controller 的时间；配置 ACK 表示存储回执，与数据面生效分开记录。

路由按有序 first-match 选择；首版域名规则仅支持标签边界 suffix，另支持节点、IPv4/IPv6 CIDR 与地区条件，默认动作可选 direct/reject，初始为空策略 reject。路径预览展示逻辑规划，访问授权、真实转发和 trace 在后续数据面中实现。

DNS、multi-transport 与加密转发的交付阶段在文档站维护。优先保留端到端加密与 `meshway trace`，系统 traceroute 作为后续增强；可信逐跳路由需要显式选择。

## 类型安全

HTTP 契约从 Rust 类型和实际路由声明生成，并驱动 Rust agent client、TypeScript SDK 与运行时 schema。前后端使用同一份生成契约，禁止手工复制 DTO。

CI 检查契约漂移、Rust 格式/Clippy/测试、TypeScript、前端 lint、两个站点构建，以及真实 controller / agent / Web 的浏览器联调。

```sh
pnpm protocol:generate  # 修改 Rust 契约后更新 OpenAPI 和客户端
pnpm protocol:check     # 逐文件比较重新生成的产物
pnpm test:protocol     # SDK 校验及真实 controller 往返
pnpm test:web          # 生成 schema 与路由草稿语义
pnpm typecheck
pnpm lint
pnpm build
pnpm exec playwright install chromium  # 首次运行浏览器测试
pnpm test:e2e                          # 独立临时数据与端口，覆盖注册、策略编辑、配置回执与故障
```

具体生成器及当前支持的契约形状见 [协议文档](docs/protocol.md)。
