# 连接获取与 UDP 穿透

Meshway 共用一套承载会话与受保护数据面，将显式端点建连和协调 UDP 穿透作为独立的连接获取实现。连接如何建立、协议如何封装、业务是否获准分别判断。

## 分层与所有权

```text
显式端点：listener／dialer ────────┐
                                  ├→ peer 认证／会话选择 → SecureNode → TUN／业务入口
协调穿透：绑定 UDP／发现／互探 ────┘
```

| 归属 | 职责 |
| --- | --- |
| `meshway-control` | 唯一身份、执行快照、lease、失效与 Server 请求 owner |
| `meshway-policy` | 业务、完整路径与获准连接范围的共同判定 |
| `meshway-underlay` | 实际 socket、捕获豁免、平台资源与释放 |
| `meshway-connectivity` | 两类连接获取、peer 认证、会话选择／重连与真实路径事实 |
| transport provider | 在要求的底层连接上完成协议握手、封装与 I/O |
| `meshway-dataplane` | 唯一业务流表、端到端保护、逐跳证明与原路径回复 |

crate 职责总表见[跨入口复用](/v2/architecture#跨入口复用)。两类入口不分别维护身份、权限、业务流表或加密状态。候选观测变化不自行修改业务授权；更换物理承载仍须符合当前许可及完整逻辑路径。

## Provider 的连接需求

连接需求描述的是 provider 接收什么底层资源，不代表网络一定可达，也不代表业务发起方必须是拨号方。

| Provider | 外层资源／连接需求 | 连接获取范围 |
| --- | --- | --- |
| Native TCP | 已建立 TCP stream | 正常拨号或接受远端拨入；可反向拨号 |
| VLESS TCP/TLS、WS/TLS | 已建立 TCP stream，再执行 TLS／HTTP／VLESS 角色 | 同上 |
| Trojan TCP/TLS | 已建立 TCP stream，再执行 TLS／Trojan 角色 | 同上 |
| Native UDP | 已绑定 UDP socket、获准 peer 与 datagram 关联 | 显式端点与协调 UDP 穿透 |
| MASQUE／HTTP/3 | UDP 上的 QUIC／HTTP 连接 | 后续 provider；可复用 UDP 连通性，但仍需协议 client/server 角色 |

当前产品穿透范围限 UDP，不实现 TCP simultaneous-open。现有 TCP 系 provider 的“需要拨号”也包括对端拨入得到连接；接受连接不意味着创建第二种业务身份。

TCP／VLESS／Trojan 会话建立后同样可双向收发。UDP 打洞解决路径可达性，不能据此推导双向业务授权。内层 TCP／UDP 业务独立于外层承载；VLESS-only 等协议限制不会静默启用 UDP 业务承载。

上表是资源与行为边界。当前 Rust 接口见[承载入口](/v2/node/transport#当前-rust-入口)。协调穿透复用现有 Native UDP 声明与 Carrier，不增加 `requiresDialing` 等推测能力字段。MASQUE 尚未接入；穿透的验收状态见[验收范围](/v2/node/validation)。

## STUN 与 Server 协调

STUN 使用 RFC 8489 Binding 交换观察 socket 的外部映射；Meshway Server 负责当前主体／网络权限、获准 peer 的候选交换和有期限的尝试协调；Node 使用实际 socket 互发探测并完成认证。

Server 的 STUN 监听与发布端点由 `--stun-listen`／`--stun-server` 分别配置，参数见 [Server 运行参数](/v2/server#运行参数)。两者默认均为空；监听地址与对外端点分开声明，不从 wildcard bind 或 HTTP 地址猜测。

`meshway-stun` 提供有界 RFC 8489 Binding 编解码与 UDP Service。它校验事务、消息／属性长度、XOR-MAPPED-ADDRESS 和可选 FINGERPRINT；客户端还验证实际响应来源。Binding 结果是未认证的映射观测，服务按来源 IP、接收包数、输出字节和有限来源表限流。STUN 不承担管理 HTTP 或 Mesh 业务转发。

- 观测必须来自将用于该连接的 UDP socket，保留本地端口、socket 身份与运行代际。另一临时 socket 的结果不能冒充当前承载映射。
- 候选须绑定网络、peer、外层 socket、来源和观测有效期；它只提供尝试地址，不授予身份、Access 或路径权限。
- HTTPS 请求来源地址不能代替 UDP 映射。STUN 结果也不保证对另一目的地址使用相同映射；观测有效期不等于 NAT 设备保证的映射寿命。
- 外部 STUN 候选属于经过身份认证的 Node 自报事实，Server 不把它视作地址所有权证明。双方当前获准的精确邻接可使用这类候选进行有界探测；目标必须是合法的数字 UDP 单播地址，排除 loopback、未指定、多播、广播、不可用的 link-local／scoped 地址和 Mesh 捕获目标。Node 还检查本机不可作为探测目标的地址。探测按原期限限制包数、字节数和并发，不能扫描相邻端口或由候选授予业务权限。
- 尝试与缓冲有界，过期、撤回、取消和 owner 停止后不再迟到安装。实际包发送仍检查原操作期限。

## Node 协调契约

以下端点使用 Node 凭据，定义来自同一[公开 OpenAPI](/v2/contracts/openapi.json)。普通候选变化／尝试过期通过封闭的 `Unavailable` 结果表达；它们不伪装成凭据失效。

| 操作 | 路由 | 行为 |
| --- | --- | --- |
| `queryUdpTraversal` | `POST /agent-api/v1/connectivity/udp/query` | 按当前 instance／configuration 返回 State epoch、明确的 STUN 端点和仅属于本节点的有效尝试；查询不续期。 |
| `prepareUdpTraversal` | `POST /agent-api/v1/connectivity/udp/attempts` | 使用稳定 request ID、绝对期限、精确 Carrier 与双方候选引用创建不可变 offer。 |
| `acceptUdpTraversal` | `POST /agent-api/v1/connectivity/udp/attempts/{attemptId}/accept` | 检查 offer version／digest；双方各自预留真实接收槽并接受后，才返回 Active。 |
| `cancelUdpTraversal` | `POST /agent-api/v1/connectivity/udp/attempts/{attemptId}/cancel` | 幂等取消未完成的获取过程；不据此撤回已经独立认证的业务通道。 |

`SocketReference` 绑定 node、instance／generation、完整 configuration 引用、实际 socket ID／generation 和 resource。`CandidateReference` 进一步绑定原始地址、来源、观测／到期时间及 bind 信息的规范摘要。摘要不包含外层 report sequence 或接收时间；重复上报／读取不会刷新观测。State 是候选的唯一 owner，peer 视图携带引用并继续隐藏不必要的本机接口／observer 细节。

每次准备、接受及获取授权均检查双方当前身份、实例、精确 UDP 邻接及共同的业务或诊断完整路径许可。诊断 witness 另绑定同一 ObservationId、source instance、service 和完整路径，只用于获准诊断连接，不授予业务包权限；可见 peer 不自动获得非相邻 socket 端点。offer 的 nonce、候选对、粗粒度 endpoint source、digest 和期限固定；寿命最多 10 秒，且不超过原候选、报告、session／instance／execution 与所选许可期限。每节点最多八个活动尝试，每网络最多 256 条记录、进程最多 4,096 条记录；同一无序 peer 对／地址族不并行安装不同活动尝试。

Node 先在实际 UDP owner 预留 STUN 事务，再经同一 Session sender 发送，由唯一 receiver 分流响应。候选的真实 State 上报获确认后，才参与 offer。双方 Active 后从同一 socket 发包，签名握手额外绑定 offer 的 nonce／digest；接入前再次验证原尝试与接收槽，旧尝试不能借新 channel scope 延期。获取成功后，通道由当前执行许可和认证健康检查维持，不依赖过期的 STUN 观测续命。

### UDP 控制消息预算

UDP 接收 owner 在每个就绪批次内先分流 health、邻接握手、STUN 和穿透控制，再交付业务帧；控制消息不进入业务事件队列。业务队列满时只丢弃未接纳的业务事件，每个有界接收批次结束后让出一次调度，不逐个丢弃包等待消费者。签名验证继续受独立的已认证 peer／全局控制速率预算限制。

Manager→SecureNode owner 的 circuit 控制／诊断帧和 lifecycle 事件另有 32 个保留槽。帧头只作调度提示，实际认证、路径与许可检查仍由 SecureNode 执行。`recv_batch_with_data(false)` 暂停业务事件消费，继续交付控制事件；换代／停止同时清理两条队列。该分流不会让已停止消费的业务队列无限增长，也不承诺在共享 socket 的任意业务洪泛下完全避免控制丢包。

发送仍由同一实际 socket 的正式 Session owner 执行。Manager 在已有发送上限内保留两帧、两个操作和 4 KiB，业务 `submit_batch` 不能消耗该控制预留；控制逐帧入口继续使用原 scope、截止与完成检查，不增加总资源上限。健康期限仍只由匹配本机新鲜 nonce 的认证 Pong 更新，业务包或远端 Ping 不延长健康窗口；间隔与失效规则见[邻接管理](/v2/node/transport#邻接管理)。

## 路径选择

普通拨号、反向拨号、协调 UDP 打洞、中继和真正直连分别报告。双方认证成功、真实双向往返与实际承载路径一起构成切换依据；STUN 返回值或已发送探测包不构成成功证明。

打洞失败时仅可使用已允许的其他连接／中继路径。例如逻辑路径要求 A→R→B，A 与 B 的候选即使能直连，也不能绕过 R。新通道和旧通道竞争时，由同一连接 owner 完成选择，业务流保持其原完整路径。

穿透、NAT 与 TCP 退路的验收状态见[验收范围](/v2/node/validation)。

## 规划：MASQUE

MASQUE 尚未实现。后续优先评估 CONNECT-UDP 与现有 datagram 会话的衔接，再决定 CONNECT-IP 的必要范围；HTTP/3 DATAGRAM 与 TCP 上的 Capsule 承载分别报告，经代理传输不标记为 P2P 直连。HTTP/3 不承诺与浏览器流量不可区分，代理／CDN 的支持需真实互通验证；QUIC 或代理 TLS 不替代 Mesh 的端到端保护与业务授权。

参考：[RFC 8489](https://www.rfc-editor.org/rfc/rfc8489)、[RFC 9297](https://www.rfc-editor.org/rfc/rfc9297)、[RFC 9298](https://www.rfc-editor.org/rfc/rfc9298)、[RFC 9484](https://www.rfc-editor.org/rfc/rfc9484)。
