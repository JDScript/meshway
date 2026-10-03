# Meshway

以 Tailscale 为常规产品基线的 Rust mesh 网络，扩展**可插拔的自定义 transport**与**按域名 / IP 经有序指定路径的路由**。Mesh 私网互访和互联网分流同为首版目标。

## 产品目标与当前实现

**已确认目标**：一个 controller 管理多张隔离网络，一台设备一次只接入一张；本地邮箱 + 密码账户、仅邀请加入；业务配置由 Web 管理、控制面下发。四层策略是连接图、叠加 grants、Mesh 内有序 first-match 必经路由，以及出口路由方案（公共规则 + 方案规则 + 兜底）。用户可选择允许的「仅 Mesh」「全局经 X」「自动按 STUN 地区」等方案。

**当前基线**：多网络 SQLite、本地用户账户与网络 RBAC、邀请/设备归属、节点令牌注册与 App PKCE、IPAM、revision/CAS/ACK、公钥绑定、平台托管 Native UDP / TCP / VLESS TCP/TLS 与 WebSocket/TLS、Linux Mesh IPv4 TUN、四层策略的 Mesh v3 正常执行、Noise E2E、节点 DNS 和 daemon 诊断。固定 `4c9a5be` 同身份直连及四地有序路径恢复复验 PASS；KR→NYC 底层经 WARP，不是纯公网直连或打洞证明。

**尚未交付**：按需自动建连/主备、真实打洞、Mesh 状态型回包、完整出口/路由方案/域名分流、平台默认 STUN/GeoIP、CLI 链接登录、IPv6 TUN 与逐跳 trace。Linux underlay 已接正常生命周期并通过隔离包流/恢复验证，远端未部署；transport-neutral NAT 协调仍在实现。旧策略迁移与 profiles 兼容不自动改变身份或授权。

配置 ACK 是校验并持久保存，心跳是联系记录，路径预览是规划；都不能代替真实隧道 / ICMP / HTTP 验证。Apple App 已有浏览器接入与库存显示，隧道未实现。

产品要求见 [产品要求](docs/product-alignment.md)，精简概念见 [产品概览](docs/concepts.md)，当前交付边界见 [状态与剩余工作](docs/milestones.md)。

## 工作区

| 目录 | 职责 |
| --- | --- |
| `apps/controller` / `apps/agent` | Rust 控制面与节点进程 |
| `crates/protocol` / `crates/policy` / `crates/dataplane` | Rust 契约、纯策略编译、认证与加密转发 |
| `crates/agent-dns` | 独立本地 DNS 服务、记录索引与租约 / 代际发送边界 |
| `crates/agent-tun` | Linux TUN 设备、配置、包 I/O 与活设备身份 |
| `crates/agent-underlay` | 用途受限 socket、底层解析与路由资源所有权 |
| `crates/agent-connections` / `crates/agent-dataplane` | 承载连接运行、节点执行计划与包流适配 |
| `crates/agent-diagnostics` | Provider 驱动的本地诊断 IPC |
| `crates/control-client` | Rust 生成 HTTP 客户端，供 Agent / Apple core 使用 |
| `crates/transport-*` | 编译期承载适配 |
| `apps/console` | 用户独立工作流维护的 pure SPA 控制台，后端仅维护必要契约兼容 |
| `apps/apple` | SwiftUI + NetworkExtension、UniFFI Rust bridge |
| `packages/protocol` / `packages/control-client` | 生成 OpenAPI、TypeScript SDK 与运行时校验 |
| `docs` | VitePress 产品、架构、开发与参考文档 |
| `experiments` | 独立隔离验收与性能工具 |
| `references` | 只读上游 checkout，版本见 [研究索引](references/README.md) |

Apple 新增开发暂停，已有接口与消费者检查保留。后续 Linux 联调优先已核对的 Mac Controller `https://meshway-dev-api.jdscript.dev`；构建优先 Mac Linux ARM64 / NYC AMD64，非 privileged/非 host network。

## 开发入口

需要 Rust 1.98.0、Node.js 24、pnpm 11.24.0；统一入口使用 Python 3、tmux、lsof。

```sh
pnpm install --frozen-lockfile
pnpm dev
pnpm dev:status
pnpm dev:update
pnpm dev:stop
```

统一入口为 `scripts/dev.py`，Controller / 文档默认在 8420 / 5173，仅管理当前 checkout 及已初始化 demo。身份和运行数据在忽略的 `.meshway/`。行为与更新边界见 [统一开发入口](docs/dev-services.md)，首次注册与单组件启动见 [本地联调](docs/getting-started.md)。

```sh
pnpm dev:docs
pnpm build:docs
```

## 契约与验证

HTTP API 由 Rust DTO 和实际路由生成，Agent / console 使用生成客户端，console 对外部响应做生成 schema 校验；禁止手工复制 DTO / endpoint。console 以 Authorization 携带 `mws_` 会话，Controller 逐请求 / 事务校验身份与网络权限。

```sh
pnpm protocol:generate  # 修改 Rust 契约后生成
pnpm protocol:check     # 生成物 drift check
pnpm test:protocol
pnpm test:codegen
pnpm check:experiments
```

Rust fmt / Clippy / 行为测试与实验入口见 [开发与验收](docs/development.md)。不保留浏览器自动化套件，console UI 由其工作流手工验证。文档构建保留死链检查；实验结果必须保留负载、资源与加密条件，不能把原生 L4 与完整 L3 隧道差值当作包装增量成本。
