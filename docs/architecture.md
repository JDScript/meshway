# 架构与实现边界

Meshway 是 Rust controller / Agent、Next.js 控制台与终端 App 组成的身份驱动网络。已确认产品模型见[概览与概念](/concepts)：以 Tailscale 为基线，扩展自定义 transport 和有序指定路径。

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

VLESS TCP/TLS 与 WebSocket/TLS 的 listener / 凭据已由平台生成和下发，旧 profiles 保留等价导入路径；手工连接候选仍待自动组网替代。Access 与 Routing 的正式下发仍是旧版；独立 v3 四层引擎和 Linux 出口 / 状态型回包原型已合入，正式策略 / 出口接线尚待完成。迁移保留身份、凭据、地址及策略行为，不能等价转换的规则须报告。可信域名上下文、IPv6 TUN、自动打洞和真实逐跳 trace 未完成。

受保护 socket、HTTP/TLS 与 bootstrap DNS 基础已经合入，原生 Runtime 的统一注入和正式 RPDB 生命周期仍待接通；不能据模块存在就宣称现网已启用全 IP 系统保护。当前优先 Linux 自动组网 / 打洞及跨地域 Docker 验收，账户与控制台并行，Apple 暂停新增开发，见[里程碑](/milestones)。

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

当前 module / crate 的具体耦合、应保留的隔离以及增量收敛顺序见[代码架构审查](/architecture-review)。该审查是对当前实现的分析，建议的 publication / control / session 等模块尚未整体落地。

| 目录 | 职责 |
| --- | --- |
| `crates/protocol` | Rust HTTP / 配置契约、校验、摘要与私有存储原语 |
| `crates/policy` | 纯策略规范化、引用校验与逻辑编译，无网络 I/O |
| `crates/dataplane` | Noise、认证邻接、E2E 与转发核心 |
| `crates/transport-*` | 编译期承载适配，保留 stream / datagram 与 buffer 所有权语义 |
| `apps/controller` | 实际 API 路由、权限、SQLite、快照与配置编译 |
| `apps/agent` | 生成 Rust client、身份、同步、运行层、Linux TUN / DNS 与诊断 |
| `packages/control-client` | 生成 TS SDK、运行时与编辑器 schema |
| `apps/web` | 真实 API 驱动的控制台，读取与每个管理 mutation 均做服务端身份检查 |
| `apps/apple` | SwiftUI / NetworkExtension、UniFFI Rust bridge，当前接入与库存显示已实现，隧道未实现 |

HTTP 契约由 Rust DTO 与实际路由生成，禁止手写平行 DTO / endpoint。WASM 仅是未来共享策略逻辑的可选方案。生成路径与限制见[协议参考](/protocol)。

## 部署与继续阅读

当前 SQLite 数据目录由独占锁保护，不提供多 controller 复制或 HA；Unix 私有目录 / 文件保护本地凭据，非本地 HTTP 凭据通道由部署层提供 TLS。开发数据库采用事务增量升级，特定升级证据不等于长期混合版本兼容保证。

- [注册与持久化](/enrollment)、[浏览器接入](/device-login)：身份、事务恢复与撤销。
- [地址与配置分发](/configuration)、[连接配置](/connections)：revision、CAS、摘要与保存回执。
- [连接运行层](/connection-runtime)、[托管数据面](/managed-dataplane)：应用、lease 与实际流量。
- [DNS](/dns)、[transport](/transports)、[加密与追踪](/security-and-tracing)：各自实现边界。
- [开发与验收](/development)：测试入口与可复现证据。
