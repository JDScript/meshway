# P3a Mesh 执行切片（实施合同）

本批目标是 IPv4 Mesh source v3 → 同事务 publication → 生成客户端 → 正常 Agent run / TUN；完成证据另行记录。Mesh 外 profile/domain/NAT、自动邻接/主备和打洞不在本批执行合同内。

- 源唯一为 policy 的 `NetworkPolicy`，管理 envelope 引用它。v12 新表保存显式 source mode、v3 JSON/JSONC，旧 v2 表、身份、地址、注释原样保留；升级不自动激活或放宽 reject。不能等价迁移的旧意图报告诊断，激活必须经管理鉴权事务和网络 revision CAS。
- 节点合同独立于 source/compiler internal：protocol 的版本化 IPv4 Mesh 快照只携带网络、地址绑定、精确有向路径、身份与同 revision 邻接。Controller 用既有托管 listener / 连接意图构造候选图；候选并非实测可达。连接 block 优先，grants 并集，Mesh 有序 first-match，无规则自动规划；本批选择唯一主路径，不引入主备。
- capability 必须绑定现有 runtime process nonce / epoch。begin 新 epoch 清除旧报告，迟到请求不能写新 epoch；不从 agent_version 推断。旧节点端点在 v3 模式返回 fail-closed 兼容库存视图，不下发可执行的旧策略授权。混合节点不能因缺少能力而改选绕过必经点的路径。
- Agent 独立校验版本、网络/节点绑定、revision/digest、路径/身份/地址和预算；不使用 v2 路由重新过滤 v3。沿用 Manager packet fence/watchdog，撤旧代→持久化/准备 sink→ACK→执行；ACK 不是生效。缓存只提供回滚下限，不离线激活。
- 仍需显式已有 TUN opt-in。复用已合 owning hook；正式 underlay preparation API 由另一任务交付后协调者衔接，不把未接通保护称为已启用。

验收须使用生成管理 API、正常 Controller 和 Agent run、隔离 Linux TUN 真实双向包，并覆盖必经点、顺序、拒绝、撤回、重启、HTTP stall/lease。未完成此验收不能称 P3a 完成。

## 实际接线与兼容

- 管理入口为 `GET /api/v1/networks/{network_id}/policy-v3` 和 `POST .../policy-v3/save`。保存唯一 `NetworkPolicy` 源，显式激活 Mesh v3；可保留 JSONC，解析内容必须等于 typed policy。管理 mutation 使用原 root/RBAC 事务、鉴权时钟与全网络 revision CAS。首版此入口拒绝公共出口规则、profile/assignment/自动地区配置及外部目的地 grants，避免保存后静默丢弃 P3c 语义。
- v12 只添加三张表；v11→v12 在一个事务内完成 DDL/schema gate/version commit，已有 v2 源及其 JSONC 不改写、不自动换 source mode。旧 deny/reject 等无法等价转换时返回迁移诊断。非空旧执行授权的 direct/exact-hop 路由与 v3 自动候选图规划不能一般证明等价，本批不提供可自动激活的迁移结果；管理员明确替换源是新的管理决策，不冒充无损迁移。无授权、可转换的源仅提供 proposal，仍须显式 CAS 保存。
- 节点入口为 `POST /api/v1/agents/mesh-execution`（operation `negotiateMeshExecution`）与 `.../ack`，由真实生成 Rust 客户端调用。前者明确是能力发布 mutation 加原子配置下载，不是无副作用状态读取。请求带进程 nonce、runtime epoch 与 `ipv4_exact_path_v1`。begin 新 epoch 删除旧能力/执行 receipt；仅当前 epoch 的请求能报告支持或 ACK，迟到请求冲突。支持变化在 v3 下 bump 同一个 revision。旧节点不收到 v3 授权的 v2 包装；旧 dataplane/configuration 只给带明确诊断的 fail-closed 库存视图，DNS/exchange/heartbeat/旧 ACK body 不变。
- publication 在同事务读取 source、活动库存/地址、tag、连接候选、epoch 能力与不可变公钥，整体编译，再投影本节点精确路径和私有材料。物理图只采用已配置 listener/candidate，不把配置候选称为实测可达。任一路径参与者缺能力/密钥时撤回完整本节点执行，不移除必经点重选路径。源非法与合法但候选路径不可用分别表达；硬预算失败回滚整个发布，不截断计划。
- Agent 正常 `run --apply-dataplane` 先建立 runtime lease，再协商取 v3 快照。独立 `.mesh-execution.json` 私有 journal 保存 network/credential/revision/digest 下限，重启不离线激活；旧库存 cache 单独读取/保存，不能授权 v3 包。直接构造 IPv4 packet admission，完全不调用 v2 routing intersection 重新解释路径。准备 sink 与 MTU/admission 校验在 ACK 前；物理 listener/socket 成功及 TUN execution 是 ACK 后的独立运行状态。
- 每节点 128 条精确路径、8 节点路径、64 union identities 和完整 2 MiB 快照为硬预算。双向 carrier 仅复用通道，授权仍有方向；首版唯一主路径，没有主备或同流聚合。本批新 Agent 的 managed TUN 协商要求支持此新操作的 Controller；saved-only 库存消费者保留旧合同。

## 尚未完成的业务边界

- **单向 Mesh grant**：只安装源→目的路径，真实请求可进入目的内核；当前没有状态型 Mesh 回包 admission/flow tracker，ICMP/TCP 回包与反向主动业务均不获隐式允许。需要往返业务时必须显式双向 grant。后续接点是端点 packet admission 的状态型回包与精确路径绑定，不能简单增加反向 grant。P1c 外部出口原型的回包跟踪不等于此项已接入正常 Mesh。
- **系统保护通路**：已合 owning/prepared hook 继续保留，正式 namespace reservation/恢复与准备 API 来自另一任务；本批未接通该待交付 API，默认 socket 级完整保护尚未启用。没有为此增加目的 IP 全局豁免或关闭 packet fence。现有 TUN 原型的 pin 限制仍需协调者用正式保护生命周期收敛。
- **STUN/GeoIP**：既有真实 UDP carrier STUN 与 typed 上报已实现；协调者核对的三个 live Demo 为 disabled/0 observations。本批未操作这些 Demo。平台默认观察点与控制面发现配置下发、GeoIP 查询/数据更新、自动方案实际消费仍未完成，`StunLocation` 只是纯引擎输入。TCP/VLESS-only 没有可冒充为 UDP carrier 的观测 socket；不把 UDP 观测推广成 TCP listener 可达性。旧 node.country、controller 来源 IP、出口 IP 均不作实测地区；后续发现配置由控制面下发，不新增设备 STUN 参数入口。
- 完整出网/profile/domain/NAT、IPv6 overlay、自动邻接/承载主备、打洞及远端联测保持后续边界。

## 候选 HTTP / 消费者协商清单

这四个新增操作属于本分支候选契约，不混入协调者冻结的已发布版 HTTP 审查结果。均为 Controller HTTP，独立于 Next Server Actions。

| 操作 | 权限/作用域 | 副作用与新鲜度 |
| --- | --- | --- |
| `getNetworkPolicy` / GET policy-v3 | 已授权管理 bearer；指定网络 Read | 源编辑资源读取，含 source-mode、CAS revision、源/JSONC及编译/迁移诊断；不是状态摘要或真实连通证据 |
| `saveNetworkPolicy` / POST policy-v3/save | 同网 Manage；root 鉴权事务 | 显式源替换/激活，必须 CAS；整个依赖发布通过后 commit |
| `negotiateMeshExecution` / POST agents/mesh-execution | 节点 bearer 推导网络/节点；当前进程 nonce/epoch | 能力发布 mutation；变更可 bump 网络 revision，然后同事务下载完整执行配置。仅此节点的配置下载携带私有承载材料，不供 SPA 状态轮询 |
| `acknowledgeMeshExecution` / POST .../ack | 同一节点及当前 epoch | 条件 revision/digest saved receipt；不证明 Applied，也不刷新数据面 lease |

源类型仍唯一为 `NetworkPolicy`。管理候选新增 `SaveNetworkPolicyRequest` / `NetworkPolicySnapshot`；节点候选为 `MeshExecutionRequest` / `MeshExecutionResponse` / `MeshExecutionSnapshot` / `NodeMeshExecution` / `MeshExecutionPath` 和专用 ACK DTO。发布白名单仅增加节点协商/ACK，管理源不加入 Agent 客户端。

nullable 与省略明确区分：保存请求 `source_jsonc` 可省略或 null，表示不保存 JSONC；管理响应 `policy`/`source_jsonc`/`migration_proposal` 总是存在，null 分别表示没有已激活 v3 源、没有保存 JSONC、没有可用迁移 proposal。节点响应 `snapshot` 必须存在，legacy-mode 为 null；v3-mode 缺失/null 均被 Agent 拒绝。空 `paths`/`materials` 是真实空投影，不用默认值伪造未知状态。执行 `valid=false` 表示本节点完整计划不准执行，不把它当作实际网络健康度。

后续 SPA 需协商新增源编辑操作及 revision CAS；纯状态页继续使用不含完整源/材料的状态读接口，并按其观测 epoch/sequence/时间判断新鲜度。不要轮询私有执行下载来制造状态、历史或健康证据。当前沿已有授权 Controller bearer/事务权限；用户已确认 SPA 沿用不透明 `mws_` 会话、LocalStorage 持久化/标签页内存捕获/Authorization 头，实例管理员 token 仅内存、无 BFF，同源 Controller 从运行时目录承载静态 dist。客户端身份上下文与静态承载由协调者另行接线，本批没有引入 Cookie/JWT 或静态导航 fallback。API/App 路径须保留，后续 fallback 不能吞 API/缺失资源错误。

成功 DTO 为 strict unknown-field：新增字段同样可能使旧消费者拒绝，不能把 additive change 当作天然兼容。源由专用 policy-v3 操作与显式 source-mode 定义版本；节点有执行 schema version/required capability/epoch 协商。后续扩展须协调生成客户端与消费者升级，不能通过宽泛 JSON/忽略未知字段规避版本边界。

## 复现入口

```sh
CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=target/p3a-execution cargo test --workspace --all-features --locked -- --test-threads=2
CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=target/p3a-execution cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
python3 experiments/transport/performance_lock.py docker build -f experiments/p3a-mesh/Dockerfile -t meshway-managedtest:p3a-execution .
python3 experiments/p3a-mesh/run.py meshway-managedtest:p3a-execution
```

新 fixture 使用普通 Controller、生成管理操作、正常 Agent run 和真实 TUN；托管 UDP listener 与连接意图均在平台设置，无本地 profiles。容器仅 `NET_ADMIN`/TUN、drop ALL、独立 bridge/私有 tmpfs，无 host network/privileged/宿主配置修改，完整功能验收持有共享 performance lock。报告输出到批准 TMP，失败证据保留，容器/网络均清理。首次 fixture 调试失败不能计作验收通过。

## 本批完成证据（2026-10-02）

实现提交：`4b00e7f`（Controller / v12 / source 与执行合同 / 生成器）和 `fe6c9e4`（Agent 正常 run / packet admission / 隔离 fixture）。两提交组成此 Mesh 纵向切片；单独第一提交不代表节点已执行 v3。

- 最终正常 Controller + Agent run + Linux TUN 验收 **PASS**，16 条证据、cleanup errors 为 0。包含 ICMP/HTTP 双向业务、必经点、first-match 与源遮挡示例、有序规则重排、grants 并集/无序与拒绝、单向请求的真实内核计数、重启、HTTP stall、lease 和必经点撤回。日志：批准 TMP 的 `p3a-execution-tun-final-2.log`；报告目录 `meshway-p3a-muqoy9ko-3c5452d16a44-3P8zLD/`。
- 镜像 `meshway-managedtest:p3a-execution-final` 的不可变 ID 为 `sha256:f0a1871ed57128b45b14f154664c5f32a553565d440e6c5c421e640fdb04245d`。使用专用 Dockerfile 和独立 Cargo cache / jobs=2；未碰 live Demo、真实 DB 或远端。
- 最终全 Rust 消费者 **572 passed / 7 ignored**，workspace all-features/all-targets Clippy、fmt 通过。默认并发的一次旧 roundtrip restart 检查遇到 `state is already in use`；最终相同 feature / 独立 target、test-threads=2 全套通过。精度修正仅在既有 history 测试按实际持久化的毫秒精度比较时间，未修改生产时间语义。最终日志 `p3a-execution-rust-final-4.log`；最后源遮挡提示文案调整再通过 Store 定向检查与 Clippy。
- 真实生成器 **7**、SDK **16**、既有 Web **184** 测试通过；最终 drift、SDK / Web strict typecheck 与 lint、Apple scripts、五个既有实验及新 fixture 的 TS 检查通过，既有 Web 与文档构建通过。SDK 原生 fixture 使用绝对独立 `target/p3a-sdk-validation`，与 CLI 验收 target 分开；没有浏览器自动化。

本批交付结论为 **IPv4 Mesh v3 的真实正常执行纵向闭环**。完整 P3a 的正式系统保护生命周期集成仍待协调者衔接；状态型 Mesh 回包、新网络默认模式的进一步产品收敛、P3b/P3c/P3d、STUN 平台下发/GeoIP 等保持上述边界，不用本批通过结果代替它们。
