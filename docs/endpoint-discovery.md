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

## 显式启动

以下是当前诊断 / 实验参数。已确认目标是 STUN 与承载配置由控制面管理下发，设备不维护本地业务配置；自动方案将使用 STUN 观测公网 IP 的地区，但当前观测切片尚未实现方案切换。

在现有 Agent `run` 命令中加入一个或多个 `--stun-server <数字IP:端口>`，并使用 `--apply-connections` 或 `--apply-dataplane`。保留原来的 state、profiles 和数据面参数；由同一 daemon 管理 socket 和身份锁。

不提供默认公共观察点。没有配置 STUN server 时，报告明确为 `disabled`；有观察点但尚无可探测的 Native UDP 资源时，不能据此宣称网络不可达。

监听 socket 与 outgoing candidate socket 分别标记。outgoing socket 得到映射不改变其 dial-only 职责，不会因此成为未经配置的入站监听器。IPv6 底层地址观测也不等于 IPv6 Mesh 数据面。

## 类型化报告

Rust 契约位于 `crates/protocol/src/endpoint_discovery.rs`，经实际路由生成 OpenAPI 和客户端：

- 节点 `reportEndpointDiscovery`：`POST /api/v1/agent/connections/discovery`，由 NodeBearer 决定节点归属。
- 管理员 `getEndpointDiscoveries`：`GET /api/v1/connections/discovery`，返回稳定 Network ID 与节点观测。

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

2026-10-01 最终原生 ARM64 验收通过 **8 个阶段、10 项主要检查**，run ID `mupsr1dt-09b3d16f20327dee`。镜像为 `meshway-manageddiscovery:m2b-20261001-b`，不可变 ID `sha256:4a2d7f1389c7dddb80c129edf9db5b2d46bd883333b5e8330f56fd0834f87724`；独立资源清理无错误。证据同时对照了 observer `recvfrom`、Agent 进程 UDP FD / inode、`ss`、生成 API 的 socket 关联，以及实际 Noise nonce 往返。

本机开发 Controller、Web dev 和三个 Docker demo 已同步更新。已有 demo 没有配置 STUN 观察点时会真实上报 `disabled`，不会自动向公共服务探测；显式开启的真实观测由上述独立 fixture 完整验证。

下一片在有身份与有效期的观测基础上，向获准的同一逻辑邻接分发候选，并做真实 peer 连接验证、打洞与优先级主备切换。
