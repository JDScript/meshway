# 承载扩展架构审查

状态：已完成源码静态审查；下述统一 registry 和能力协商是推荐方案，尚未实现。
审查固定于账户与 P2a 组合候选 `e4d51b5`，并只读核对 underlay `58323a9`、Web 表单
`5dc2f24` 的接口，不以正在修改的工作树作为稳定契约。本次架构审查没有重跑行为测试。
既有 WS 取消边界、错误脱敏和能力投影问题继续独立修复。

## 已核对的重复定义

| 位置 | 当前事实 |
| --- | --- |
| `crates/protocol/src/connections.rs` | `ConnectionTransport` 固定三种协议；能力数组的 schema 和校验都固定最多三个；语义按协议手动匹配。 |
| `crates/dataplane/src/config.rs` | 单独定义 `TransportKind`，也枚举 Native UDP / TCP / VLESS。 |
| `crates/dataplane/src/runtime/{mod,statistics,wire}.rs` | 统计有固定三槽/三个字段；Noise prologue 中的 transport wire ID 明确为 1/2/3，重构必须保持稳定。 |
| `apps/agent/src/state/connections.rs` | 编译能力由手写列表构造；P2a 调整了 feature 边界，但仍需统一注册来源。 |
| `apps/web/lib/connections-model.ts` | 可选协议、名称、提示和是否需要 profile 分别手写。新增托管表单也需要具体协议表单适配。 |
| `crates/protocol/src/lib.rs`、`apps/controller/src/store.rs` | `agent_version` 已随注册/心跳存储和展示，不等同于完整的配置 schema / wire 协商。 |
| `docs/transports.md` 与各 transport crate | Trojan / SOCKS5 / Shadowsocks 有库级实现，尚未形成统一的控制面托管与 Agent 装配。 |
| `scripts/generate-protocol.mjs` | 当前只处理特定 nullable union 与普通 enum，尚不支持通用判别联合；强类型插件配置升级前必须补生成器支持。 |

P2a 独立审查还确认：WS 子能力曾仅影响管理摘要，没有约束有效执行投影。
该缺陷正在修复，也说明目录、能力检查与执行编译应复用同一组定义，避免各处各写判断。
闭合枚举本身提供类型安全；整理目标是收敛重复维护，不是把执行配置改成任意字符串和 JSON。

以现有 Trojan crate 为扩展样本，沿当前 VLESS 特判模式接入至少涉及 **16 个已有手写文件**，
包括两套枚举、能力上限、材料生成/投影、双侧验证、Agent 装配、wire ID、统计和 Web 选项；
这还不含测试、生成产物和迁移。因此当前尚未达到“一处注册 + 对应实现”的目标。

## 建议的职责与数据流

```text
共享、强类型的编译期 transport 注册声明
  ├─ controller：目录、配置 schema/默认值、校验、材料策略与兼容规则
  └─ agent：实际编译实现与 dial/accept factory

agent ── 本构建版本、配置/wire 兼容范围、实际能力 ──→ controller
web   ←─ controller 目录 + 所选节点/节点对的可用结果 ─ controller
agent ←─ 同 revision 的已验证配置、允许候选与必要材料 ─ controller
agent ── 已保存 ACK、执行状态、连接与失败观测 ──────→ controller
```

“控制面支持”“Agent 编译支持”“网络策略允许”“本机当前具备执行条件”“已配置监听”
和“实际已认证连接”分别表达。目录提供的可选项不能替代 Agent 能力声明，能力声明也不等于
端口已绑定、入口可达或业务已获授权。对一条邻接，拨号端和接收端都必须支持所选组合。

### 配置校验

- controller 是管理配置的权威：字段、协议组合、网络作用域、权限、两端能力、兼容版本、
  配额和最终下发预算均在保存/编译阶段检查。默认参数与凭据继续由平台生成和下发。
- 不依赖节点运行环境的规则尽量由共享的强类型定义/校验函数提供，避免 controller、Agent
  和 Web 各维护一份业务规则。Web 使用生成 schema 做即时提示，服务端仍须独立验证。
- Agent 在执行前继续校验本构建是否支持、schema、身份/作用域、revision/digest/lease、
  证书和材料绑定；端口占用、系统权限和 socket 保护等实际条件只能在本机验证。
  本地拒绝需要回报明确原因，不能将保存 ACK 当成执行成功。

### 注册边界

建议把协议描述与运行实现分开：控制面只依赖所需的配置定义、校验和材料管理，
不为展示目录引入 TUN 或整套 transport 运行时。Agent 注册对应实现，复用统一的
受保护 socket/resolver、buffer/batch、背压和取消合同。

新增协议仍需要实现插件、注册并编译发布相应组件；不承诺旧二进制只拉一个目录就能运行新协议。
VLESS 的 TCP/TLS/WS 是有约束的组合，不将协议、字节流封装和 TLS 当作可任意交叉拼接的选项。
建议优先在现有 `crates/protocol/src/transport/` 收拢纯定义/校验，controller 与 Agent
各自拥有材料 handler 和运行 factory；具体 transport crate 继续负责协议实现。

第一可合入切片可采用明确枚举 + 静态描述表 + 穷举分派，先保持现有三协议合同与行为。
完成目标采用一个小型声明式注册清单/`macro_rules!`，在各 crate 内生成薄分派代码，
使新协议的稳定 ID、配置/材料类型和实现入口有一个注册来源。宏不承载异步业务逻辑，
也不引入动态加载 ABI、全局扫描或独立的一串 registry crate。

同一 catalogue 中，controller 可管理子集与各 Agent feature 构建的 factory 子集可不同；
HTTP schema 的权威输出不能随着某个 Agent 的 Cargo feature 改变而发生漂移。

### 版本与能力

建议分别协商软件/build 信息、配置 schema 版本、wire 兼容性及具体能力/限制。
版本可用于已知兼容门槛和诊断，但不能只按版本号猜测 feature、拨号/监听资格或 WS 支持。
能力更新应与当前节点运行实例关联；失去能力时保留管理意图、撤回不再可执行的投影，
并触发既有 revision/lease 失效机制。不能为可达性切到不允许的承载或绕过必经节点。

混合版本下，控制面需按接收端理解的版本编译投影；未知执行类型明确拒绝，
同时设计目录的兼容表达，避免新增协议导致旧客户端连节点清单都无法读取。
未知库存项可以用独立、受限、不可执行的摘要表达；不能使用 `Unknown(JSON)` 转发执行配置。
已有 v1 客户端不会自动理解新摘要，须保留原合同或明确兼容错误；旧编辑器看不到完整意图时，
服务端应拒绝可能抹除未知配置的全量保存。当前 digest 是摘要而非数字签名，两者不能混称。

## 后续验收方向

1. 已确认的正确性修复独立完成；可并行整理现有三协议的 descriptor、共享校验与材料需求，
   以及生成器的判别联合支持。既有 WS/HTTP/underlay 修复文件继续由原任务单独维护。
2. 冻结版本协商与 instance-bound 原子能力报告合同，再提供控制面目录与逐节点/节点对的
   适配结果；Web 选项、合法组合和默认值由该结果驱动。
3. 统一 Agent factory，原生 UDP/TCP 与 VLESS 共用 owning socket/resolver 接口，保留
   socket 保护和 packet retirement 的不同职责；消除固定三槽统计与重复枚举映射。
4. 选择一个现有库级协议验证完整扩展流程，记录新增协议到底需要修改哪些注册点。
5. 覆盖不同 feature/版本组合、未知 schema、能力撤回、非法协议组合、秘密隔离和缓冲取消。

以上为审查建议与验收方向，不表示已增加协议或完成自动主备。首版仍以优先级主备为目标，
按流负载分担、同流多路径和动态插件加载不由本次 registry 整理引入。
