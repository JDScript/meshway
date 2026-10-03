# 技术栈与代码结构

**V2 结构提案。** 复用现有 Rust 控制面／Agent 与 React pure SPA。新增 shared crates 按可独立构建、测试的职责划分；下列目标布局不是已完成的源码迁移。

## 技术栈

| 层 | 推荐 | 现状与理由 |
| --- | --- | --- |
| Controller | Rust、Tokio、Axum | 已使用；同一进程装配 Console／Agent 两个 router |
| 持久存储 | SQLite、rusqlite、显式 SQL 事务 | 已使用；V2 增加完整版本与独立 State，不必为此换 ORM／数据库 |
| State／订阅 | Rust 内存结构、有界 channel、Axum SSE | 内存观测已有部分；统一 State／SSE 属于新增能力 |
| 契约 | Serde＋Utoipa、OpenAPI、生成 Rust／TS SDK | 已有生成链；流式与密钥认证支持需扩展验证 |
| 用户认证 | Argon2id＋不透明 session | 已使用；新开放注册／建网按已确认模型推进 |
| Node 认证 | 持久密钥身份、成熟持钥认证机制 | 新设计；具体握手选型独立定版 |
| Agent | Rust、Tokio、现有领域 crates、Linux TUN | 保留已拆分能力和实际数据面边界 |
| Console | React、TypeScript、Vite、TanStack Query／Router、Base UI／Tailwind | 沿用独立 Console 工作流；fetch SSE、生成 schema 校验、身份隔离缓存 |
| Apple | SwiftUI、NetworkExtension、UniFFI Rust core | 保留已有接口与消费者，新隧道开发仍暂停 |
| 文档 | VitePress | 当前实现和 V2 设计分区 |

具体版本由 workspace manifest／lockfile 固定。本提案不增加 Redis、外部消息队列或时序数据库作为首版运行依赖；State 和有界历史的压力先实测。

## Workspace 布局

```text
apps/
  controller/                 # HTTP、身份／权限、持久事务、编译发布、State 聚合
  agent/                      # 接入、控制同步、CLI／IPC 与领域运行编排
  console/                    # pure SPA，独立工作流
  apple/                      # SwiftUI + 平台桥接 Rust core

crates/
  model/                      # 提案：共享 ID、公开实体语义、State 类型
  configuration/              # 提案：源／节点配置、纯校验、默认解析、digest／delta
  transport-spec/             # 提案：承载声明、配置 schema／校验、能力目录
  protocol/                   # HTTP 请求／响应与事件 envelope
  policy/                     # access／topology／routing 纯编译器
  control-client/             # 生成 Rust 客户端与传输接线
  private-storage/            # 提案：私有文件与原子持久化原语
  dataplane/                  # 认证／加密／转发唯一实现
  agent-dns/
  agent-tun/
  agent-underlay/
  agent-connections/
  agent-dataplane/
  agent-diagnostics/
  transport-*/                # 各协议运行实现

packages/
  protocol/                   # 从 Rust 生成的 OpenAPI
  control-client/             # 生成 TS SDK、Zod、编辑器 schema

docs/
experiments/
scripts/
references/
```

crate 不与数据库表一一对应。Controller 的账户、存储和发布流程可以先作为 App 内职责模块；Agent 的已有独立运行库继续拥有实现。共享含义是存在实际跨组件消费者，不是所有类型都塞入一个 common／utils 包。

## Shared crates

| Crate | 拥有什么 | 消费者／边界 |
| --- | --- | --- |
| `model`（提案） | NetworkId／UserId／NodeId 等强类型 ID，公开 Entity／Membership 语义，NodeState／观测类型 | Controller、Agent 和契约使用；不含密码哈希、SQL row、HTTP handler 或全局可变 State |
| `configuration`（提案） | NetworkConfiguration／NodeConfiguration、纯结构／语义校验、默认值解析、稳定摘要、full／delta 处理 | Controller 生成、Agent 校验安装；输入给出身份／材料事实，库自身不查库或生成并落盘秘密 |
| `transport-spec`（提案） | 稳定协议 key、Listen／Dial／TLS／WS 声明、默认参数规则、组合校验、声明式能力目录 | 配置、Controller 目录和运行适配共用；不创建 socket，不自动启用 Agent 未编译的协议 |
| `policy`（已有，调整依赖） | access／topology／routing 的求值、候选完整路径和出口编译、遮蔽诊断 | 纯输入→输出；复用 configuration 类型，不能成为第二套配置源 |
| `protocol`（已有，收窄） | Console／Agent HTTP DTO、认证消息、分页／错误、SSE envelope | 包装共享类型；不持有 DB、业务编译器、私有文件实现 |
| `control-client`（已有） | 从实际路由生成的操作、认证／stream 传输接入与严格解码 | Agent、Apple core 直接消费；不依赖 Agent App |
| `private-storage`（提案） | 私有目录／文件校验、锁、原子写入和持久性结果 | Controller／Linux Agent；文件路径与身份 journal 格式仍由应用拥有，Apple Keychain 由平台实现 |

建议将目前 `protocol::local` 的文件 I/O 移至 private-storage；其随机凭据生成与摘要则由实际身份／认证模块拥有，不能把一个混合模块原样改名搬走。`model` 的 State 类型不意味着共享 Controller 内存状态管理实现。

节点持钥认证若需要 Controller、Agent、Apple 共用密码学实现，可在握手选型后独立 `node-identity` crate，严格限定密钥／证明原语；HTTP、会话存储、Keychain 和文件布局仍分别由消费者接线，不提前复制一套自制握手。

### 依赖方向

以下箭头表示左侧依赖右侧：

```text
transport-spec → model（仅需要的基础类型）
configuration → model + transport-spec
policy → configuration + model
protocol → configuration + model
control-client → 生成操作／公开契约类型 + HTTP transport

controller → 上述纯模型／编译／契约 + 自有存储与服务
agent → control-client + 配置／模型 + agent-* 运行库
agent-dataplane → 节点执行类型 + dataplane
agent-connections → 承载配置 + transport-* + dataplane
```

model 保持基础层：State 中的配置关联只使用版本／digest／资源引用，不嵌入 configuration 或 transport-spec 的完整对象。configuration 不依赖 protocol／policy；policy 产出 configuration 定义的执行类型，避免“配置需要编译器类型、编译器又依赖配置”的循环。dataplane 通过窄运行接口获得数据，不为取几个 ID／执行字段反向依赖整个 HTTP 契约。

共享 Rust 类型是权威定义。生成客户端优先通过生成器类型映射引用现有公开类型；必须生成的 wire 类型和转换也由生成链维护、严格校验，不手抄一套同名结构。TypeScript 使用生成类型和运行时 schema；WASM 不是 HTTP 通信或 Console 首屏前置。

## Controller 内部

```text
src/
  main.rs                     # 进程入口
  lib.rs                      # 装配与受控公开入口
  app.rs                      # 应用依赖／生命周期；目标替代含义宽泛的 AppState 容器
  settings.rs                 # 实例部署配置
  http/
    mod.rs
    console/                  # /api/v1
    agent/                    # /agent-api/v1
    static_files.rs           # Console dist 与精确 SPA fallback
  accounts/                   # 注册、登录、会话和成员权限
  enrollment/                 # 设备批准／接入、密钥绑定与幂等恢复
  configuration/              # 编辑提交、JSONC、材料准备、CAS
  execution/                  # 需求、编译、依赖发布、实例与租约
  state/                      # 当前报告、聚合、过期、可见视图和订阅
  observations/               # 诊断任务、采集与历史查询
  resources/                  # 规则集／GeoIP／材料的获取和版本准备
  store/                      # SQLite rows、事务与有界 DB 执行入口
    schema/                   # 当前结构与完整性约束
  maintenance/                # 本地运维操作，按实际需要保留
```

这是职责布局，不要求为每项立刻建空目录。模块有子模块时使用 `foo/mod.rs`，否则 `foo.rs`；实现相关单元测试放所属模块，公共 API 黑盒测试放 crate 顶层 tests，生成代码仍由权威生成器维护。

### 一条配置保存链

```text
HTTP DTO 校验
  → accounts：身份／网络权限
  → configuration：编辑语义、解析引用和准备材料
  → policy：纯编译
  → store：重新鉴权／CAS，提交配置与发布
  → execution / state：切换当前目标，通知 Agent／Console
```

数据库约束与最终权限重检由事务保证。服务协调跨记录的用例，Store 封装 SQL 和事务原语；不形成每层都重新实现一次校验／编译的重复服务。资料下载、KDF、编译和 SSE 写入不能持有数据库锁。

API 两个入口复用同一配置与 State 服务，但分别构造 Console／Agent 视图。静态 fallback 必须识别两种 API 根路径，不能把缺失 Agent endpoint 返回为 HTML。

## Agent 运行边界

| 位置 | 职责 |
| --- | --- |
| Agent App | 接入与私有身份、控制同步、CLI／IPC 投影、组件编排和日志 |
| agent-connections | 候选／承载执行、认证邻接、连接生命周期；不拥有 TUN／上层 PacketPlan |
| agent-dataplane | 安装节点执行配置、准入与包流调度，适配 dataplane |
| agent-underlay | socket 保护、底层解析、capture／sink、NAT／路由恢复 owner |
| agent-tun／agent-dns | 平台 TUN 设备 I/O／DNS 服务的独立生命周期 |
| agent-diagnostics | 通过窄 provider 获取真实状态／执行获准诊断 |
| dataplane／transport-* | 唯一加密转发核心／编译期具体协议实现 |

HTTP SSE 接收不等于执行：先还原完整配置、校验并私有持久保存，再 ACK 和安装；资源失败如实上报。执行 lease／撤回独立停止旧代际，SSE 保活不能续期。

## 需要从现有结构收敛的地方

- 当前 protocol 同时包含 HTTP DTO、执行字段、凭据与文件原语；V2 以共享模型、configuration 和私有存储职责拆清。
- 当前 policy 依赖 protocol；目标是依赖纯 configuration／model，HTTP 外层反向包装共享类型。
- 当前 Store 同时有 SQL Connection 和 discovery 内存 map；目标为持久事务与 State 运行存储各自拥有生命周期。
- 当前 Agent 领域库已经拆分，沿其实现继续；统一 transport-spec／registry、SSE 与 Node 密钥认证仍是新增工作。

独立能力迁移应删除旧实现和无必要的长期整模块重导出，形成可验证提交。根 workspace／lock 与生成客户端由整合工作统一维护；当前页面仅确定审阅目标，不直接搬迁源码。
