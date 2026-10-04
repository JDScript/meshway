# 数据面、TUN 与 DNS 组件

当前库提供策略执行、IP 包处理、DNS 查询生命周期、Linux TUN I/O、资源归属保护和候选受保护 Mesh circuit。Linux daemon 已通过关闭系统 DNS 的 Mesh-only Native TCP 承载验收，包含内层 IPv4／IPv6 UDP／TCP、原路径回复、持续续租、撤销与失效清理。安装门槛见[网络模式](/v2/node/runtime#安装范围与状态)，精确验证入口与范围见本页下文。

## 策略与包准入

`meshway-policy` 是 Access、主体匹配、routing、topology 和编译规则的实现归属。Server 对可信完整输入编译；Node 使用自己的 `NodeConfiguration`／`PathPermit`，通过 `admission::admit_forward`／`admit_reply` 检查业务元数据与完整路径。

执行发布中的 `mesh_hosts` 也按该节点作为请求源的 Access 裁剪。拥有中继／回复资格、获知路径 peer 的地址，不自动获得其 MagicDNS 名称可见性；预需求发现仍通过精确名称接口完成。

判定检查当前 permit、业务源／目的地址、协议／端口、完整路径、实际相邻 peer 和出口资格。回复必须对应数据面已有的正向流，并沿原完整路径反向运行。规则语义分别见 [Access](/v2/configuration-reference/access) 和 [Routing](/v2/configuration-reference/routing)；NodeID 或 pure predicate 的调用不证明身份认证。

`meshway-dataplane::Engine` 持有一个[执行快照](/v2/node/control#执行快照)和唯一有界流表。`Packet::parse(Vec<u8>)` 验证包并保留原始 buffer；`process` 产生 `PendingAction`。宿主必须在最终同步发送／交付边界调用其 `release`，等待期间保留 wrapper 并监听 `invalidated()`。

`Ingress::Peer` 的 original origin、packet producer、previous peer 必须由受信身份层提供。网络入口使用下节的 `SecureNode::receive`，验证后进入同一流表；不能把未经认证的 transport payload 直接包装成明文入口输入。

### 包与流边界

| 项目 | 当前范围 |
| --- | --- |
| IPv4 | 精确长度与 checksum；暂不支持 options 或分片 |
| IPv6 | 精确长度；最多两个顺序合法、仅含 padding 的扩展头；拒绝 routing header、全部 fragment header 和 jumbogram |
| 业务协议 | TCP、UDP、ICMP echo；检查端口、协议头及适用 checksum |
| UDP／ICMP | 回复匹配已建立反向 tuple；echo 另绑定 identifier／sequence |
| TCP | 三次握手、连续新数据、每方向 16 MiB 有界序号历史内的 ACK／重分段重传、FIN 与已建立 RST；常量内存，支持序号回绕。尚非完整 conntrack，不缓存乱序新数据，不支持 simultaneous-open、TFO 或握手 RST |
| 资源 | 默认 4,096 流、60 秒空闲；最大 65,536 流、24 小时空闲设置；实际寿命仍受 permit 和执行快照约束 |

同一流固定原 demand ID 与完整路径；同一节点／角色范围的活 tuple 在多个 demand ID 之间也不能被另一条路径接管。`LocalFlow` 只读查询同一 FlowTable，供本机源包／原路径回复使用，不刷新流、推进协议状态或形成第二份路由表。中继流不能借作本机发起资格；歧义拒绝。

`Engine::reconcile` 使用 Control 的[直接连续性交接](/v2/node/control#执行快照)，仅保留原授权与完整路径仍相同的活流。比较使用排除 `expiresAt` 的 `permit_authorization_digest`，同时检查路径相关身份、地址与 key 绑定。换 instance、撤回、取消、跳过 publication 或旧期限已到都不迁移状态。

交接本身保守保留流的旧 idle／permit／快照截止，不凭读状态延长流。只有旧流仍有效时，实际获准包通过同一协议状态机才能刷新其期限；已排队输出继续受原截止与 guard 约束。拒绝包不建立或推进流状态。

## 候选受保护 circuit

`meshway-dataplane::secure::SecureNode` 是同一执行快照下的 circuit 和流状态 owner。`start` 发起握手，`receive` 验证承载帧，`send` 接收本机 IP 包；它们使用相同的 `Engine`／`meshway-policy`，中转使用通过认证的 `PacketFacts`，不构造伪造明文包来调用另一套转发实现。

| 边界 | 当前实现 |
| --- | --- |
| 端到端身份／加密 | Snow 的 `Noise_IK_25519_ChaChaPoly_SHA256`，静态 X25519 公钥取自本机 Server 执行配置；每次握手使用新 ephemeral |
| 握手绑定 | network、demand、完整有序 path、随机 circuit ID、permit 授权摘要（排除 expiresAt）、路径上全部控制／数据面 key ID、generation 和公钥 |
| 早期数据 | 两个 Noise 握手消息的业务 payload 必须为空；握手不创建业务流 |
| 路径证明 | 按顺序的 Ed25519 逐跳证明，绑定原始 header、ciphertext、此前证明和当前 carrier；接收端验证全部证明 |
| 承载约束 | carrier 包含实际 dialer、listener、listener key 与协议；验证该连接在所选完整路径的候选集合内，拨号方向与业务方向分别处理 |
| 重放 | 每 circuit／方向独立的 64-bit nonce 和 1,024 包窗口；nonce 不回绕，只有通过认证与准入的包推进接收窗口 |
| 生命周期 | circuit、流、permit 与 Control 截止取交集；关闭、销毁、失效同时取消排队密文和待交付明文 |

**隐私范围：**IP 包通过 Noise 在业务两端之间加密。为执行中转检查，外层同时携带签名保护的五元组、有限 TCP／echo 状态和长度；这些元数据不具备机密性。中继不解密业务 payload，目标端解密后必须把真实包解析结果与签名元数据精确比较。

密钥通过 `Control::dataplane_identity` 从有效执行快照取得。包平面签名有独立的 purpose／network／node／key generation transcript，不开放任意控制认证消息签名。该句柄是受信进程内接口；密钥不进入 Debug 或序列化，不能视为恶意插件的内存隔离。

当前 wire 标识为 `MWCIR\0\0\x02`，包平面 proof purpose 为 2，是需继续测量的候选格式，不是稳定插件 ABI。UUID 编码为固定 36-byte 规范 ASCII；数值采用大端。最多 8 个 path 节点、128 KiB 帧、65,519-byte 内层包、255-byte listener key；默认最多 1,024 个 circuit 和 10 秒握手期限。关闭／失败 circuit 的重放 tombstone 在该 SecureNode owner 生命周期内保留并计入 maxCircuits；permit 过期或同授权重新授予都不清除，耗尽则拒绝新 circuit。重启重新握手，不能恢复旧 cipher state。

`SecureNode::reconcile` 在相同授权、原路径和有效直接交接下保留 Noise 状态、nonce、重放窗口与同一流表。显式授权续租可在旧 UTC／单调期限到达前延长活 circuit 的期限；已关闭／过期 circuit 不复活，握手期限也不由续租重置。已签发输出保留旧 guard；流截止按上节处理。当前逐跳签名的 CPU／带宽成本尚未完成性能验收。

`carrier::send` 把完整 I/O lifetime 随已接纳工作交给正式 transport worker；即使完成等待者暂不被轮询，每次底层 write／flush 也检查同一许可期限和失效条件。`carrier::deliver` 在每次恢复 TUN 写入前检查所有限制。完成只表示本机写入完成，对端接收／应用回包另行证明。

## Linux TUN

`meshway-tun::Tun::create(Options)` 在宿主已经选择的 network namespace 中创建内核自动命名、非持久的 L3 TUN。安全 Rust 层使用固定版本 tun-rs；不接管已有名称，不导出 FD，drop 由内核回收设备。

`Options::new(mtu)` 接受 1280–65535 bytes，可为新设备设置 IPv4／IPv6 地址及前缀。地址带来的本地／直连路由由该 namespace 的内核产生。创建需要已启用 I/O 的 Tokio runtime、既有 `/dev/net/tun` 和相应设备权限。

`recv` 一次读取一个 IP 包，缓冲小于 MTU 时在读取前拒绝；`send` 一次写入一个包，短写报错，不续写为第二个报文。当前关闭 offload、multiqueue 和 packet-information header。外部修改该设备 MTU／offload 不受支持；IPv6 DAD 就绪由宿主等待。

### Linux underlay

`meshway-underlay::Host` 是一个 namespace 内本节点 capture／socket 保护的唯一 owner；daemon 驱动其 install／suspend／down／shutdown。Host 独占私有目录锁与 journal，使用新建的 Node-owned TUN、IPv4 `/32`／IPv6 `/128` 地址，默认 MTU 1280。已有 TUN 不接管。

全部发布地址池都进入拒绝捕获边界；只有本机具有对应地址族的已分配地址时，才安装该族的正向 TUN 路由。缺少某族地址不会解除该族的拒绝边界；无本机地址的中转不据此获得本机发起业务资格。

capture 使用执行配置中完整的 `meshAddressPools`，其[契约](/v2/configuration-reference/node-configuration#capture-与本机-egress)不授予 Access。私有 route table 只包含所属 capture 路由；RPDB 安装 own-mark goto、各池 lookup／unreachable 和 end，受保护 socket 跳到此块末尾后继续原 RPDB，而非强制走 main table。SO_MARK 选择、保留已有 selector 语义与读回验证必须成功；同 UID 其他 socket 不获全局绕过。

| 生命周期 | 处理 |
| --- | --- |
| 安装／直接新鲜交接 | 验证目标与资源归属；相同 capture 可保留 TUN FD，重新绑定有效快照 |
| 期望 up 时过期／断连／shutdown | 停止正向 I/O，保留历次受管池并集的负向 capture，直到显式 down |
| 显式 down | 停止／join 所属任务，清理可证明所属的规则／路由与 TUN |
| 外部修改或恢复冲突 | 停止正向 I/O 并保留 journal／负向边界，不 flush 外部资源 |

journal 绑定 namespace／boot 身份，记录有界资源及待完成操作。未知归属、未知／未解决操作、身份不匹配阻断恢复。Linux rule／route 没有 incarnation cookie 或 compare-and-delete：特权主体用完全相同对象替换的 ABA 无法证明，资源归属要求外部不并发修改这些对象。

`Options` 默认上限为 64 个池、128 个受保护 socket、16,384 个内核对象，连接超时 10 秒；一部分 socket 预算为 Server bootstrap 保留。网络安装成功不自动表示 DNS、邻接或应用包流成功。

#### Bootstrap 与普通业务隔离

Server 与获准候选的 bootstrap 只开放用途受限 socket。Server HTTP 使用固定 origin 的私有 UDS relay 接入 reqwest，HTTPS 仍执行正常系统根 TLS 校验；不会为同 IP 的普通应用流量创建全局豁免。候选 handle 绑定已发布的 peer／listener／endpoint 和有效执行快照。

bootstrap 名称可使用[启动参数](/v2/node/runtime#启动与接入)的显式 hosts 或真实 nameserver，也读取系统 hosts／真实上游；已知 systemd-resolved stub 可从其上游文件取得真实服务器。未知保护关系的本地 DNS stub 不支持；须提供所需名称的 hosts 地址或显式真实上游。解析失败不回退公共 DNS。宿主系统 DNS、业务 resolver socket 和出口 resolver 不由这些 bootstrap 权限代替。

### TUN 验证入口

显式 Linux 验证入口：

```sh
mise run test:tun:linux
```

脚本在宿主交叉编译，容器仅运行预编译测试。使用固定 digest 镜像、独立 network namespace、单独的 TUN device／NET_ADMIN 权限和资源预算，输出二进制哈希、源码指纹及测试报告到仓库私有 `.meshway/validation/tun/`。常规检查不隐式申请设备权限。

### 受保护包流 conformance

```sh
mise run test:packet:linux
```

入口在宿主编译 `meshway-packet-fixture`，使用 Controller、A、R、B 四个独立容器／network namespace 和一个隔离 internal bridge。Controller 使用真实 Server Router／HTTPS／生成管理客户端；各 Node 独立持钥接入、取得执行快照并通过正式 Native TCP provider 连接。A／B 的 OS UDP 应用通过各自 TUN 验证 IPv4／IPv6 echo，R 只转发受保护帧，回复遵循 B→R→A。

各端只接触自己的私有 state；Node 不持有管理 token 或 Controller TLS 私钥。脚本不发布宿主端口，只给 TUN 端点精确的设备／NET_ADMIN 权限，并清理本轮容器和网络。源码指纹、实际二进制、进程结果及各角色报告保存在 `.meshway/validation/packet/`。此入口的既有通过证据属于 v1 内层 UDP／外层 Native TCP；当前 v2 候选须重跑。daemon 安装、其他承载、多路径切换、完整 TCP conntrack、MTU／分片与性能需各自验证。

### daemon 与内核资源 conformance

```sh
mise run test:runtime:linux
```

该入口在宿主编译实际 `meshwayd`、`meshway` 和独立验证 actors。隔离容器中的 Node 通过 CLI 上线和真实 Server 批准；验证程序使用 OS UDP／TCP socket，包处理由 daemon 的正式运行组件执行。

场景配置为关闭系统 DNS 的 Mesh-only 网络，强制 A→R→B 及原路径回复。已通过的 daemon 验收包含双栈 UDP／TCP、同一 socket 跨需求添加与至少 125 秒持续续租、独立 UDP 流停止发包后的入站活动续租、Access 撤销／恢复、中转取消 IPv6 地址后继续转发 IPv6 密文、Server 失联后期限截止、拒绝路由保留及离线 `down` 清理。单独的内核测试检查同目的地址的普通／受保护 socket、旧池拒绝边界、journal 恢复及外部同优先级规则冲突；HTTPS bootstrap 另验显式 down 前后的联系。

容器只运行预编译程序，使用固定镜像、独立 IPv4／IPv6 internal bridge 和精确 NET_ADMIN／TUN 设备权限；不发布宿主端口。源码／二进制指纹、各角色结果和清理证据保存到 `.meshway/validation/runtime/`。入口的通过结果只覆盖其明确配置，不代表 DNS 系统安装、出口转发、其他承载或完整 TCP conntrack 已验收。

## DNS 查询 owner

`meshway-dns` 使用 Hickory 编解码、TCP framing 和 DNS 选择规则。`Owner` 持有同一个执行快照与有限 pending 请求；宿主用唯一 Control driver 执行 `Control::query_dns`，将类型化响应交回 `complete_discovery`。

Ticket 绑定 owner 代际与本次操作；DNS request ID 相同不表示同一任务。完成时检查规范名称、网络／源节点、配置 revision 和可验证的地址绑定。`asOf` 是 Server 查询时点，不以本机与 Server 时钟相等作为条件，也不据此制造 TTL。

当前已发布／发现的 MagicDNS 记录均以 TTL 0 应答，没有正／负缓存，也不新增 TTL 来源。已知名称缺少对应地址族返回 NODATA；精确发现的 `record: null` 保持类型化 `NotVisible` 并编码为 REFUSED，无 AA／SOA，不制造可缓存 NXDOMAIN。部分 `mesh_hosts` 缺席先进行精确发现，不能被当作权威 NXDOMAIN。

`refresh`、`replace`、`stop` 或 owner 销毁取消旧 ticket 与尚未发送的回答；失效 Control 快照同样阻止回答交付。

### Service 与 resolver 路由

`Service` 消费预绑定、地址／端口相同的 loopback UDP socket 与 TCP listener，并校验客户端同为 loopback。两种传输共用一个 `OwnerDispatcher`；默认同时最多 128 查询、32 TCP 连接，查询期限 5 秒、TCP idle 30 秒、UDP payload 上限 1232 bytes。每条 TCP 连接顺序处理查询；超预算 UDP／连接丢弃，停止时取消并 join 服务任务。

`OwnerDispatcher` 接受执行快照与 resolver key → demand ID 的有界映射，使用 `DiscoveryGateway` 调用宿主唯一 Control driver。普通上游仅通过 `ResolverExchange` 消费密封的 `LocalResolverRoute`：其构造重新检查同一 publication 中[精确 localEgress](/v2/configuration-reference/node-configuration#capture-与本机-egress)，绑定源、resolver endpoint、传输、profile、action 和期限。单独的 Direct／Unmanaged decision 不能构造该路由，也不是 bootstrap 豁免。

exchange 适配器须在每次 connect／write／read poll 检查路由期限与取消，验证来源、事务及 framing；UDP 许可不隐含 TCP 重试。scope 更新（包括同目标续租）取消旧 DNS ticket／回答，不把旧查询移到延长后的 scope。服务对 dispatcher 失败给出原查询期限内的空 SERVFAIL；失效正向回答不能改装为负缓存。

当前库已有 Service／OwnerDispatcher 和 resolver 路由验证；生产 ResolverExchange、系统 DNS 保留／安装、出口 resolver 与可信域名关联入口尚未接线。因此 daemon 的 DNS 门槛仍按[安装范围](/v2/node/runtime#安装范围与状态)整体阻断 applied。出口选择及尚未实现的显式 DNS plan 见 [DNS](/v2/configuration-reference/dns#使用出口时)。

宿主 DNS 的已确定安装方向是本机 bind／backend 安装选项：优先使用 systemd-resolved，否则须显式选择并证明所有权的 resolv.conf proxy，不能安装则 failed。该接口及选项尚未实现，不构成可用 CLI 或云端字段。
