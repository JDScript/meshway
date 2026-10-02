# 架构与实现边界

Meshway 是 Rust controller / Agent、Web 控制台与终端 App 组成的身份驱动网络。既有 Next.js Web 可用于联调，新 pure SPA console 由用户独立工作流维护。已确认产品模型见[概览与概念](/concepts)：以 Tailscale 为基线，扩展自定义 transport 和有序指定路径。

## 控制面与数据面

```text
Web 管理 / 浏览器接入
        │ 生成 TypeScript SDK + 运行时 schema
        ▼
Controller：身份、网络、IPAM、策略编译、配置版本
        │ 生成 Rust 客户端
        ▼
Agent：校验 / 私有缓存 / ACK → 应用 → 认证邻接 → TUN / DNS
        │
源节点 ═══ E2E 密文，经必经节点转发 ═══ 目标节点 / 出口
```

Controller 维护与编译控制意图，不默认中继业务流量。Agent 执行连接管理、授权路径、加密转发和诊断。出口解密后的外部流量由出口节点处理，Mesh E2E 的边界终止于出口，而非任意网站。

### 已确认目标

- 多网络隔离：成员、地址、DNS、四层策略和相关配置按网络作用域管理，一台设备一次只接入一张。
- 本地邮箱账户、仅邀请、设备归属用户与 tag 接入密钥；人类登录、管理权限、节点凭据、公钥身份分别建模。
- 平台托管 listener、承载、DNS、出口与所有业务配置；按已授权流量按需自动建连。
- 连接图、grants、Mesh 内有序路由、出口路由方案分别求值，联合编译可执行且不绕过命中规则必经节点的路径。

### 当前实现

当前是多网络、单实例 SQLite。v11 已接本地用户账户、网络成员权限和设备归属；用户 session、实例 bootstrap、注册令牌、节点 bearer 分权。管理查询在根事务内校验身份 / 网络权限并显式携带网络 ID；Agent 由凭据推导网络，SQL 绑定网络列，不使用连接级环境作用域。App 已有用户浏览器批准 / PKCE、隔离多接入和库存显示，暂无隧道。托管 Agent 支持公钥绑定、认证邻接、路径绑定的 Noise E2E、Linux Mesh IPv4 TUN、节点 DNS 与私有 daemon 诊断。

VLESS TCP/TLS 与 WebSocket/TLS 的 listener/凭据已由平台生成和下发，旧 profiles 保留等价导入；手工候选仍待自动组网替代。IPv4 Mesh v3 已接通同事务 publication、生成执行 DTO与正常 Agent/TUN，旧策略源保留且须显式 CAS 激活新模式。Linux 出口/状态回包原型已验，但完整出口及 Mesh 状态型回包尚未正式接线。不能等价迁移的策略报告诊断，保留身份/数据；可信域名上下文、IPv6 TUN、打洞与逐跳 trace 未完成。

受保护 socket、HTTP/TLS、bootstrap DNS、原生 Runtime owning factory 和 Manager prepared owner 接点已合入；正式主循环激活与 RPDB 生命周期仍待接通，默认仍 unmarked，不能据模块存在就宣称现网已启用全 IP 系统保护。当前优先 Linux 自动组网 / 打洞及跨地域 Docker 验收，进一步前端和 Apple 开发暂缓，见[里程碑](/milestones)。

## 证据与生命周期

| 状态 | 能证明什么 |
| --- | --- |
| 注册 / 心跳 | 控制面身份有效、最近联系时间 |
| 公钥绑定 | controller 对该身份绑定了不可任意替换的公钥 |
| 配置 ACK | Agent 校验并私有持久保存了该 revision / digest |
| 运行报告 | Agent 自报告资源已安装、近期认证邻接探测等状态 |
| TUN 代际与实际 ICMP / HTTP | 指定快照的数据面运行及本次真实访问 |

配置、连接、内部授权和端点公钥必须来自同 revision 的一致快照。撤回和 lease 到期通过独立 watchdog 终止旧运行代际，不等待阻塞 HTTP 完成。TUN 停止后 Mesh 网段不得落回底层默认路由。

身份和观测分开：名称、Mesh IP、国家元数据、STUN endpoint 都不能证明节点身份。能力声明不表示 listener 已开启；节点位置不表示业务授权。拓扑分别展示配置意图、认证邻接报告和规划路径，不能虚构全连通图。

## Rust 与前端边界

保持 source、compiler internal、稳定 node execution DTO 和 Agent 本机执行四个边界。Controller publication 共享同事务事实/编译/投影，Agent 保留防御性校验。领域模块按职责命名；历史 schema/wire 版本集中于迁移/兼容，测试支持与历史 fixture 有明确归属。完整 dump/审查不等于源码组织清理完成。

| 目录 | 职责 |
| --- | --- |
| `crates/protocol` | Rust HTTP / 配置契约、校验、摘要与私有存储原语 |
| `crates/policy` | 纯策略规范化、引用校验与逻辑编译，无网络 I/O |
| `crates/dataplane` | Noise、认证邻接、E2E 与转发核心 |
| `crates/agent-dns` | 本地权威 DNS、Zone 与租约 / 代际发送边界 |
| `crates/transport-*` | 编译期承载适配，保留 stream / datagram 与 buffer 所有权语义 |
| `apps/controller` | 实际 API 路由、权限、SQLite、快照与配置编译 |
| `apps/agent` | 生成 Rust client、身份、同步、运行层、Linux TUN / DNS 与诊断 |
| `packages/control-client` | 生成 TS SDK、运行时与编辑器 schema |
| `apps/web` | 真实 API 驱动的控制台，读取与每个管理 mutation 均做服务端身份检查 |
| `apps/apple` | SwiftUI / NetworkExtension、UniFFI Rust bridge，当前接入与库存显示已实现，隧道未实现 |

HTTP 契约由 Rust DTO 与实际路由生成，禁止手写平行 DTO / endpoint。WASM 仅是未来共享策略逻辑的可选方案。生成路径与限制见[协议参考](/protocol)。

Controller 内部由 `http/` 装配路由，`contracts/` 保存不依赖 handler 的契约，`accounts/` 管账户领域；
`store/` 管事务与持久化，`store/migrations/` 统一拥有历史升级和结构校验，`store/test_support/` 只用于测试。
策略源 / CAS 位于 `store/network_policy/`，节点执行租约与回执位于 `store/mesh_execution.rs`；
`network_scope.rs` 提供受限的网络存在性类型，不代替成员权限检查。`lib.rs` 保留组合与公开入口。

## 部署与继续阅读

当前 SQLite 数据目录由独占锁保护，不提供多 controller 复制或 HA；Unix 私有目录 / 文件保护本地凭据，非本地 HTTP 凭据通道由部署层提供 TLS。开发数据库采用事务增量升级，特定升级证据不等于长期混合版本兼容保证。

- [注册与持久化](/enrollment)、[浏览器接入](/device-login)：身份、事务恢复与撤销。
- [地址与配置分发](/configuration)、[连接配置](/connections)：revision、CAS、摘要与保存回执。
- [连接运行层](/connection-runtime)、[托管数据面](/managed-dataplane)：应用、lease 与实际流量。
- [DNS](/dns)、[transport](/transports)、[加密与追踪](/security-and-tracing)：各自实现边界。
- [开发与验收](/development)：测试入口与可复现证据。
