# 技术栈与 Crates

**已确认的 V2 实现结构。** 新 Controller 位于 `apps/meshway-controller`，Cargo 包名仍为 `meshway-controller`。共享库的 Cargo 包名统一 `meshway-*`，磁盘目录沿用短名，Rust import 使用下划线，例如 `meshway-configuration` → `meshway_configuration`。

Controller、共享模型、配置编译与 HTTP／事件合同从本分区重新实现；客户端从新合同生成。已有 dataplane、transport 与 Agent 运行能力继续复用，随后接入新合同。包边界已确认，精确接口按下面的契约 checkpoint 落实，不把文档示例当作已经完整实现的载荷。

## 共享库

核心共享模型收敛为三个库：

| Cargo 包名 | 目录 | 职责 |
| --- | --- | --- |
| `meshway-transport-spec` | `crates/transport-spec` | 协议 key、Listen／Dial／TLS／WS 字段、默认参数规则、组合校验、能力声明 |
| `meshway-model` | `crates/model` | 强类型 ID、公开实体值类型、State／观测类型和基础引用 |
| `meshway-configuration` | `crates/configuration` | NetworkConfiguration／NodeConfiguration、Schema、规范化、校验、编译、digest／delta |

**编译与配置放在同一 crate，按模块分责。** V2 的纯编译实现直接属于 meshway-configuration，最终替代现有 meshway-policy 的对应职责。新实现以 V2 语义和独立行为验收为依据，不以搬运旧编译器作为任务；不建立平行配置模型或长期重导出。

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

model 中的 State 只保存配置 revision／digest／发布引用，不嵌入完整 Configuration。公共引用、应用阶段和共享问题值不能因为文档页面归属而反向放入 configuration／protocol；HTTP 错误上下文仍归 protocol。transport-spec 不引用 NodeId／UserId，避免和 model 形成循环。

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
  meshway-controller/        # 新 API、权限、存储、编译发布、State 聚合
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
  authorization/             # 当前权限与可见范围的共同规则
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

业务服务协调一次用例，Store 保证事务、约束和最终权限重检；不会在 handler、service、store 各自再编译一遍配置。authorization 接收明确身份／成员／配置上下文，不回调其他业务服务；事务重检复用相同规则。协议／数据模型见[数据库字段](/v2/storage/database)和[内存字段](/v2/storage/memory)。

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

具体版本由 manifest／lock 固定。私有实现测试在所属模块，公共 API 黑盒测试按 Cargo 自动发现组织；单文件模块用 foo.rs，有子模块时用 foo/mod.rs，不为测试扩大生产可见性。

## 实现隔离与集成

已确认采用 `.worktree/controller-rewrite` 的替换式开发；并行实现 worktree 同样位于本仓库 `.worktree/` 下。新基线选择新 Controller 和新合同，当前主工作区继续服务现有消费者。

- 旧 apps/controller 已占用 meshway-controller 包名，新开发基线不能同时选择两个同名应用。由协调者维护开发 workspace 的成员和依赖替换，不新增 controller-v2／policy-v2 双轨包。
- fresh 编码任务只读取 V2 文档、共同契约和新代码；旧实现的消费者核对、已有回归接线由独立整合任务负责。独立验证仍覆盖鉴权、网络隔离、CAS、秘密、撤回、lease 和同会话原路径回复。
- 根 manifest／lockfile、共享合同、数据库执行器与 schema 总入口、路由装配和生成链由协调者维护。领域任务只修改分配的模块及私有存储子模块。
- 模型／声明可最先并行；共同契约确定后，配置库、账户／接入、配置提交／执行、State／观测可按目录分派。紧密相关的配置提交／执行先由同一工作包负责。
- 每个活跃任务使用指定、有总量边界的 Cargo target。不同 feature 验收不并发共用目录，结束任务回收专用缓存。

开发基线的检查范围明确列出，不将仅新 Controller 的构建称为已有 Agent／Apple 消费者兼容通过。整合前完成必要消费者适配、生成 drift 和真实 Linux 包流验收；现有服务切换是独立部署操作。

## 公共契约 checkpoint

开始实现不必等待全部功能参数定版，但各任务不能独立发明以下接口：

| Checkpoint | 必须统一的内容 |
| --- | --- |
| 基础类型与生成 | ID／时间／计数编码，类型所有权，完整配置／非秘密视图／预览计划，Schema／SSE 生成声明 |
| 配置提交 | 确定性编辑与待准备事项、真实材料／地址准备、固定输入及事务重检、提交结果与取消后的完成责任 |
| Node 认证与执行 | 持钥证明、密钥代际与撤销、完整编译输入、并发需求合成、完整节点目标及替代／lease 边界 |
| 事件与读取 | DB 提交与 RAM 变化的有序入口、snapshot 截点、权限收缩、统一 cursor／reset 与 GET／SSE 片段 |

新 demand 的业务目标／协议端口、方案选择、可信能力和 DNS 地址关联等实际编译输入须进入相应依赖与摘要。Node 的完整目标要保留其他仍有效需求；不能用最后一次 demand 的切片覆盖整个节点，也不能仅凭相同源 revision 判断发布相容。

首个纵向交付使用真实 SQLite、用户会话和网络隔离，贯通配置持久化／CAS 与 SSE；随后以正式 Node 认证接入配置／State，再连接已有 Linux 运行库。编译 fixture、saved／applied 回执与实际 TUN 连通分别验收。

### 基础编码

新实现首先固定以下值类型；它们不会自动改变当前运行 API：

- 持久资源 ID 与请求 ID 使用独立 Rust newtype，JSON 为小写、带连字符的非零 UUID 字符串。model 负责解析／表示，生成和认证绑定由使用方负责；ID 不充当凭据。
- Revision／记录 Version 使用正整数，Sequence 可为零；JSON number 最大为 `9007199254740991`，构造、解码和递增均检查范围，避免 TypeScript 精度丢失。不把任意 u64 都默认为安全 JSON number，流量大计数在其合同中另定无损编码。
- SHA-256 摘要使用独立的 32-byte 值类型和 64 位小写十六进制字符串；秘密摘要留在私有存储，不因类型可序列化而对外公开。完整配置摘要还须固定规范编码与用途分隔。
- 时间统一以 UTC RFC 3339 表达。每个公共值类型的 Serde 解码约束和导出 Schema 必须一致；schema 不替代服务端语义校验。

首批 transport-spec 先固定协议 key、外层 socket／stream 类型与显式能力值。roles 缺省、协议字段／默认参数尚待定版的部分不自动填充，目录中的声明也不代表新 Controller 或 Agent 已经托管该协议。
