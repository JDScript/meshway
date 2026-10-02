# 代码架构审查：边界与收敛顺序

审查基线：`c72cebb`。Rust / Web 专项读取同一冻结 checkout，协调者复核关键调用链与工具链。
Rust、Web 与工具链专题均已完成；协调者复核了发布事务、账户双向依赖、草稿生命周期及 SDK 错误处理等关键证据。
这是源码结构审查，不是漏洞利用、性能或远端验收；本页建议的模块尚未创建，不能当成现状。
原始报告保留在批准 TMP 的 `meshway-architecture-{rust,web,tooling}-c72cebb.md`。

## 总结

**优先收敛 module 的职责和数据转换边界，保持当前 crate 大方向。** 当前正常依赖未发现
Cargo crate 环；Controller 有真实的 `store ↔ accounts` module 双向依赖。
主要高变更耦合集中在 Controller 发布流程、Agent 运行编排、策略到执行合同以及承载装配。
文件长度本身不是缺陷，独立 crate 也不能替代清晰的接口与资源所有权。

当前已启动的 policy 发布边界、native underlay、账户 / 控制台三个任务继续按原文件所有权推进。
审查不读取其未提交内容，也不另起覆盖同一代码的大搬迁；合入前后以实际提交分别记录。

## 当前依赖与应保留的隔离

```text
controller ──→ protocol
           └─→ policy ──→ protocol
agent ──→ protocol / policy / 生成 HTTP client（包内）
      └─ feature → dataplane / transport-vless / 平台 TUN
apple-core ──→ agent / protocol / policy / UniFFI
transport-* ──→ 各自上游协议库
web ──→ 生成 TS SDK / runtime schemas ──→ Controller HTTP
```

Controller 不链接数据面或承载实现；Agent / Apple 对 Controller 的依赖只在测试中。
以下是已有的正确边界，整理时继续保留：

- Rust 实际路由与 DTO 是 HTTP 权威，SDK / schemas 生成；生成 DTO 与权威 DTO 的验证转换不等于手工维护两套合同。
- `NetworkId` 的存在 / 作用域证明与根事务内的用户角色检查分开；不能让类型搬迁开放任意字符串构造。
- 同 revision 的配置、路径授权和公钥快照；持久 ACK、应用状态、lease 与真实连接观测分开。
- packet Device / admission、Noise 与 E2E 路径边界；Linux 路由 / NAT 操作不进入纯策略引擎。
- transport 只承载流量，代理凭据不替代 Mesh 身份；FFI unsafe 限于 Apple 桥接。
- 实验锁 FD 继承、取消与精确 cleanup，以及开发容器原位更新的身份 / 可写层保护。

## 重点发现

优先级“近期”表示会影响下一阶段接线，不等于已确认安全漏洞。
以下行号相对冻结基线，可用 `git show c72cebb:<路径>` 重查。

### R1 · Controller 缺少统一发布入口（近期）

`apps/controller/src/store/access.rs:400–427` 的保存操作自行调用 configuration / connections /
access 三组 `ensure_publishable`；路由保存（`store/configuration.rs:239–274`）、DNS
（`store/network.rs:134–156`）与承载能力变更（`store/managed_transports.rs:568–598`）
也分别维护发布校验依赖。新增一种影响执行的配置，要依靠每个写操作记住完整依赖。

节点读取 `store/access.rs:533–553` 又会调用 `compiled_authorizations`，后者在
`:212–328` 遍历 eligible source × destination，编译全网授权，最后才取当前节点。
这些操作位于 `lib.rs:97–143` 的全局 `Arc<Mutex<Store>>` 内；`spawn_blocking` 避免阻塞
Tokio worker，但不消除存储临界区的串行化。本次未测量其耗时或吞吐影响。

**建议**：在 Controller 内增加具体的 `publication::{facts, compile, project, budget}`。
先保持事务语义，收拢“完成网络变更”的入口和一次编译、多节点投影。后续有测量依据再考虑
锁外编译 / 缓存；必须重新鉴权、CAS，并证明缓存 key 覆盖能力和观测的代际变化。
不先引入通用 Repository、异步任务平台或新的数据库。

### R2 · 编译器内部对象与设备执行合同之间缺少明确转换层（近期）

Controller 目前通过 `store/configuration.rs:128–159` 发布旧源路由；Agent
`managed_dataplane/mod.rs:107–172` 再调用 `policy::access::compile_mesh_paths`，
由 `crates/policy/src/access.rs:335–393` 重做路由交集。这是现有防御性边界，应在迁移中保留等价性。

v3 的 `Compiled` / `NetworkCompilation` 则含节点索引、路径池、matcher、前缀表和诊断
（`crates/policy/src/v3/compiled.rs:15–36`、`profiles.rs:119–126`）。P1c Agent 出口还使用
独立的 `ProfileCatalog` / `ExitPlan`（`apps/agent/src/managed_dataplane/exit.rs:58–105`）。
它们是不同阶段的内部表示，不能直接序列化编译器工作对象就当成稳定 wire 合同。

**建议明确三段转换**：

1. `policy` 负责纯策略语义和编译器内部计划。
2. `controller::publication` 投影为 `protocol` 定义的版本化、节点作用域、有上限的执行快照。
3. `agent::control` 对 wire 做防御性验证，产生构造受限的执行计划；运行层据此安装本机资源。

编译预算、完整 `global:<node-id>` 和 compiled 反序列化边界由已启动的 policy 任务处理。
其修复不等于 P3a 已上线；完整 profile / 域名 / 出口转换随 P3c 交付。

### R3 · accounts 与 Store 存在 module 双向依赖（可并行收敛）

`store/account_authorization.rs:2–7,57–69` 调用 accounts 授权；反向
`accounts/model.rs:194` re-export `crate::store::NetworkId`，账户 repository 通过
Store 定义的 lookup 构造它（`accounts/repository.rs:94–103`）。`model.rs:323–342`
还直接查询 `settings.admin_hash`。账户值模型因此知道持久层结构，而持久层又依赖账户模型。

**建议**：在 Controller 内建立中性的 `network_scope` / `identity` module，保持受限构造；
SQL 认证移入账户认证 / 存储 adapter，model 保留用户、角色、主体等值与语义。保持根事务
内鉴权和最后 Owner 等行为，不拆独立 accounts crate，也不把 network scope 当授权。

### R4 · Agent 状态模块兼任运行编排与进程入口（随自动组网收敛）

`state/connections/dataplane.rs:277–385` 同时负责 HTTP、校验、旧 runtime 撤回、材料安装、
sidecar 保存、TUN sink 与 ACK。`state/connections/managed.rs:110–175` 创建 Manager、
启动诊断、监听 Ctrl-C、关闭 DNS 与撤回；`:229–233,321–330` 直接写终端输出。

这使持久存储、同步、运行会话和 CLI 进程职责混在一个层内。多网络切换、主备和 endpoint
变化会不断触及同一编排代码，增加遗漏 teardown 顺序的机会。

**建议**：`state` 留身份与持久化；`control` 负责生成客户端 / DTO 适配 / 验证；
`session` 负责生命周期顺序；CLI binary 处理 signal 与输出，库接收取消信号和类型化事件。
沿现有调用链逐函数迁移，暂留兼容 wrapper。保留独立 lease watchdog，以及先撤旧代、
准备 sink、持久化 / ACK 的具体约束，不合并 socket 与 packet 两种 generation。

### R5 · Native 与 VLESS 仍有两条装配路径（近期，已启动）

Agent `managed_connections/session.rs:85–200` 单独创建 VLESS listener / dial tasks，
dataplane `runtime/mod.rs:328–418` 自行创建 native UDP / TCP；保护、解析、发现与退休必须
分别接入两个资源 owner。`apps/agent/Cargo.toml:8–11,52–53` 的 feature 主要隔离整块运行层，
尚不是逐协议独立能力目录。

**建议**：承接 native-underlay 的 owning socket / resolver 合同，逐步统一 carrier 装配；
core 保留 Noise、认证 channel、buffer / batch 和精确路径，Controller 管目录 / 材料 / 兼容。
静态 enum / descriptor 足够，不新增 registry crate 或动态插件 ABI。详见[承载扩展审查](/transport-registry)。
冻结树的 WS stream 能力已进入有效执行投影，不再将旧 MV3 缺陷写成当前问题。

### R6 · 生成 Rust client 随整个 Agent 包公开（延后）

`apps/apple/core/src/signin.rs:1,97–105` 直接消费 `meshway_agent::generated`；manifest
因此依赖 Agent 包。默认构建能排除 managed 数据面，但普通 CLI / policy 等依赖仍在包边界内。
这证明构建 / API 耦合，不证明 Apple 拥有管理权限或包含整个数据面二进制。

先保留兼容的内部 client facade；Apple 恢复或存在独立 SDK 发布需求时，再考虑一个独立
生成 `control-client` crate。它有真实的多个消费者，是较有依据的未来拆包候选，当前不阻塞 Linux。

## Web 分层与共享职责

会话 Cookie、渲染登录 marker、URL 网络作用域、生成 SDK 响应校验、草稿 owner 和真实地图
数据边界已形成合理设计。本次发现主要是这些边界的执行代码重复，以及契约限制在 UI 中另有来源。
`packages/control-client` 保持一个包；当前没有将仅 Web 使用的 UI / model 再抽共享包的需求。

### W1 · CAS 编辑会话复制了三份（P3a 新编辑器前收敛）

Routing `components/routing/policy-editor.tsx:466–525`、Access
`components/access/access-editor.tsx:319–361` 和 Connections
`components/connections/connections-editor.tsx:401–450` 分别实现相同的保存 / 重读生命周期：
捕获 base 与发送序号、判断当前 owner、处理卸载后的保存结果、屏蔽过时重读。
claim / release、持久化 effect 和 Provider 也各自重复。草稿种类同时登记在
`lib/draft-vault.ts:6–49`、`components/network/network-workspace.tsx:64–70` 和导航等位置；
managed listener 已计入 vault，却没有对应导航草稿种类。

**建议**：提取窄的 CAS session hook 和草稿注册描述，业务文档的保存归并逻辑仍由具体 feature 提供。
先迁移一个编辑器验证，再用于新 grants / 连接模板 / 路由方案。每个文档保留自己的 CAS 基准；
网络共用 revision 不意味着草稿可以自动跟随它。owner、sentSeq、注释 / 无效 JSONC 和冲突保留
是必须保持的行为，不能为了抽象减少这些状态。

### W2 · Server Action 边界由重复骨架维持（随账户 / 策略收敛）

`lib/connections-actions.ts:49–85`、`policy-actions.ts:75–113`、
`managed-listener-actions.ts:44–73` 等反复实现登录 marker / 网络检查、调用、状态映射与 401 跳转。
`MAX_REVISION` 和输入大小检查也多处手写，结果类型集中在全局 `lib/action-state.ts`。

**建议**：一个 server-only 的 network-action helper 只负责共同边界，状态映射和结果类型归各 feature。
Next `redirect` 以异常实现，继续在错误捕获之外执行；401 跳转不误删新登录 Cookie，403 保留会话。
上限校验尽量直接消费生成 schema。少数 scope 从已绑定的数据 / query 组装的调用可统一接
`useActionScope`；本次只确认写法不统一，未证明这些等价来源造成跨网越权。

### W3 · 限值、枚举与镜像校验缺少单一来源（跨前后端接线）

`lib/connections-model.ts:21–30`、`managed-listener-model.ts:55–60`、
`tags-model.ts:9`、`access-model.ts:19–26` 手写 OpenAPI 已有的上限 / 枚举。
`policy-model.ts:19–25` 的部分规则限值尚未表达在 wire schema。
`managed-listener-model.ts:14–18` 明确说明它在镜像 Controller 校验。

这里表单到 DTO 的 adapter 方向本身正确，问题是约束数据需要人工同步且不受 drift 检查。
SDK `packages/control-client/src/transport.ts:37–40` 丢弃失败 body，只暴露状态码；
这不等于 Controller 完全没有错误 body，而是 UI 缺少可消费的字段级诊断。

**建议**：优先从现有生成 schema 派生枚举 / 限值；必要时由生成器输出紧凑的 limits 产物。
纯策略限值进入权威契约后随 drift 检查。后续提供生成的字段级 422 诊断，使 UI 不必复制服务端
业务校验；保留有界读取、脱敏和 status-first 认证错误处理，不直接回显任意错误 body。
不同用途的同名视图类型按真实语义命名，不为减少类型数量强行统一。

### W4 · 观测轮询与多源状态合并混在一起（节点重整后收敛）

`components/nodes/node-detail.tsx:125–174` 与
`components/topology/topology-view.tsx:101–125` 复制多源读取 / 合并逻辑，
`components/connections/use-runtimes.ts:37–95` 的 runtime 轮询实际驱动多个资源。
历史和 endpoint 观测另有自己的可见性轮询。

**建议**：提取只管理可见性、调度、取消和序号的 poll helper；多源结果归 observation / nodes
状态模块。各数据源继续保留自身缓存 / 失败策略和时间：失败保留旧值不代表旧值仍新鲜，
读取得到的观测不移动编辑草稿基准，也不创造新的延迟样本。由现有节点任务采用，或在其合入后迁移。

### W5 · 目录与检查入口强化了 lib 平铺（渐进整理）

`apps/web/package.json:13` 的测试入口只收集 `lib/*.test.ts`，把纯模型移到 feature
子目录后不会自动执行其测试。`components/network/network-workspace.tsx:5–11,23–70`
实际承担全控制台的登录 / 网络 / 草稿基础设施，其他 feature 为取得 action scope 反向依赖
“网络设置”目录。两个 `network-context` 名字分别代表服务端 action 边界与客户端路由 context。

**建议**：先让测试发现支持 feature 子目录，再把控制台公共工作区与 feature UI 分清；
纯 model / result / action 与对应 feature 逐步归拢，保持 server-only 与可在 Node 中测试的纯模块边界。
冻结树有无调用的 `selectedNetworkId` / `requireActionCredential`；弃用的 `requireAdminToken`
仍有两个调用方，应先迁移调用后删除别名。账户工作树合入前不抢改其 session / shell 文件。

### SDK 公开面补充

`packages/control-client/src/index.ts:1–55` 是手写 re-export 清单，部分实验因此深导入生成文件；
Apple 脚本还从 SDK 的 `node_modules/zod` 路径取依赖。由生成流程维护公开 operation 导出，
需要时提供明确 agent 子路径，消费者声明自己的直接依赖。该调整与生成器一起由协调者推进，
无需新建 UI / domain 包。

## 工具与实验的长期归属

### T1 · 日常入口反向依赖 experiments

`scripts/dev.py:22–24,350–392` 依赖 `experiments/transport/performance_lock.py` 和
`experiments/managed-connections/demo.py`；`scripts/dev_demo_update.py:126–136,197–209`
使用实验目录的锁与镜像入口。部分“实验”已经承担长期开发基础设施职责。

将共用锁、进程与容器 ownership 收拢到稳定的 `scripts` / `tools` 内部模块，日常服务与实验共同使用。
保留同一锁路径、FD 继承与精确 cleanup，不趁搬迁改部署行为。远端部署产品化时再收敛其入口。

### T2 · 生成器能力是隐含的类型扩展边界

`scripts/generate-protocol.mjs:20–43` 编排多个生成器，`:85–94` 只支持 nullable 引用联合，
`:185–217` 限定 GET/POST、单 bearer、唯一 200 等形状。显式拒绝不支持类型是正确的，
但 typed transport / v3 必须先扩展生成链，不能为迁就生成器改成 string 加任意 JSON。

分离 schema / operations / editor emitters 与运行编排，明确受支持形状，按真实新增合同补必要
fixture。保留实际路由权威与再生成 drift，不追求一次实现全部 OpenAPI。

### T3 · 消费者清单分散

根 `package.json:25–26` 检查 pnpm workspace 和 Apple scripts；CI
`.github/workflows/check.yml:89–105` 手写四个实验 TS 目录，但
`experiments/managed-transports/run.ts:8–15` 也消费生成 SDK，且有自己的 tsconfig。
目前完整覆盖依赖协调者按 AGENTS 人工补跑；本次未发现其当前编译失败。

用单一消费者清单或受约束的 tsconfig 发现驱动本地 / CI 类型检查；行为测试另按实际入口组织，
不因发现 tsconfig 就启动 Docker。不同 Cargo feature 构建仍须遵守独立 target / 串行边界。

### T4 · 已合入功能仍有分支拼装入口

`apps/agent/tests/underlay_fixture/combine.py:22–62,95` archive旧 P2a 提交，复制当前
underlay，通过字符串锚点和 `docs/underlay-*.patch` 拼装历史组合。该历史实验有价值，
但当前源码、待应用补丁与历史验证混排，且组合输入不只一个固定提交。

日常验收覆盖实际 workspace；历史组合固定双方提交 / 产物，已合入补丁退出当前接线指引。
保留原反例和行为测试，不按“实验 / 旧”目录名字批量删除证据。

## 随里程碑落实的顺序

下表保留审查时建议的依赖顺序。用户随后决定暂停进一步前端重整，当前只沿 Linux 真实组网
所需边界推进；Web 结构建议留给后续独立工作流，最新执行优先级以[里程碑](/milestones)为准。

| 时机 | 结构工作 | 保留的交付边界 |
| --- | --- | --- |
| 当前三条实现线 | policy 发布预算 / ID、native owning I/O、账户 / 控制台功能 | 不做全树 rename、不覆盖活跃文件；根 manifest / lock / 生成物由协调者维护 |
| Web 首批页面合入后 / P3a 编辑器之前 | feature 测试发现、network action helper、CAS 编辑会话与草稿注册 | 现有用户页面先交付；归拢共享机制再扩展新编辑器，按文档保留基准与 owner |
| P3a + underlay | Controller publication；policy → wire → validated plan | 同事务鉴权 / CAS、一致快照、撤回、迁移与预算；不急于锁外缓存 |
| P3b / P3d | carrier 目录 / 装配、Agent control / session / CLI 职责 | 主备不改变逻辑路径，socket 与 packet 退休独立有效 |
| 四地 Linux 联测 | 固定边界，以真实部署和故障恢复验收 | 不插入大规模拆包 / 性能矩阵；用测量决定后续调度优化 |
| P3c | profile / 域名 / 出口的唯一编译投影与本机安装 | 逐流可信域名、授权、NAT / 回包、系统保护先于全 IP 接管 |
| Apple 恢复后 | 评估独立 Rust control-client | 沿生成链兼容迁移，避免 Agent-core 大拆迁 |

实现时只为关键行为边界运行必要测试；本次审查本身不运行行为测试或浏览器自动化。

## 审查后独立修复记录

原已启动的 policy 发布边界任务以 `ddaee7c` / `a1b992f` 合入：实际展平安装表计入双栈、
direct 与 nodes final，超限整体失败；Agent 与 policy 共用完整 global / 自定义方案 ID 校验；
编译计划顶层及索引不再提供直接 `Deserialize`。独立复核通过，随后主线 workspace
all-features / all-targets 编译检查通过。这些变更不改 HTTP 契约。

此处只关闭直接解码入口；公开字段仍可由 Rust 调用者修改，不构成不可伪造的 validated plan。
R1 / R2 的正式 publication / 节点 wire 转换、原型目录上限及出口侧总预算仍待后续接线。

Native owning factory 与 SocketGeneration 接线以 `46a7b42` / `94c65ea` 合入。
独立复核先复现旧 carrier 入站跨 rebind 交付，再验证修复后的 UDP/TCP × endpoint/relay
四种交错、新 carrier 恢复和已退休 PacketGeneration 不复活。R5 的统一 owning I/O 接点已落地，
正式主循环 prepared 激活、RPDB owner / capture 与后续 registry 仍待接通；不计作生产全 IP 保护已启用。
