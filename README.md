# Meshway

以 Tailscale 为常规产品基线的 Rust mesh 网络，扩展**可插拔的自定义 transport**与**按域名 / IP 经有序指定路径的路由**。Mesh 私网互访和互联网分流同为首版目标。

## 产品目标与当前实现

**已确认目标**：一个 controller 管理多张隔离网络，一台设备一次只接入一张；本地邮箱 + 密码账户、仅邀请加入；全部业务配置由 Web 管理、控制面下发。四层策略是连接图、叠加 grants、Mesh 内具体度路由，以及出口路由方案（公共规则 + 方案规则 + 兜底）。用户可选择允许的「仅 Mesh」「全局经 X」「自动按 STUN 地区」等方案。

**当前基线**：单网络 SQLite、管理员 bearer、节点令牌注册、App PKCE 浏览器批准、IPAM、版本化配置 / ACK、公钥绑定、托管 Native UDP / TCP / TLS-VLESS 邻接、Linux Mesh IPv4 TUN 与 Noise E2E、节点 DNS、只读 daemon CLI。旧版 Access / Routing 有序策略与本地 profiles 尚待迁移；多网络、用户账户、平台全托管、按需自动建连、出口 / 路由方案、可信域名上下文、IPv6 TUN 与逐跳 trace 尚待交付。

配置 ACK 是校验并持久保存，心跳是联系记录，路径预览是规划；都不能代替真实隧道 / ICMP / HTTP 验证。Apple App 已有浏览器接入与库存显示，隧道未实现。

产品要求权威见 [产品对齐记录](docs/product-alignment.md)，精简概念见 [产品概览](docs/concepts.md)，交付依赖见 [里程碑](docs/milestones.md)。

## 工作区

| 目录 | 职责 |
| --- | --- |
| `apps/controller` / `apps/agent` | Rust 控制面与节点进程；Agent 生成客户端位于 `apps/agent/src/generated` |
| `crates/protocol` / `crates/policy` / `crates/dataplane` | Rust 契约、纯策略编译、认证与加密转发 |
| `crates/transport-*` | 编译期承载适配 |
| `apps/web` | Next.js + shadcn/ui 控制台 |
| `apps/apple` | SwiftUI + NetworkExtension、UniFFI Rust bridge |
| `packages/protocol` / `packages/control-client` | 生成 OpenAPI、TypeScript SDK 与运行时校验 |
| `docs` | VitePress 产品、架构、开发与参考文档 |
| `experiments` | 独立隔离验收与性能工具 |
| `references` | 只读上游 checkout，版本见 [研究索引](references/README.md) |

## 开发入口

需要 Rust 1.98.0、Node.js 24、pnpm 11.24.0；统一入口使用 Python 3、tmux、lsof。

```sh
pnpm install --frozen-lockfile
pnpm dev
pnpm dev:status
pnpm dev:restart web
pnpm dev:update
pnpm dev:stop
```

统一入口为 `scripts/dev.py`，Controller / Web / 文档默认在 8420 / 3000 / 5173，仅管理当前 checkout 及已初始化 demo。身份和运行数据在忽略的 `.meshway/`。行为与更新边界见 [统一开发入口](docs/dev-services.md)，首次注册与单组件启动见 [本地联调](docs/getting-started.md)。

```sh
pnpm dev:web
pnpm dev:docs
pnpm build:docs
```

## 契约与验证

HTTP API 由 Rust DTO 和实际路由生成，Agent / Web 使用生成客户端，Web 对外部响应做生成 schema 校验；禁止手工复制 DTO / endpoint。每个 Web 管理 mutation 均须服务端认证。

```sh
pnpm protocol:generate  # 修改 Rust 契约后生成
pnpm protocol:check     # 生成物 drift check
pnpm test:protocol
pnpm test:web
pnpm typecheck
pnpm lint
pnpm build
```

Rust fmt / Clippy / 行为测试与实验入口见 [开发与验收](docs/development.md)。Web 不保留浏览器自动化套件，UI 在开发页面手工验证。文档构建保留死链检查；实验结果必须保留负载、资源与加密条件，不能把原生 L4 与完整 L3 隧道差值当作包装增量成本。
