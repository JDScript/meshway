# 总体结构与 Crates

共享依据是**实际消费者和职责**。crate 可以是共享契约、Node 组件或独立算法实现；放在 `crates/` 不等于所有程序都应依赖它。

## 应用与目录

```text
apps/
  meshway-server/           # Rust Server
  meshway-daemon/           # binary: meshwayd
  meshway-cli/              # binary: meshway
  meshway-apple/            # macOS App / Packet Tunnel System Extension
    meshway-apple-bridge/   # UniFFI ABI：Node 生命周期、NE 回调与 IPC 字节流
  meshway-web-console/      # React pure SPA
crates/
  meshway-model/            # 含 wire（严格对象声明）与 transport（协议键）
  meshway-configuration/
  meshway-policy/
  meshway-protocol/         # 含 ipc（本机控制协议）
  meshway-control-client/
  meshway-private-storage/
  meshway-engine/           # 含 keys（包平面密钥）
  meshway-control/
  meshway-node/             # Node 运行协调、IPC 分派与平台 Host 接缝
  meshway-connectivity/
  meshway-stun/
  meshway-underlay/
  meshway-netns/            # SO_NETNS_COOKIE：Linux namespace 的经审计 unsafe 边界
  meshway-dataplane/        # 含 diagnostics（探测调度）
  meshway-tun/
  meshway-dns/
  meshway-logging/
  meshway-transport/        # 含 spec、tcp、udp
  meshway-transport-stream/ # 含 tls、vless、trojan
  meshway-rules/            # MWRS 规则集格式、校验 loader、matcher、转换器、签名目录
tools/
  meshway-test-server/      # 测试专用：进程内 Server、钉钥代理、签名 sync 回复、配置生成器
  meshway-desired-fixture/ # 基准／fixture 的编译期望内容与密钥材料（非生产，不签发）
  meshway-netmap-perf/      # netmap 投影测量
  meshway-rules-cli/        # binary: meshway-rules（规则集转换、检查、目录签名与验证）
  meshway-rules-perf/       # 规则集打开与匹配测量
  meshway-circuit-perf/     # 进程内受保护 circuit CPU 测量（L0）
  meshway-crypto-bench/     # 非生产对照密码原语基准
  meshway-transport-perf/   # 承载 raw／datagram 对照测量（L1）
  meshway-dataplane-perf/   # 隔离 Linux TUN／SecureNode／Manager 测量（L2–L4）
  meshway-packet-fixture/   # 隔离 Linux 受保护包流验收
  meshway-runtime-fixture/  # 实际 daemon／CLI 与 OS socket 验收
packages/control-client/    # 生成 TypeScript SDK 与运行时校验
packages/protocol/          # 生成 OpenAPI
docs/                       # 当前架构与字段参考
scripts/                    # 契约生成、开发服务、验收与性能脚本
mise.toml                   # 工具链版本、workspace 任务与 CI 入口
references/                 # 独立上游参考 checkout
```

`meshway-node` 的 `runtime/` 负责唯一运行协调器、持久意图和状态投影；`runtime/control/` 负责调度与单次 Server 控制流程；`runtime/network/` 装配 Host、Manager、SecureNode、自动需求和安装结果，不另持流表或控制凭据；`ipc/` 对已认证连接做类型化分派。`host/` 定义平台接口，`host/linux/` 包装现有 underlay、系统 DNS 与出口 driver。daemon 只装配参数、日志、信号、Unix socket／UID 认证、systemd 与进程输出。CLI 的 `arguments`、`commands`、`output` 分别负责参数、等待／退出语义和结果展示，通信使用 `meshway_protocol::ipc::Client`。Node 能力范围见 [Node](/v2/node)。

`meshway-node::runtime::network` 还持有 dataplane 的有界 crypto pool：批量包设备／Manager 输入先经唯一 SecureNode 准备，再交给不访问 authority 的工作线程，结果保序交回 owner 提交 replay／端点流，最后按冻结 channel 批次发送或写入包设备。pool、guard 与存储实现在 dataplane；Node 运行时只装配和驱动，L3／L4 性能工具复用同一入口。

## 命名规则

**Rust 包根目录名与 `Cargo.toml` 的 package name 相同，统一使用 `meshway-xxx`。** 例如 `crates/meshway-model/Cargo.toml` 定义 `name = "meshway-model"`。

Rust import 按语言规则使用下划线，如 `meshway_model`。可执行名显式声明：`meshway-daemon` → `meshwayd`，`meshway-cli` → `meshway`；Server 目标名为 `meshway-server`。分类目录 `apps`／`crates`／`tools`、包内 `src`／`tests` 和 Rust 模块按职责命名。非 Rust 应用／包遵循其自身包体系。

同一包名在 workspace 中唯一。规划中的 native 薄入口若成为独立 Cargo 包，将使用独立包名（例如 `meshway-plugin-transport-vless`），不与实现库 `crates/meshway-transport-stream` 共用一个 Cargo 名称。

## 跨端共享库 {#跨端共享的契约库}

| 包名＝目标目录名 | 消费者 | 唯一职责 |
| --- | --- | --- |
| `meshway-model` | Server、Node、各 Rust 契约库 | ID、基础引用、实体与 State 值类型；`transport` 协议键与外层 socket 分类；`wire` 共同对象／判别联合声明（`object!`／`tagged!`／`string_enum!`）、map-only 和重复键检查 |
| `meshway-configuration` | Server、Node | Config／执行类型、基础规范化与字段约束、digest／delta |
| `meshway-policy` | Server、Node 数据面 | Access／routing／topology 语义、主体匹配、纯编译与执行许可判定 |
| `meshway-protocol` | Server、生成客户端；`ipc` 由 Node 运行时、daemon、CLI 使用 | HTTP／SSE 请求响应、认证消息、错误与分页；`ipc` 为本机命令、查询、事件与客户端；引用上面的类型 |
| `meshway-control-client` | Node control、其他 Rust API 消费者 | 生成操作、严格解码和 HTTP／stream 接线 |
| `meshway-transport` | 配置库、Server 目录（`spec`）；connectivity、dataplane、underlay、stream providers | `spec`：协议参数、默认规则、结构校验和能力类型；承载会话、batch／buffer 与背压接口；`tcp`／`udp`：Native TCP／UDP provider |
| `meshway-private-storage` | Server、Node 运行时／control／daemon／underlay | 私有目录／文件、锁与原子发布原语；记录格式归调用者 |
| `meshway-logging` | Server、daemon、CLI、Apple bridge 程序入口 | 统一 tracing subscriber、日志参数与格式；默认 stderr，Apple bridge 经平台 writer 回调送 os_log；见[运行日志](/v2/logging) |
| `meshway-transport-stream` | Server 材料验证（`tls`）、HTTP client 装配、connectivity | `tls`：同一 ring provider、证书／密钥验证、精确叶证书信任、显式系统信任配置与既有流上的 TLS 握手；`vless`／`trojan`：既有流上的 TCP CONNECT 封装 |
| `meshway-rules` | `tools/meshway-rules-cli`；规划中 Server `resources/`、Policy、Node control（R2a，尚未接线） | `MWRS` 规则集格式、确定性 builder、任意字节安全的校验 loader（只读入堆，无 `unsafe`）、域名／IP matcher、来源转换器与报告、签名规则目录；见[规则集引擎设计](/v2/rule-sets-design) |
| `meshway-stun` | Server STUN 服务、Node connectivity | 有界 RFC 8489 Binding 编解码、映射观测与 UDP Service；见[连接获取](/v2/node/connectivity#stun-与-server-协调) |

Server 使用 `policy` 的完整规范化／语义校验入口及编译器；Node 数据面调用它对已发布执行许可的纯判定，无需取得全网配置和秘密后再编译全网。`configuration` 提供规范化阶段的语义 hook，由 `policy` 在同一次处理内注入，保持字段位置、保留依赖和错误优先级；没有 `configuration → policy` 反向依赖或第二套 Access 实现。SQL、账户密码哈希、全局可变容器和平台 I/O 不放进 model。

Server 验证 VLESS 等配置时使用 `meshway_transport::spec`；Server 链接的 transport 库同时含 provider，但不装配 Node 的连接与邻接实现。本机 IPC 不与远端 HTTP 共用 endpoint。

## Node 内部共享的基础接口

| 包名＝目标目录名 | 消费者 | 职责 |
| --- | --- | --- |
| `meshway-engine` | Node 运行时（唯一装配）；connectivity、dataplane、underlay、dns（读取 `Desired`）；control（读取私钥） | 唯一期望状态 owner：`Engine::reconfig(Option<&Netmap>)`、租约计时（`next_expiry`／`expire`）、共享 `LocalCompiler` 的不可变 `Desired`；本身份已登记的包平面密钥 `DataPlaneKeys` 与逐跳证明 transcript（不是授权，不含租约或期限） |

Control 只验证、保存并发布签名 netmap（内容与租约）；`meshway-engine` 是租约的唯一计时者，并按固定顺序把同一 `Desired` 应用到各组件：Host（capture／TUN／负向 DNS／exit 角色）→ 邻接 Manager → SecureNode（流／circuit／exit 投影）→ DNS 查询权限；撤回时顺序相反，每个组件都应用空期望。组件不持有租约、代际、继承物或可转让的输出权限；到期 fail closed 由空期望保证，不靠逐包检查期限。基准与 fixture 的静态内容只位于 `tools/`（`meshway-desired-fixture`），`meshway-node` 装配 Control 与 Engine，`meshwayd` 驱动其公开运行入口。

`meshway-plugin`／`meshway-plugin-host` 是规划中的插件 SDK／装载边界，当前没有对应 crate 或可用动态 ABI，见[插件与能力（规划）](/v2/node/plugins)。

## Node 组件库

这些是 Node 运行组件，Server 不链接其实现：

| 实际 crate | 实现归属与边界 |
| --- | --- |
| `meshway-node` | 供进程宿主复用的唯一运行协调器：持久意图、控制调度、Engine／包平面／DNS 装配、状态与诊断、已认证 IPC 分派；`host::Platform`／`Host` 隔离平台资源与执行器，Linux 实现在 `host/linux/`，实验 Apple Host 在 `host/apple/`，实际范围见 [Apple 设计](/v2/node/apple-design) |
| `meshway-control` | [接入、netmap 验证与发布、回执和报告顺序](/v2/node/control)的唯一生产 owner；不签发执行快照 |
| `meshway-dataplane` | [共同 IP 准入、唯一流表、Noise circuit 与逐跳证明](/v2/node/dataplane)；`diagnostics` 模块为[有界 source 探测调度、单调计时、窗口与结果](/v2/node/diagnostics)，借用同一 SecureNode，不持独立连接或密码状态；期望内容与包平面密钥来自 Engine |
| `meshway-connectivity` | [Manager、stream／Native UDP 邻接、协调 UDP 穿透、签名握手与真实 link 事实](/v2/node/transport#邻接管理)；期望内容与包平面密钥来自 Engine，穿透上下文来自 control |
| `meshway-underlay` | [Linux Host、私有 route／rule journal、capture／sink 与用途受限 socket](/v2/node/dataplane#linux-underlay)；期望内容来自 Engine，穿透类型来自 control |
| `meshway-transport`（`tcp`／`udp`）、`meshway-transport-stream`（`vless`／`trojan`） | 正式承载 provider；既有流／socket 上的协议封装与 I/O |
| `meshway-tun` | `PacketDevice`：MTU、可取消有界批量读、保留已提交前缀的批量写与 `PathLimits`；Linux `Tun` 与 macOS `Utun` 实现此接口，dataplane carrier 依赖接口而非具体设备；macOS 接受已拥有 fd，unsafe 原始 fd 处理只在 Apple 桥接 |
| `meshway-netns` | 读取当前 network namespace 的 cookie（`SO_NETNS_COOKIE`）及唯一的 namespace 记录比较规则（`same`），Underlay 记录与系统 DNS 预约共用；此 Linux 桥接的一次 `getsockopt` 经审计，Apple ABI／fd 边界独立隔离在 `meshway-apple-bridge`，见[生命周期](/v2/node/runtime#生命周期) |
| `meshway-dns` | DNS wire／选择、查询 owner、有界 loopback Service 与 dispatcher，以及系统 DNS 后端／归属 journal；实验 macOS 的 `packet-service` 用 smoltcp 处理 DNS-only UDP／TCP 端点，复用同一 Dispatcher／回答许可；`upstream` 为各 Host 共用的 sealed-route 交换与校验。期望内容来自 Engine |

`meshway-netstack` userspace 运行库尚未实现。

### 跨入口复用

Node 的每项业务规则和可变状态都有唯一实现归属。多个入口通过同一领域接口调用，状态展示读取 owner 的快照／事件。

| 共同逻辑 | 实现归属 | 复用入口 |
| --- | --- | --- |
| 上线、下线与期望状态协调 | `meshway-node` 的运行协调器 | 已认证 IPC 命令、启动恢复、运行目标变化；切换 Meshway 网络尚未实现 |
| 规则语义、递归路段展开、路径编译与执行许可纯判定 | `meshway-policy` | Server 校验／编译／DNS 可见性、Node 包准入 |
| netmap 租约计时与期望内容（`Desired`）、组件应用顺序 | `meshway-engine` | Node 事件循环驱动 `next_expiry`／`expire` |
| netmap 验证、保存、发布与 Server 绑定 | `meshway-control` | Node 运行时装配；基准静态内容限于 `tools/` |
| 业务准入、端到端保护、路径执行与原路径回复 | `meshway-dataplane` | TUN；规划中的 userspace IP 包入口 |
| 获准邻接、握手、重连与真实通道状态 | `meshway-connectivity` | Native TCP／VLESS／Trojan stream、Native UDP 显式端点与协调穿透 |
| 内核 capture／sink、资源归属与用途受限 socket | `meshway-underlay` | Server bootstrap、获准候选、daemon TUN 安装 |
| 单个承载协议的握手、封装与会话实现 | `meshway-transport`／`meshway-transport-stream` 的对应模块 | Rust 调用、协议测试；规划中的插件导出层 |
| 业务域名选择、查询与回答生命周期 | `meshway-dns` | loopback UDP／TCP 共用 OwnerDispatcher，daemon 装配系统 DNS |
| 实际资源、通道、包流和测量事实 | 所属运行组件 | CLI 状态、Node 报告与诊断视图 |

bootstrap 解析与业务 DNS 具有不同启动条件和权限，见 [Linux underlay](/v2/node/dataplane#linux-underlay)。Server 事务鉴权、Node 执行准入和 IPC 本机授权分别检查自己的信任边界；共享类型与验证器的复用不取消这些检查。

## 依赖方向

下图列出各包的普通（生产）依赖，省略统一的 `meshway-` 前缀，并省略 `logging` 基础项；其余依赖（含 `stun`、`tun`、`transport` 等边界依赖）全部列出。

```text
model ──► —
transport ──► model
transport-stream ──► model + transport
configuration ──► model + transport
policy ──► configuration + model + transport
protocol ──► configuration + model
control-client ──► protocol + transport-stream
engine ──► configuration + model + policy

Server ──► configuration + model + policy + private-storage + protocol + stun + transport + transport-stream
control ──► configuration + control-client + engine + model + policy + private-storage + protocol
dataplane ──► configuration + engine + model + policy + protocol + transport + tun
dns ──► configuration + dataplane + engine + model + policy + private-storage + protocol + transport
underlay ──► configuration + control + dns + engine + model + policy + private-storage
             + transport + transport-stream + tun
connectivity ──► configuration + control + dataplane + engine + model + policy + protocol + stun
                 + transport + transport-stream + underlay
node ──► configuration + connectivity + control + control-client + dataplane + dns + engine
         + model + private-storage + protocol + rules + transport + tun + underlay
apple-bridge ──► node + protocol + logging
meshwayd ──► node + configuration + dns + private-storage + protocol
meshway CLI ──► protocol

tools/test-server ──► configuration + control + control-client + model + policy + protocol + Server
tools/desired-fixture ──► configuration + engine + model + policy
tools/circuit-perf ──► dataplane + desired-fixture
tools/transport-perf ──► model + transport + transport-stream
tools/netmap-perf ──► configuration + model + policy + transport
tools/dataplane-perf ──► configuration + connectivity + dataplane + desired-fixture + engine + model
                         + private-storage + transport + tun + underlay
tools/packet-fixture ──► configuration + control + control-client + dataplane + engine + model
                         + policy + protocol + Server + transport + transport-stream + tun
tools/runtime-fixture ──► configuration + control-client + dns + model + packet-fixture
                          + protocol + Server + transport
```

生产依赖不反向依赖 App。node、control、connectivity、dataplane、underlay、dns、engine 的组件测试经 dev-dependency `tools/meshway-test-server` 启动真实 Server（进程内、临时数据目录、随机端口）、完成账号／网络／enrollment，并通过钉钥代理与签名 sync 回复投递自编译 netmap。control-client 的 SDK 测试与 daemon／CLI 生命周期测试仍直接启动 Server；CLI 测试使用 `meshway-node` 的正式入口与认证测试传输，daemon 的进程测试验证真实二进制、Unix socket、信号和进程输出。`tools/` 的 packet／runtime fixture 以普通依赖链接 Server App。underlay／connectivity 保留 Control 的穿透类型依赖，但打开内核 owner、受保护 socket 和静态邻接不要求创建 Control 或联系 Server。

provider 通过合同交互，不读取另一 provider 的私有容器；Rust 类型依赖不表示允许跨动态库直接传递 Rust 内部对象。

TLS 密码后端统一为 `meshway_transport_stream::tls` 显式选择的 ring。workspace reqwest 关闭默认 features，启用 `rustls-no-provider`；HTTP 装配传入预配置的 rustls `ClientConfig`，不安装或依赖进程全局 provider。`meshway_control_client::http_client_builder`、Server 下载和 Underlay 受保护 HTTP builder 使用同一可选 `platform-verifier` 配置，保留 OS 系统信任策略；managed TLS 及 fixture 的精确叶证书路径复用 `meshway_transport_stream::tls::Client::configuration`，每次新握手检查链、域名、有效期及精确叶证书。预配置模式下不能再用 reqwest 的附加根证书选项改变信任，应在 TLS 装配时明确选择配置。套接字保护、resolver、代理和整体请求截止仍归原调用者。

## 工程约定 {#旧实现退出与复用}

- Rust 使用 Tokio；Server HTTP 使用 Axum，数据库访问见[存储](/v2/storage)。
- 协议实现位于领域 crate；`apps/` 装配用例，测试使用生产入口。新增抽象须有明确职责或实际消费者，状态 owner 不因增加一层适配而复制。
- 引入组件须明确职责、依赖、协议／生命周期约束和行为回归。历史代码只作为经审查的复用材料，不成为当前合同或目录的约束。
- native/FFI 桥接单独审查，业务实现保持安全边界；共享协议不混入 loader 或系统调用。
- 源码组织、测试布局与构建缓存等仓库约定见根目录 `AGENTS.md`。
