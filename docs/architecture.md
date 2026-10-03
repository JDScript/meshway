# 架构与实现边界

Meshway 是 Rust controller / Agent、Web 控制台与终端 App 组成的身份驱动网络。控制台是用户独立工作流维护的 pure SPA `apps/console`。已确认产品模型见[概览与概念](/concepts)：以 Tailscale 为基线，扩展自定义 transport 和有序指定路径。

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

当前是多网络、单实例 SQLite，已接本地用户账户、网络成员权限和设备归属；用户 session、实例 bootstrap、注册令牌、节点 bearer 分权。管理查询在根事务内校验身份 / 网络权限并显式携带网络 ID；Agent 由凭据推导网络，SQL 绑定网络列，不使用连接级环境作用域。App 已有用户浏览器批准 / PKCE、隔离多接入和库存显示，暂无隧道。托管 Agent 支持公钥绑定、认证邻接、路径绑定的 Noise E2E、Linux Mesh IPv4 TUN、节点 DNS 与私有 daemon 诊断。

VLESS TCP/TLS 与 WebSocket/TLS listener/材料由平台管理。四层 NetworkPolicy 是唯一源，Controller 同事务 publication 投影稳定 NodeMeshExecution；已支持静态授权路径需求的自动邻接及 TCP/UDP/ICMP echo 状态型回包。完整出口、域名上下文、IPv6 overlay、首包按需建连、打洞与真实 trace 仍未完成。

Linux 正常接入、控制同步与 TUN 生命周期已接受保护 socket、HTTP/TLS、bootstrap DNS 和 RPDB owner；同地址普通业务不获全局豁免，退出保留 Mesh sink。隔离包流与恢复验证不代表远端已部署或所有宿主/VPN均受支持。Agent 领域库已独立并统一消费当前执行合同，后续推进自动组网 / 打洞及跨地域 Docker 验收；Apple 新增开发暂停，见[里程碑](/milestones)。

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

保持source、compiler internal、稳定node execution DTO和Agent本机执行四个边界。Controller共享同事务事实，Agent保留防御校验。当前schema一次初始化，普通启动严格拒绝未知/损坏状态，版本不扩散到业务兼容分支。

| 目录 | 职责 |
| --- | --- |
| `crates/protocol` | Rust HTTP / 配置契约、校验、摘要与私有存储原语 |
| `crates/policy` | 纯策略规范化、引用校验与逻辑编译，无网络 I/O |
| `crates/dataplane` | Noise、认证邻接、E2E 与转发核心 |
| `crates/agent-dns` | 本地权威 DNS、Zone 与租约 / 代际发送边界 |
| `crates/agent-tun` | Linux TUN 设备创建、配置、I/O 与真实设备身份，不拥有业务路由 |
| `crates/agent-underlay` | Socket 用途/代际保护、底层解析、capture/sink/journal 所有权 |
| `crates/agent-connections` | 连接与承载执行、内存材料与运行观测，不拥有 TUN/HTTP/私有文件 |
| `crates/agent-dataplane` | 节点执行计划/admission 与底层包流适配，不复制加密/转发 |
| `crates/agent-diagnostics` | 本地 IPC 和查询协议，经 provider 获取真实运行状态/探测 |
| `crates/control-client` | Rust 生成 HTTP 客户端与 checked transport 注入 |
| `crates/transport-*` | 编译期承载适配，保留 stream / datagram 与 buffer 所有权语义 |
| `apps/controller` | 实际 API 路由、权限、SQLite、快照与配置编译 |
| `apps/agent` | 身份、控制同步、领域库运行编排与诊断 |
| `packages/control-client` | 生成 TS SDK、运行时与编辑器 schema |
| `apps/console` | 独立工作流维护的 pure SPA 控制台，以 Authorization 携带 `mws_` 会话；Controller 逐请求 / 事务检查身份与网络权限 |
| `apps/apple` | SwiftUI / NetworkExtension、UniFFI Rust bridge，当前接入与库存显示已实现，隧道未实现 |

HTTP 契约由 Rust DTO 与实际路由生成，禁止手写平行 DTO / endpoint。WASM 仅是未来共享策略逻辑的可选方案。生成路径与限制见[协议参考](/protocol)。

Controller 内部由 `http/` 装配路由，`contracts/` 保存不依赖 handler 的契约，`accounts/` 管账户领域；
`store/` 管事务与持久化，`store/schema/` 拥有当前结构的原子初始化与完整性校验，`store/test_support/` 只用于测试。
策略源 / CAS 位于 `store/network_policy/`，节点执行租约与回执位于 `store/mesh_execution.rs`；
`network_scope.rs` 提供受限的网络存在性类型，不代替成员权限检查。`lib.rs` 保留组合与公开入口。

## 部署与继续阅读

当前 SQLite 数据目录由独占锁保护，不提供多 controller 复制或 HA；Unix 私有目录 / 文件保护本地凭据，非本地 HTTP 凭据通道由部署层提供 TLS。开发数据库仅使用当前 schema，切换时由部署者显式重置旧数据；普通启动拒绝旧版本、结构损坏和缺失归属，不自动修复或清库。

- [注册与持久化](/enrollment)、[浏览器接入](/device-login)：身份、事务恢复与撤销。
- [地址与配置分发](/configuration)、[连接配置](/connections)：revision、CAS、摘要与保存回执。
- [连接运行层](/connection-runtime)、[托管数据面](/managed-dataplane)：应用、lease 与实际流量。
- [DNS](/dns)、[transport](/transports)、[加密与追踪](/security-and-tracing)：各自实现边界。
- [开发与验收](/development)：测试入口与可复现证据。
