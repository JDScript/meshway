# 总体结构与 Crates

共享依据是**实际消费者和职责**。crate 可以是共享契约、Node 组件或独立算法实现；放在 `crates/` 不等于所有程序都应依赖它。

## 应用与目录

当前源码结构：

```text
apps/
  meshway-server/           # Rust Server
  meshway-daemon/           # binary: meshwayd
  meshway-cli/              # binary: meshway
  meshway-web-console/     # React pure SPA
crates/
  meshway-model/
  meshway-transport-spec/
  meshway-configuration/
  meshway-policy/
  meshway-protocol/
  meshway-control-client/
  meshway-serde/
  meshway-private-storage/
  meshway-authority/
  meshway-control/
  meshway-connectivity/
  meshway-stun/
  meshway-underlay/
  meshway-dataplane/
  meshway-diagnostics/
  meshway-tun/
  meshway-dns/
  meshway-ipc/
  meshway-logging/
  meshway-transport/
  meshway-transport-tcp/
  meshway-transport-udp/
  meshway-tls/
  meshway-transport-vless/
  meshway-transport-trojan/
tools/meshway-transport-perf/ # 独立运行的性能测量程序
tools/meshway-packet-fixture/ # 独立运行的隔离 Linux 包流 conformance
tools/meshway-runtime-fixture/ # 实际 daemon／CLI 与 OS socket conformance
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

Node 的应用目录为 `apps/meshway-daemon`／`apps/meshway-cli`，提供控制接入与 Linux Mesh-only TUN 运行装配。候选实现和验收范围见 [Node](/v2/node)。

daemon 的 `runtime/` 负责唯一运行协调器、持久意图和状态投影；`runtime/control/` 负责调度与单次 Server 控制流程；`runtime/network/` 装配 underlay、Manager、SecureNode、自动需求和安装结果，不另持流表或控制凭据；`ipc/` 负责本机服务和 socket 归属。CLI 的 `arguments`、`commands`、`output` 分别负责参数、等待／退出语义和结果展示，通信使用 `meshway-ipc::Client`。

## 跨端共享库 {#跨端共享的契约库}

这些库是 Server 和 Node 的共同基础：

| 包名＝目标目录名 | 消费者 | 唯一职责 |
| --- | --- | --- |
| `meshway-model` | Server、Node；按需由 IPC 使用 | ID、基础引用、实体与 State 值类型 |
| `meshway-transport-spec` | 配置库、Server 目录、transport 实现 | 协议名称、参数、默认规则、结构校验和能力类型 |
| `meshway-configuration` | Server、Node | Config／执行类型、基础规范化与字段约束、digest／delta |
| `meshway-policy` | Server、Node 数据面 | Access／routing／topology 语义、主体匹配、纯编译与执行许可判定 |
| `meshway-protocol` | Server、生成客户端 | HTTP／SSE 请求响应、认证消息、错误与分页；引用上面的类型 |
| `meshway-control-client` | Node control、其他 Rust API 消费者 | 生成操作、严格解码和 HTTP／stream 接线 |
| `meshway-serde` | 各 Rust 契约库 | 共同对象／判别联合声明、map-only 和重复键检查 |
| `meshway-private-storage` | Server、Node control／daemon／underlay | 私有目录／文件、锁与原子发布原语；记录格式归调用者 |
| `meshway-logging` | Server、daemon、CLI 程序入口 | 统一 tracing subscriber、日志参数与 stderr 格式；见[运行日志](/v2/logging) |
| `meshway-tls` | Server 材料验证、Node TLS providers | 同一证书／密钥验证、精确叶证书信任、既有流上的 TLS 握手 |
| `meshway-stun` | Server STUN 服务、Node connectivity | 有界 RFC 8489 Binding 编解码、映射观测与 UDP Service；见[连接获取](/v2/node/connectivity#stun-与-server-协调) |

Server 使用 `policy` 的完整规范化／语义校验入口及编译器；Node 数据面调用它对已发布执行许可的纯判定。Node 无需取得全网配置和秘密后再编译全网。`configuration` 提供规范化阶段的语义 hook，由 `policy` 在同一次处理内注入，保持字段位置、保留依赖和错误优先级；没有 `configuration → policy` 反向依赖或第二套 Access 实现。SQL、账户密码哈希、全局可变容器和平台 I/O 不放进 model。

## Node 内部共享的基础接口

以下接口服务 Node 的实际消费者：

| 包名＝目标目录名 | 消费者 | 职责 |
| --- | --- | --- |
| `meshway-ipc` | daemon、CLI | 本机命令、查询、事件与客户端 |
| `meshway-authority` | control（签发）、daemon、dataplane、connectivity、underlay、dns、diagnostics、packet-fixture（消费） | `ExecutionSnapshot`、`DataPlaneIdentity`、UTC 高水位 `Clock` 与三重 `Deadline`；只定义绑定与检查，不读存储、不联系 Server |
| `meshway-transport` | connectivity、dataplane、transport providers | 承载会话、batch／buffer 与背压接口 |

Server 不因需要验证 VLESS 配置而链接 Node 的 transport 实现；它使用 `transport-spec`。本机 IPC 也不与远端 HTTP 共用一套 endpoint。

`meshway-authority` 的构造入口 `ExecutionSnapshot::issue`／`DataPlaneIdentity::new` 是公开的，但**生产中唯一的签发者是 `meshway-control`**：它把当前 lease、持久私有目标、共享 validity／invalidation 和三重 deadline 绑进快照。`crates/` 内不得出现第二个签发实现；分层基准与 fixture 的静态签发只能位于 `tools/`，`meshwayd` 只装配 Control。读取或克隆快照不延长任何期限；该约束由类型保持，签发来源由装配纪律保持。

`meshway-plugin`／`meshway-plugin-host` 是规划中的插件 SDK／装载边界，当前没有对应实现 crate 或可用动态 ABI。

## Node 组件库

这些是 Node 运行组件，不要求 Server 链接其实现：

| 实际 crate | 实现归属与边界 |
| --- | --- |
| `meshway-control` | [接入、执行快照、连续性、回执和报告顺序](/v2/node/control)的唯一生产 owner；使用 authority 类型签发 |
| `meshway-dataplane` | [共同 IP 准入、唯一流表、Noise circuit 与逐跳证明](/v2/node/dataplane)；直接消费 authority |
| `meshway-diagnostics` | [有界 source 探测调度、单调计时、窗口与结果](/v2/node/diagnostics)；直接消费 authority，借用同一 SecureNode，不持独立连接或密码状态 |
| `meshway-connectivity` | [Manager、stream／Native UDP 邻接、协调 UDP 穿透、签名握手与真实 link 事实](/v2/node/transport#邻接管理)；身份／快照来自 authority，穿透 scope 仍来自 control |
| `meshway-underlay` | [Linux Host、私有 route／rule journal、capture／sink 与用途受限 socket](/v2/node/dataplane#linux-underlay)；直接消费 authority，穿透类型来自 control；`Host::open(directory, options)` 打开内核 owner／受保护工厂，`attach_control(origin)` 可选挂接 bootstrap resolver／固定 origin 代理 |
| `meshway-transport-{udp,tcp,vless,trojan}` | 正式承载 provider；既有流／socket 上的协议封装与 I/O |
| `meshway-tun` | Linux TUN 设备 I/O |
| `meshway-dns` | DNS wire／选择、查询 owner、有界 loopback Service 与 dispatcher，以及系统 DNS 后端／归属 journal；直接消费 authority |

daemon 已装配 Mesh-only stream／Native UDP 数据面、系统 DNS 与获准诊断；Linux 验收范围见[运行模式](/v2/node/runtime)。出口转发与 `meshway-netstack` 运行库仍待实现。

组件可独立测试，并可被其他宿主复用。native 导出入口只适配共同 ABI，不复制业务、配置或状态模型。私有文件操作确有多个实际消费者时才抽窄存储原语，不为每张表、每个任务、每种状态各建库。

### 跨入口复用

Node 的每项业务规则和可变状态都有唯一实现归属。多个入口通过同一领域接口调用，状态展示读取 owner 的快照／事件。

| 共同逻辑 | 实现归属 | 复用入口 |
| --- | --- | --- |
| 上线、下线、网络切换与期望状态协调 | daemon 的运行协调器 | CLI 命令、启动恢复、运行目标变化 |
| 规则语义、递归路段展开、路径编译与执行许可纯判定 | `meshway-policy` | Server 校验／编译／DNS 可见性、Node 包准入 |
| 执行快照、lease 时钟与失效代际的类型与检查 | `meshway-authority` | 数据面、DNS 及宿主 I/O 等待／交付检查 |
| 执行快照的生产签发、连续性交接与 Server 绑定 | `meshway-control` | daemon 装配；基准静态签发限于 `tools/` |
| 业务准入、端到端保护、路径执行与原路径回复 | `meshway-dataplane` | TUN 和 userspace IP 包入口 |
| 获准邻接、握手、重连与真实通道状态 | `meshway-connectivity` | Native TCP／VLESS／Trojan stream、Native UDP 显式端点与协调穿透；连接池待实现 |
| 内核 capture／sink、资源归属与用途受限 socket | `meshway-underlay` | Server bootstrap、获准候选、daemon TUN 安装 |
| 单个承载协议的握手、封装与会话实现 | 对应 `meshway-transport-*` | Rust 调用／协议测试与动态插件导出层 |
| 业务域名选择、查询与回答生命周期 | `meshway-dns` | loopback UDP／TCP 共用 OwnerDispatcher，daemon 装配系统 DNS；SOCKS5 接线与缓存待实现 |
| 实际资源、通道、包流和测量事实 | 所属运行组件 | CLI 状态、Node 报告与诊断视图 |

bootstrap 解析与业务 DNS 具有不同启动条件和权限，当前实现见 [Linux underlay](/v2/node/dataplane#linux-underlay)，规划中的插件接口见 [host services](/v2/node/plugins#数据面与-host-services)。Server 事务鉴权、Node 执行准入和 IPC 本机授权分别检查自己的信任边界；共享类型与验证器的复用不取消这些检查。

协议实现位于领域 crate；`apps/` 装配用例，`plugins/` 提供 ABI 薄适配，测试使用生产入口。新增抽象须有明确职责或实际消费者，状态 owner 不因增加一层适配而复制。临时接线在正式入口完成时移除。

## 依赖方向

```text
以下简写省略统一的 meshway- 前缀：

transport-spec ──► serde
model ──► serde + transport-spec（测量中的实际承载事实）
configuration ──► model + transport-spec
policy ──► configuration + model + transport-spec
protocol ──► model + configuration
control-client ──► protocol

Server ──► protocol + configuration + policy + private-storage + 自有领域／存储模块
authority ──► configuration + policy（邻接许可）+ model + protocol（instance 引用）
Node control ──► authority（签发）+ control-client + 共享配置／模型 + private-storage
Node dataplane ──► policy + authority（执行快照）
Node diagnostics ──► dataplane + authority（执行快照）+ protocol/model（任务与测量）
Node connectivity ──► dataplane（受保护帧 I/O）+ authority + control（穿透 scope）+ policy + underlay + stun + transport providers
Node underlay ──► authority + control（穿透类型／origin 校验）+ configuration + private-storage + dns（resolver 接口）+ tun + transport/tcp/udp + transport-spec
Node DNS ──► policy + configuration + authority（执行快照）+ protocol（发现响应）+ private-storage
Node providers ──► transport + transport-spec + tls（按协议需要）
meshwayd ──► control + authority + connectivity + underlay + dataplane + diagnostics + dns + ipc
meshway CLI ──► ipc
```

库不反向依赖 App。provider 通过合同交互，不读取另一 provider 的私有容器；Rust 类型依赖不表示允许跨动态库直接传递 Rust 内部对象。

dataplane、DNS 和 diagnostics 的真实生命周期测试通过 dev-dependency 使用 Control／Server；生产消费接口直接依赖 authority。underlay／connectivity 保留 Control 的穿透类型依赖，但打开内核 owner、受保护 socket 和静态邻接不要求创建 Control 或联系 Server。

## 复用边界 {#旧实现退出与复用}

新 Node 依据本分区实现。引入组件须明确职责、依赖、协议／生命周期约束和行为回归。历史代码可作为经审查的复用材料，但不成为当前合同或目录的约束；运行验证须使用当前 Server 和正式节点生命周期。

## 工程约定

- Rust 使用 Tokio；Server HTTP 使用 Axum，数据库访问见[存储](/v2/storage)。
- 单文件模块 `foo.rs`，有子模块则 `foo/mod.rs`；私有测试随模块，黑盒测试遵守 Cargo 标准入口。
- native/FFI 桥接单独审查，业务实现保持安全边界；共享协议不混入 loader 或系统调用。
- 根 manifest／lock、生成链与联合验收集中维护。完成任务的构建缓存有边界，保留源码与必要证据。
