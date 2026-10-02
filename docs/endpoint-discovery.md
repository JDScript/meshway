# 实际承载上的 endpoint 观测

这条 M2 切片把 STUN 查询接到 managed Native UDP 的真实监听 / 拨号 socket 上。控制面单独接收有期限的 socket 观测，控制台显示候选来源、关联资源和失效状态。STUN 观测仍须经过后续 peer 认证与连接验证，才可能用于自动建连。

## 与独立 netcheck 的分工

| 来源 | socket 生命周期 | 含义 |
| --- | --- | --- |
| `netcheck` / `independent_udp_probe` | CLI 自己创建，检查结束就关闭 | 当前网络环境的一次观测 |
| `managed_udp_socket_stun` | managed Runtime 已有的 UDP socket | 该运行代际、该资源实际使用的 socket 的观测 |
| 管理员 advertised endpoint | 已保存连接意图 | 希望对端尝试的地址，保存本身不证明可达 |
| 已认证邻接与 nonce ping | 当前已认证 channel | 同一邻接上的真实认证 / 往返证据 |

这些来源分别呈现。观测结果不会自动改写连接配置、Access 或 Routing；必经节点与端到端加密语义保持由原契约约束。

## 产品中的地址与地区观测

管理员配置本地监听地址 / 端口；NAT 下对外映射的端口可能不同，并会随网络、目的端或
socket 生命周期变化。STUN 查询观测已有映射，peer 打洞通过发包建立 NAT / 防火墙状态；
UPnP、NAT-PMP、PCP 则是向网关显式申请映射，三者不等价。IPv6 还可能没有 NAT，
但仍需验证防火墙和 peer 可达性。当前实现只提供上表的来源，不宣称支持显式端口映射。

已确认的控制台目标：

| 展示项 | 含义 |
| --- | --- |
| 监听配置与实际绑定 | 协议、本地地址 / 端口，以及 Agent 是否成功绑定 |
| 对外候选地址 | 管理员填写或实际 socket 发现的地址，保留来源与有效期 |
| 对端连接验证 | 针对哪个 peer、哪个承载的认证连接与探测结果，不是地址的全局可达布尔值 |
| 自动路由方案依据 | 当前底层公网 IP、推导地区、观测时间与选中方案；无有效依据时明确显示兜底 |

候选归属实际 listener 或 outgoing socket；无 listener 的节点仍可用主动拨号 socket
建立双向承载。UDP 映射不证明同端口 TCP/VLESS/WebSocket 可达，不自动把 dial-only
socket 变成产品上的入站 listener。配置值和动态观测独立，地址发现不回写管理员的 bind。

地区推导可复用有效 STUN 结果中的 IP，但只表达设备当前底层网络的出口归属，不是 GPS。
它必须区别于 Mesh 地址、已选择的隧道出口地址和管理员手填的国家。多网卡、IPv4/IPv6
或观察点结果不一致时保留观测上下文，不能任取一个结果冒充唯一设备位置；没有有效
底层观测时使用自动方案的兜底。GeoIP 推导与自动方案下发尚待 P3c，不能将当前
endpoint 上报描述为自动地区切换已完成。

实际进度分为四项：socket STUN 与上报已经实现；平台默认观察点 / 配置下发尚未完成；
Controller 尚无 GeoIP 数据库查询 / 更新与有来源的位置接口；纯策略引擎仅接受外部提供的
`StunLocation.region`，不负责取得它。自动方案使用真实数据并应用到节点仍待 P3c。
逐项验收及仅 TCP/VLESS、无 UDP、换网、多源冲突等场景见[里程碑核对表](/milestones#发现与地区信息的闭环核对)。

## 显式启动

以下是当前诊断 / 实验参数。已确认目标是 STUN 与承载配置由控制面管理下发，设备不维护本地业务配置；自动方案将使用 STUN 观测公网 IP 的地区，但当前观测切片尚未实现方案切换。

在现有 Agent `run` 命令中加入一个或多个 `--stun-server <数字IP:端口>`，并使用 `--apply-connections` 或 `--apply-dataplane`。保留原来的 state、profiles 和数据面参数；由同一 daemon 管理 socket 和身份锁。

不提供默认公共观察点。没有配置 STUN server 时，报告明确为 `disabled`；有观察点但尚无可探测的 Native UDP 资源时，不能据此宣称网络不可达。

监听 socket 与 outgoing candidate socket 分别标记。outgoing socket 得到映射不改变其 dial-only 职责，不会因此成为未经配置的入站监听器。IPv6 底层地址观测也不等于 IPv6 Mesh 数据面。

## 类型化报告

Rust 契约位于 `crates/protocol/src/endpoint_discovery.rs`，经实际路由生成 OpenAPI 和客户端：

- 节点 `reportEndpointDiscovery`：`POST /api/v1/agent/connections/discovery`，由 NodeBearer 决定节点归属。
- 管理员 `getEndpointDiscoveries`：`GET /api/v1/networks/{network_id}/connections/discovery`，只返回所选网络的稳定 ID 与节点观测。

报告引用已取得的连接 runtime epoch、独立递增的 discovery sequence，以及当前 node-scoped revision / digest。socket 包含 controller resource ID、运行 generation、非秘密 socket ordinal、实际绑定地址和最多四个观察点结果。

每笔结果有 observer、outcome、可空 mapped endpoint / RTT、`observed_at` 和 `expires_at`。失败不能携带伪造映射；同一次观测的重报不是续租，真正的新事务才产生新的观测时间。

## 新鲜度与撤回

- socket 观测绑定当前 Runtime。配置换代、lease 到期、socket 退出或 daemon 停止时，旧代际的结果失去资格；等待中的查询也必须随之结束。
- 有效期不超过测量时的配置 lease 和 30 秒上限。普通配置续租不会延长上次 STUN 答复的有效期。
- 控制面校验节点 / epoch / revision / digest 与资源归属，再按运行报告、撤销状态和观测期限投影可见结果。读取和重复提交不会刷新期限。
- 尚未报告、显式关闭、正在测量、过期、撤回和失败分别表示。过期 endpoint 不能继续以新鲜候选显示。
- 诊断上报的 409 / 422 不直接等同于运行租约或配置失效：Agent 在同一个有界 HTTP budget 内确认 epoch / source，只有确认变化才撤回。观测时间回退时暂停新采样，不钳制时间或延长旧期限；前端使用单调时钟避免已到期地址重新变得可复制。

控制面接收的是经过节点身份认证的自报告，不能独立证明 NAT 或 STUN 服务返回内容真实。它不把候选观测转化为访问授权或 peer 身份。

## 本地验证与下一片

隔离验收使用独立 controller、两个 Agent 和真实 STUN 观察点；观察点根据 UDP `recvfrom()` 返回源地址。对照 Runtime 的实际 socket 端口、生成 API 的资源关联，以及同时进行的 Noise 邻接探测，验证这次没有借用临时探测 socket。

生命周期验收还覆盖观察点失联、恢复、删除监听器、配置换代、控制面断联与节点撤销。独立 kernel NAT 条件矩阵仍见 [Netcheck](/netcheck)。

2026-10-01 原生 ARM64 历史验收通过 8 阶段、10 项主要检查，run `mupsr1dt-09b3d16f20327dee`，独立资源清理无错误。Artifact 对照 observer `recvfrom`、Agent UDP FD / inode、`ss`、生成 API 关联与 Noise nonce；镜像 digest 由结果记录。重现入口为 `experiments/managed-discovery/run.py`，构建与参数见目录 README。

没有配置观察点的 demo 真实报告 `disabled`；当前现场需重新检查，历史验收不表示所有已接入设备都已开启发现。
2026-10-02 本轮只读查询确认三个本地 Demo 均为 `disabled`，socket / observation 为 0。
这不否定已运行的邻接延迟采样，也不表示已有 GeoIP 依据。

下一片在有身份与有效期的观测基础上，向获准的同一逻辑邻接分发候选，并做真实 peer 连接验证、打洞与优先级主备切换。
