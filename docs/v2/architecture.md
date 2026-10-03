# 技术栈与 Crates

**V2 结构提案。** Cargo 包名统一 `meshway-*`；磁盘目录沿用短名，Rust import 使用下划线，例如 `meshway-configuration` → `meshway_configuration`。现有 workspace 已采用这种命名方式。

## 共享库

核心共享模型收敛为三个库：

| Cargo 包名 | 目录 | 职责 |
| --- | --- | --- |
| `meshway-transport-spec` | `crates/transport-spec` | 协议 key、Listen／Dial／TLS／WS 字段、默认参数规则、组合校验、能力声明 |
| `meshway-model` | `crates/model` | 强类型 ID、公开实体值类型、State／观测类型和基础引用 |
| `meshway-configuration` | `crates/configuration` | NetworkConfiguration／NodeConfiguration、Schema、规范化、校验、编译、digest／delta |

**编译与配置放在同一 crate，按模块分责。** 下一版把现有 `meshway-policy` 的纯编译能力迁入 meshway-configuration，不保留一套平行配置模型或长期重导出。此项仍是工程提案，当前 policy 源码未迁移。

其他已有边界按用途保留：

| Cargo 包名 | 目录 | 职责 |
| --- | --- | --- |
| `meshway-protocol` | `crates/protocol` | HTTP 请求／响应、错误、分页、认证消息和 SSE envelope；引用共享类型 |
| `meshway-control-client` | `crates/control-client` | 生成 Rust API 操作、严格解码与 HTTP／stream 接线 |
| `meshway-private-storage` | `crates/private-storage` | 提案：私有文件、锁、原子写入和持久性结果 |
| `meshway-dataplane` | `crates/dataplane` | 认证邻接、端到端加密和转发核心 |
| `meshway-agent-*` | `crates/agent-*` | 已拆分的 DNS／TUN／underlay／connections／dataplane／diagnostics 运行能力 |
| `meshway-transport-*` | `crates/transport-*` | 各协议运行实现，与纯 transport-spec 分开 |

SQL row、密码哈希和 Controller 内存容器留在 Controller 内部。meshway-model 共享的是类型，不包含数据库访问或可变全局状态。Node.identity／enabled 类型定义在 meshway-configuration，Node 数据库记录不会成为第二份授权源。

## 依赖方向

箭头表示左侧依赖右侧：

```text
meshway-transport-spec                 # 纯协议声明，不依赖 Meshway 业务对象
meshway-model → meshway-transport-spec # State 使用能力／协议类型；定义基础 ID
meshway-configuration → meshway-model + meshway-transport-spec
meshway-protocol → meshway-model + meshway-configuration
meshway-control-client → 生成操作／公开契约类型 + HTTP transport

Controller → protocol + configuration（编译）+ model + 自有 store
Agent → control-client + configuration（校验／delta）+ agent-* 运行库
Apple core → control-client + 所需共享类型／平台桥接
```

model 中的 State 只保存配置 revision／digest／发布引用，不嵌入完整 Configuration。transport-spec 不引用 NodeId／UserId，避免和 model 形成循环。

可以将较重的编译模块放入 meshway-configuration 的 `compiler` feature：Controller 启用，Agent 只需完整类型、输入防御校验和 delta。feature 不得删掉 Agent 执行前的权限／材料一致性检查。

## meshway-configuration 内部

```text
src/
  lib.rs
  network.rs                 # NetworkConfiguration 与 NodeSettings／NodeIdentity
  node.rs                    # 完整 NodeConfiguration 执行类型
  normalize.rs               # 确定的默认值／格式规范化
  validate/                  # 配置、引用、执行输入校验
  compile/                   # access、topology、routing、路径／出口／DNS 投影
  delta.rs                   # full／增量还原
  digest.rs                  # 确定编码／摘要
```

共享 Schema 从类型与约束生成。地址池与分配来自完整 Configuration，编译输入另传可信实体、公钥和规则集依据；编译器不查库、不访问 HTTP、不随机生成并落盘秘密。Controller 负责地址分配／重分配、材料准备、权限和事务，Agent 负责本机资源检查与安装。

生成 Rust 客户端优先通过生成器类型映射复用公开类型；必要 wire 类型／转换由生成链维护。TypeScript 使用生成类型和运行时 schema，业务组件不复制 DTO。WASM 是将来复用纯逻辑的选项，不是通信前置。

## Workspace

```text
apps/
  controller/                # API、权限、存储、编译发布、State 聚合
  agent/                     # 接入、控制同步、CLI／IPC 与运行编排
  console/                   # React pure SPA，独立工作流
  apple/                     # SwiftUI + Rust core 平台桥接
crates/                      # 上述 meshway-* Rust 包
packages/
  protocol/                  # 从 Rust 导出的 OpenAPI
  control-client/            # 生成 TS SDK、Zod、编辑器 schema
docs/
experiments/
scripts/
references/
```

crate 不按数据库表拆分；存在独立实现、测试和实际消费者的能力再成库。协议库不混入文件 I/O，private-storage 只接收原子文件原语；当前 protocol::local 中的凭据生成／摘要回到认证用途模块。

## Controller

```text
src/
  main.rs / lib.rs           # 入口与装配
  app.rs                     # 应用依赖与生命周期
  settings.rs                # 部署配置
  http/
    console/                 # /api/v1
    agent/                   # /agent-api/v1
    static_files.rs          # Console dist；识别两个 API 根路径
  accounts/                  # 注册、登录、会话与成员权限
  enrollment/                # 设备批准、接入／换钥与幂等恢复
  configuration/             # JSONC 编辑、identity 转换权限、材料准备、CAS
  execution/                 # 需求、一致快照发布、节点目标／实例／lease
  state/                     # RAM 报告、派生视图、过期与订阅
  observations/              # 测量任务、历史接纳／查询
  resources/                 # 规则集与 GeoIP 下载／规范化；凭据生成在配置准备内
  store/                     # 私有 SQL rows、事务与 DB 执行入口
    schema/                  # 当前结构与完整性约束
```

业务服务协调一次用例，Store 保证事务、约束和最终权限重检；不会在 handler、service、store 各自再编译一遍配置。协议／数据模型见[数据库字段](/v2/storage/database)和[内存字段](/v2/storage/memory)。

发布状态沿用 ExecutionPublication；节点回执持久保存、实时步骤进入 State／SSE。各 Node 同步后正常应用，安装与失败清理由各运行资源 owner 负责，UI 只提交目标并展示结果。

嵌套的配置凭据、listener 代际、发布内节点切片、任务内订阅都由所属模块维护。模型类型、缓存条目、后台 Job 不各建一套 repository／service／crate；新增独立库以真实复用和可独立测试的能力为依据。

## Agent

| 位置 | 职责 |
| --- | --- |
| App | 接入、私有身份、控制同步、CLI／IPC 投影、组件编排 |
| agent-connections | 获准候选／承载执行、认证邻接、连接生命周期 |
| agent-dataplane | 安装执行配置、准入与包流调度，适配 dataplane |
| agent-underlay | 保护 socket、底层解析、capture／sink、NAT／路由恢复 owner |
| agent-tun／agent-dns | TUN 设备 I/O／DNS 服务生命周期 |
| agent-diagnostics | 窄 provider 上的真实状态／获准诊断 |

底层加密／转发由 dataplane 唯一实现。Connections 不拥有 TUN／上层 PacketPlan；HTTP、Keychain／文件格式留在应用。Node 持钥认证方案定版后，确有跨端密码学复用再抽 `meshway-node-identity`，不提前发明通用 auth 大包。

## 技术栈

| 层 | 推荐 |
| --- | --- |
| Controller／Agent | Rust、Tokio；Controller 使用 Axum |
| 存储 | SQLite、rusqlite、显式事务；最新 State 使用内存与有界 channel |
| 契约 | Serde／Utoipa、OpenAPI、生成 Rust／TS SDK；SSE 生成待扩展 |
| 认证 | 用户 Argon2id＋不透明会话；Node 持久密钥身份，握手待定版 |
| Console | React、TypeScript、Vite、TanStack Query／Router、Base UI／Tailwind |
| Apple | SwiftUI、NetworkExtension、UniFFI；新增隧道开发仍暂停 |
| 文档 | VitePress |

具体版本由 manifest／lock 固定。新库和迁移尚未实施；现有 Agent 领域实现与回归继续复用。私有实现测试在所属模块，公共 API 黑盒测试按 Cargo 自动发现组织；源码迁移时删除旧实现，独立形成可验证提交。
