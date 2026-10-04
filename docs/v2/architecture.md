# 总体结构与 Crates

共享依据是**实际消费者和职责**。crate 可以是共享契约、Node 组件或独立算法实现；放在 `crates/` 不等于所有程序都应依赖它。

## 应用与目录

当前源码结构：

```text
apps/
  meshway-server/           # Rust Server
  meshway-web-console/     # React pure SPA
crates/
  meshway-model/
  meshway-transport-spec/
  meshway-configuration/
  meshway-protocol/
  meshway-control-client/
  meshway-serde/
packages/control-client/    # 生成 TypeScript SDK 与运行时校验
packages/protocol/          # 生成 OpenAPI
docs/                       # 当前架构与字段参考
scripts/protocol/           # 契约与客户端生成／检查
scripts/dev.py              # 持久开发服务生命周期
mise.toml                   # 工具链版本、workspace 任务与 CI 入口
references/                 # 独立上游参考 checkout
```

## 命名规则

**Rust 包根目录名与 `Cargo.toml` 的 package name 相同，统一使用 `meshway-xxx`。** 例如 `crates/meshway-model/Cargo.toml` 定义 `name = "meshway-model"`。

Rust import 按语言规则使用下划线，如 `meshway_model`。可执行名显式声明：`meshway-daemon` → `meshwayd`，`meshway-cli` → `meshway`；Server 目标名为 `meshway-server`。分类目录 `apps`／`crates`、包内 `src`／`tests` 和 Rust 模块仍按职责命名。非 Rust 应用／包遵循其自身包体系。

同一包名在 workspace 中唯一。native 薄入口若是独立 Cargo 包，也使用相同目录／包名，例如 `plugins/meshway-plugin-transport-vless`；它不与实现库 `crates/meshway-transport-vless` 共用一个 Cargo 名称。

新 Node 的应用目录为 `apps/meshway-daemon`／`apps/meshway-cli`，接口库和 provider 目录按同名规则建立；目前尚未实现，不在当前构建成员中。

## 跨端共享的契约库

这些契约库是 Server 和 Node 的共同基础：

| 包名＝目标目录名 | 消费者 | 唯一职责 |
| --- | --- | --- |
| `meshway-model` | Server、Node；按需由 IPC 使用 | ID、基础引用、实体与 State 值类型 |
| `meshway-transport-spec` | 配置库、Server 目录、transport 实现 | 协议名称、参数、默认规则、结构校验和能力类型 |
| `meshway-configuration` | Server、Node | Config／执行类型、规范化、校验、digest／delta；纯编译在内部模块 |
| `meshway-protocol` | Server、生成客户端 | HTTP／SSE 请求响应、认证消息、错误与分页；引用上面的类型 |
| `meshway-control-client` | Node control、其他 Rust API 消费者 | 生成操作、严格解码和 HTTP／stream 接线 |
| `meshway-serde` | 各 Rust 契约库 | 共同对象／判别联合声明、map-only 和重复键检查 |

`configuration` 中较重的编译模块可用 feature 限定消费者；Node 仍需要执行输入校验。SQL、账户密码哈希、全局可变容器和平台 I/O 不放进 model。HTTP handler 不成为 DTO 的依赖。

## Node 内部共享的基础接口

以下为新 Node 提案，不是跨整个项目的通用服务层：

| 包名＝目标目录名 | 消费者 | 职责 |
| --- | --- | --- |
| `meshway-ipc` | daemon、CLI | 本机命令、查询、事件与客户端 |
| `meshway-plugin` | 宿主、provider | 插件 SDK、描述与生命周期合同 |
| `meshway-plugin-host` | daemon | 装载、ABI／依赖检查及受审 native loader |
| `meshway-transport` | connectivity、dataplane、transport providers | 承载会话、batch／buffer 与背压接口 |

Server 不因需要验证 VLESS 配置而链接 Node 的 transport 实现；它使用 `transport-spec`。本机 IPC 也不与远端 HTTP 共用一套 endpoint。

## Node 组件库

`meshway-control`、`meshway-dataplane`、`meshway-connectivity`、`meshway-underlay`、`meshway-transport-{udp,tcp,vless,trojan}`、`meshway-tun`、`meshway-netstack`、`meshway-dns`、`meshway-diagnostics` 按[节点职责](/v2/node)实现。这些是节点运行能力，不是必须供 Server 使用的 shared crates。

组件可独立测试，并可被其他宿主复用。native 导出入口只适配共同 ABI，不复制业务、配置或状态模型。私有文件操作确有多个实际消费者时才抽窄存储原语，不为每张表、每个任务、每种状态各建库。

### 跨入口复用

Node 的每项业务规则和可变状态都有唯一实现归属。多个入口通过同一领域接口调用，状态展示读取 owner 的快照／事件。

| 共同逻辑 | 实现归属 | 复用入口 |
| --- | --- | --- |
| 上线、下线、网络切换与期望状态协调 | daemon 的运行协调器 | CLI 命令、启动恢复、运行目标变化 |
| 业务准入、端到端保护、路径执行与原路径回复 | `meshway-dataplane` | TUN 和 userspace IP 包入口 |
| 下一跳选择范围内的拨号竞速、重试、主备与打洞协调 | `meshway-connectivity` | 所有承载 provider |
| 单个承载协议的握手、封装与会话实现 | 对应 `meshway-transport-*` | Rust 调用／协议测试与动态插件导出层 |
| 业务域名解析与缓存 | `meshway-dns` | 系统侧 DNS 入口与本机 SOCKS5 的域名请求 |
| 实际资源、通道、包流和测量事实 | 所属运行组件 | CLI 状态、Node 报告与诊断视图 |

bootstrap 解析与业务 DNS 具有不同启动条件和权限，按 [Node 底层接口](/v2/node/plugins#数据面与-host-services)区分。Server 事务鉴权、Node 执行准入和 IPC 本机授权分别检查自己的信任边界；共享类型与验证器的复用不取消这些检查。

协议实现位于领域 crate；`apps/` 装配用例，`plugins/` 提供 ABI 薄适配，测试使用生产入口。新增抽象须有明确职责或实际消费者，状态 owner 不因增加一层适配而复制。临时接线在正式入口完成时移除。

## 依赖方向

```text
以下简写省略统一的 meshway- 前缀：

transport-spec
model ──► transport-spec
configuration ──► model + transport-spec
protocol ──► model + configuration
control-client ──► protocol

Server ──► protocol + configuration（编译）+ 自有领域／存储模块
Node control ──► control-client + 共享配置／模型
Node providers ──► plugin + 领域接口 + 所需共享类型
meshway CLI ──► ipc
```

库不反向依赖 App。provider 通过合同交互，不读取另一 provider 的私有容器；Rust 类型依赖不表示允许跨动态库直接传递 Rust 内部对象。

## 复用边界 {#旧实现退出与复用}

新 Node 依据本分区实现。引入组件须明确职责、依赖、协议／生命周期约束和行为回归。历史代码可作为经审查的复用材料，但不成为当前合同或目录的约束；运行验证须使用当前 Server 和正式节点生命周期。

## 工程约定

- Rust 使用 Tokio；Server HTTP 使用 Axum，数据库访问见[存储](/v2/storage)。
- 单文件模块 `foo.rs`，有子模块则 `foo/mod.rs`；私有测试随模块，黑盒测试遵守 Cargo 标准入口。
- native/FFI 桥接单独审查，业务实现保持安全边界；共享协议不混入 loader 或系统调用。
- 根 manifest／lock、生成链与联合验收集中维护。完成任务的构建缓存有边界，保留源码与必要证据。
