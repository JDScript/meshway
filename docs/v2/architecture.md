# 总体结构与 Crates

共享依据是**实际消费者和职责**。crate 可以是共享契约、Node 组件或独立算法实现；放在 `crates/` 不等于所有程序都应依赖它。

## 应用与目录

建议统一为下列目标结构，**本轮仅整理设计，不搬迁或删除源码**：

```text
apps/
  meshway-server/           # 目标 Server 包；当前实现仍为 meshway-controller
  meshway-daemon/           # binary: meshwayd
  meshway-cli/              # binary: meshway
  console/                  # React pure SPA
  apple/                    # 原生应用；新增开发暂停
crates/
  meshway-model/
  meshway-transport-spec/
  meshway-configuration/
  meshway-protocol/
  meshway-control-client/
  meshway-serde/
  meshway-ipc/
  meshway-plugin/
  meshway-plugin-host/
  meshway-transport/
  ...                       # Node 组件同样使用完整包名
plugins/                    # native 导出／打包薄入口
packages/control-client/    # 生成 TypeScript SDK 与运行时校验
packages/protocol/          # 生成 OpenAPI
docs/                       # 当前架构与字段参考
experiments/                # 隔离实验与验收工具
scripts/                    # 生成、构建、开发与发布
references/                 # 独立上游参考 checkout
```

## 命名规则

**Rust 包根目录名与 `Cargo.toml` 的 package name 相同，统一使用 `meshway-xxx`。** 例如 `crates/meshway-model/Cargo.toml` 定义 `name = "meshway-model"`，不再同时维护目录短名和包名两套对应关系。

Rust import 按语言规则使用下划线，如 `meshway_model`。可执行名显式声明：`meshway-daemon` → `meshwayd`，`meshway-cli` → `meshway`；Server 目标名为 `meshway-server`。分类目录 `apps`／`crates`、包内 `src`／`tests` 和 Rust 模块仍按职责命名。非 Rust 应用／包遵循其自身包体系。

同一包名在 workspace 中唯一。native 薄入口若是独立 Cargo 包，也使用相同目录／包名，例如 `plugins/meshway-plugin-transport-vless`；它不与实现库 `crates/meshway-transport-vless` 共用一个 Cargo 名称。

本页展示目标名称。现行 Server／共享库仍在原目录，实际迁移时统一调整 workspace、path 依赖、生成／构建脚本和引用，不留旧目录重导出兼容层。

## 跨端共享的契约库

这些契约库已存在，是新实现的共同基础；以下列包名，也即迁移后的目录名：

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

## 旧实现退出与复用

合并前，**旧 Agent App 和未进入复用白名单的节点运行库整体退出活跃源码树**；不另留一个长期 `legacy/` 副本。旧实现、NAT／出口成果、失败反例和验收证据保留在明确的 Git 分支／提交及私有证据位置。

新 Node 依据本分区设计。复用代码须说明职责、依赖和仍适用的回归；可以重新实现，也可以按白名单取回，不默认整包继承旧接口或状态机。源码清理、main 合并和运行服务切换分别执行，当前讨论没有触发这些操作。

## 工程约定

- Rust 使用 Tokio；Server HTTP 使用 Axum，数据库访问见[存储](/v2/storage)。
- 单文件模块 `foo.rs`，有子模块则 `foo/mod.rs`；私有测试随模块，黑盒测试遵守 Cargo 标准入口。
- native/FFI 桥接单独审查，业务实现保持安全边界；共享协议不混入 loader 或系统调用。
- 根 manifest／lock、生成链与联合验收集中维护。完成任务的构建缓存有边界，保留源码与必要证据。
