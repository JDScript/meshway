# 承载与邻接模型

本页定义两个 Node 之间的认证邻接如何建立和使用：谁建立底层连接、连接建立后数据如何流动，以及谁能发起业务流。字段定义见 [Listeners](/v2/configuration-reference/listeners)、[Dialers](/v2/configuration-reference/dialers) 与 [Topology](/v2/configuration-reference/topology)；连接获取的实现与 UDP 穿透契约见[连接获取](/v2/node/connectivity)，会话、帧与发送语义见[承载会话](/v2/node/transport)。

## 结构

### 三个方向

一条邻接有三个互相独立的方向：

| 方向 | 决定者 | 含义 |
| --- | --- | --- |
| 拨号方向 | 双方允许的承载交集、listener 与已解析的拨号候选 | 哪个节点建立底层连接（TCP 连接、TLS／WebSocket 握手或 UDP 首个握手报文） |
| 数据方向 | 邻接会话本身 | 认证邻接建立后总是双向收发，不区分拨号方与接受方 |
| 业务方向 | [Access](/v2/configuration-reference/access) 与 [Routing](/v2/configuration-reference/routing) | 哪个节点可以发起新的业务流；回复只沿已建立流的原完整路径返回 |

拨号方向不授予业务资格，业务方向也不要求业务发起方是拨号方。认证邻接的握手在 Carrier 中记录实际拨号方、监听方、listener key 和协议，见[邻接管理](/v2/node/transport#邻接管理)。

### 两类建连机制

Node 用两种机制获得底层连接。两者交付的是同一种认证邻接会话：

| | 固定端点 | 协调端点 |
| --- | --- | --- |
| 配置来源 | 对端的 listener：固定 bind、可选额外 `endpoints` 与 Node 上报的本机地址 | 一方的 Native UDP listener（可不公告 `endpoints`）与另一方含 `native_udp` 的 dialers，加上 State 中的候选 |
| 适用承载 | Native UDP、Native TCP、VLESS、Trojan | 仅 Native UDP |
| 可连接的对端 | 任何当前获准的 peer，可随时连接 | 仅本次尝试中协调的对端 |
| 目标地址 | 已发布执行配置中的 `configured`／`local` endpoints | STUN 观测与候选交换产生的候选对 |
| 有效期 | 跟随 listener 资源与执行许可 | 每次尝试有期限（最长 10 秒），过期、撤回或取消后失效 |
| 发起方式 | 拨号方连接，监听方接受 | 经 Server 协调后双方从实际 socket 同时互探 |

两类机制共用以下 owner 与规则，不分别维护：

- peer 身份认证与邻接握手；
- 每个 peer 一条选定通道的选链、健康检查与重连；
- 执行 lease 与逐操作 scope；
- 唯一业务流表、端到端保护与数据面。

协调尝试取得的通道与固定端点通道同等接受当前执行许可、健康检查和数据面检查，不因来源不同而获得额外权限。

### 按承载类型的建连需求

| 承载 | 建连机制 | 拨号方 | 说明 |
| --- | --- | --- | --- |
| Native UDP | 固定端点或协调穿透 | 任一方均可：固定端点由有候选的一方拨号；协调穿透由逻辑拨号方准备尝试，双方同时互探 | 协调尝试同样引用一个 Native UDP listener key 作为 Carrier，需双方当前获准的 Native UDP 邻接，范围见[连接获取](/v2/node/connectivity#stun-与-server-协调) |
| Native TCP | 固定端点 | 必须一方 listener、另一方拨号 | 可反向拨号；不做 TCP simultaneous-open |
| VLESS（TCP/TLS、WS/TLS） | 固定端点 | 同上 | TLS／VLESS 的 client/server 角色与拨号方向绑定 |
| Trojan（TCP/TLS） | 固定端点 | 同上 | 同上 |
| MASQUE | 规划中 | 需协议 client/server 角色 | 见[规划：MASQUE](/v2/node/connectivity#规划-masque) |

### 为什么 UDP socket 也在“监听”

Native UDP 的 socket 在操作系统层都是已绑定、持续接收报文的 socket。仅拨号的节点为协调穿透绑定的候选 socket 同样会接收来自对端的报文，但它不是配置意义上的 listener：

- 它没有公告 endpoints，不出现在执行配置的 listener 集合中，其他 peer 无法据配置找到它；
- 它只接受与本次尝试候选对、nonce 和期限相符的对端握手；
- NAT 映射与防火墙放行只是本次尝试的副产物，不构成固定入口。

一个 Native UDP listener socket 可以同时服务两类机制：配置或本机上报的地址可作为端点被拨入，开启协调时也作为该节点参与穿透的实际 socket。未配置额外 `endpoints` 不影响本机地址上报；协调 socket 不会因此变成 listener。

## 字段

本页不定义新字段。承载约束由以下现有字段共同表达：

| 字段 | 位置 | 对承载的作用 |
| --- | --- | --- |
| `topology[].transports` | 网络配置 | 无方向边允许的承载及其尝试顺序；省略用平台顺序，匹配与交集见 [Topology](/v2/configuration-reference/topology#承载交集) |
| `nodes[id].listeners` | 节点配置 | 该节点作为监听方提供的固定端点及其协议 |
| `nodes[id].dialers` | 节点配置 | 该节点主动拨号的协议能力与进阶选项，不承载策略；省略时继承 `defaults.dialers`，`{}` 禁用主动建连 |
| 执行配置 `dialers` | NodeConfiguration | Server 已解析的具体拨号候选，key 为 `<peer>/<listener-key>`，值为 [DialCandidate](/v2/configuration-reference/node-configuration) |
| 执行配置 path `dialCandidates` | NodeConfiguration 许可 | 每条完整路径上按实际拨号方分组的候选 key；监听方据此验证拨入的 Carrier |

DialCandidate 包含 `peer`、`listenerKey`、`endpoints`、由 Server 按候选协议在边级承载顺序中的位置编译的 `priority` 与已解析的协议参数 `parameters`（目标 UUID／password、TLS 验证材料、WS path／Host 等）；排序规则见 [Topology 承载交集](/v2/configuration-reference/topology#承载交集)。每个执行 endpoint 包含 `host`、`port` 与 `source`（`configured`／`local`）。监听方只取得完整监听参数，拨号方不取得对端私钥。

## 行为

### 承载交集

对路径上的每条边 A–B，Server 分别计算 A 拨 B 与 B 拨 A 的候选。一个候选需要同时满足：

1. [Topology](/v2/configuration-reference/topology#承载交集) 允许该无方向边及该协议，且拨号方 `dialers`（或继承的默认集合）启用该协议；
2. 监听方存在已启用的同协议 listener，VLESS 的 stream 与 dialer 设置相容；
3. 拨号方具有该协议的 Dial 能力，监听方具有 Listen 能力；
4. TLS 版本范围有交集（VLESS／Trojan）。

两个方向都没有候选时，这条边不能进入完整路径。Node 的 Manager 只在发布的候选内选择，端点来源与边级承载顺序的排序规则见 [Topology 承载交集](/v2/configuration-reference/topology#承载交集)。不存在于候选中的协议即使双方都已编译该 provider 也不会被使用。

### 拨号与接受

Node 应用执行配置后，Manager 为执行配置 `dialers` 中出现的每个 peer 启动一个拨号 owner。拨号是**急切**的：配置生效即开始，不等待首个业务包；该 peer 已有选定通道时，拨号 owner 只做退避等待，失联后重新按候选顺序拨号。

- 拨号只使用 Server 执行发布的 `endpoints`，按 [Topology 候选排序规则](/v2/configuration-reference/topology#承载交集) 尝试；每个 listener 最多八个本机地址。Native UDP 另保留协调穿透。候选均为空时流式承载等待后续发布。
- 本机地址变化经既有 State 上报、Server 发布与 SSE／刷新路径更新 peer 候选；不需要管理员修改配置。State 只补充拨号地址，不创建 listener、不改变 Access／Topology／Routing 或边级承载交集。
- 已启用的 listener 在执行配置生效时创建，接受获准 peer 拨入。监听方用本节点许可中路径的 `dialCandidates` 校验拨入的 Carrier，不在本机保存对端的拨号参数。
- 每个 peer 同时只有一条选定通道。已有不同协议的健康通道时，新建立的通道不替换它。

### 反向拨号与发布协调

业务发起方不能拨号时，由对端拨号。设 LAX 上的业务需要访问只允许 VLESS 且没有可达 listener 的 CN：

1. LAX 产生需求，Server 检查 Access、Topology 与 Routing，计算完整路径 LAX→CN。
2. 这条边上只存在 CN 拨 LAX 的 VLESS 候选。Server 把 LAX 与 CN 都作为参与节点，向两者发布相容的执行配置。
3. CN 收到的 `dialers` 含该候选：peer 为 LAX、listener key、VLESS 参数与 LAX 的公告 endpoints。CN 的 Manager 按发布急切建立连接。
4. LAX 只接受拨入；认证邻接建立后，LAX 的业务包经这条由 CN 建立的会话发出，CN 的回复沿同一会话返回。

Node 侧没有第二套“请对端来连”的信令，**发布就是协调**：对端是否拨号，完全取决于它是否应用了含该候选的执行配置。CN 尚未应用相容配置时，LAX 的业务按[执行许可](/v2/execution#许可与-state)等待或拒绝，不退化到其他承载，也不由 LAX 自行构造拨号计划。

### 原路径回复

回复不重新做反向路由或反向选择承载。数据面只接受与已建立正向流匹配的回复，并沿原完整路径反向运行，见[数据面](/v2/node/dataplane)；每一跳都使用该邻接当前的选定通道。由于每个 peer 只有一条选定通道，回复与正向包共用同一条认证会话。

## 示例

### CN→LAX 只允许 VLESS

CN–LAX 边只允许 VLESS，LAX 提供 VLESS listener；CN 可对其他边使用 Native UDP。Access 允许 CN 访问 LAX 的 `tcp:443`。

| 方向 | 本例取值 |
| --- | --- |
| 拨号方向 | CN 拨 LAX（VLESS over WebSocket/TLS） |
| 数据方向 | 建立后双向 |
| 业务方向 | CN 发起；LAX 只能回复，除非 Access 另行允许 LAX→CN（例如 `bidirectional: true`） |

配置节选：

```json
{
  "topology": [
    {
      "remark": "允许 CN 与 LAX 直接邻接",
      "action": "allow",
      "a": "node:<cn-id>",
      "b": "node:<lax-id>",
      "transports": ["vless"]
    }
  ],
  "nodes": {
    "<cn-id>": {
      "dialers": { "native_udp": {}, "vless": {} },
      "listeners": {}
    },
    "<lax-id>": {
      "listeners": {
        "edge": {
          "type": "vless",
          "listen": "0.0.0.0",
          "listenPort": 443,
          "endpoints": [{ "host": "lax.example.com", "port": 443 }],
          "stream": "websocket",
          "tls": { "serverName": "lax.example.com" },
          "websocket": { "path": "/meshway", "host": "lax.example.com" }
        }
      }
    }
  }
}
```

某条边只允许某承载，需要三个要素同时成立：

1. `topology` 允许这对节点直接邻接，`transports: ["vless"]` 限定该边只用 VLESS；
2. 一方（LAX）有该协议的 listener；
3. 另一方（CN）的 `dialers` 启用该协议。本例 CN 没有 listener，因而只有 CN 拨 LAX 的方向；即使节点级 dialers 还启用其他协议，也不能超出本边的 `transports`。

编译结果：CN 拨 LAX 只有 `<lax-id>/edge` 一个 VLESS 候选；LAX 拨 CN 没有候选，因为 CN 没有 listener。CN 应用配置后立即建立 VLESS 会话。

CN 的业务包经这条 TCP/TLS 会话到达 LAX。LAX 的回复沿同一条 VLESS 会话原路返回，不会变成 LAX→CN 的 UDP，原因是：

- 回复遵守原路径回复规则，复用该邻接的选定通道，不重新选路或选承载；
- Manager 只在双方允许的承载交集中选择，交集里只有 VLESS，不存在 UDP 候选；
- LAX 没有指向 CN 的候选，不会发起拨号。

会话断开后，重连仍由 CN 按同一候选发起。

## 限制

- **边级许可不创造能力**：`topology[].transports` 只收窄双方声明的交集，不创建 listener、启用 dialer 或提供网络可达性；字段与匹配语义见 [Topology](/v2/configuration-reference/topology#承载交集)。
- **穿透只限 UDP**：协调端点只用于 Native UDP。Native TCP／VLESS／Trojan 必须一方提供 listener、另一方拨号，不实现 TCP simultaneous-open；双方都没有可达 listener 时，这条边不能使用 TCP 系承载。
- **MASQUE 未实现**：MASQUE／HTTP/3 需要明确的 client/server 角色，属于规划项，见[规划：MASQUE](/v2/node/connectivity#规划-masque)。
- **验收状态**：VLESS／Trojan 各 stream 类型上的 daemon 整组包流验收状态以[验收范围](/v2/node/validation)为准。
- **双向承载不等于双向业务授权**：邻接会话双向收发只说明数据可以双向流动；新业务流的发起资格仍由 Access 和 Routing 判定，回复只匹配已建立的正向流。
