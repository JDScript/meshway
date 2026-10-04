# 数据面、TUN 与 DNS 组件

当前库提供策略执行、IP 包处理、DNS 查询生命周期、Linux TUN I/O，以及候选的受保护 Mesh circuit。daemon 仍处于控制接入阶段；网络安装与持续连接调度尚未接线。独立 conformance 程序使用同一实现验证受保护包流。

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
| TCP | 保守握手／顺序检查、最近段重传、FIN 与已建立 RST；尚非完整 conntrack，不支持乱序、simultaneous-open、TFO、握手 RST |
| 资源 | 默认 4,096 流、60 秒空闲；最大 65,536 流、24 小时空闲设置；实际寿命仍受 permit 和执行快照约束 |

同一流固定原完整路径。`Engine::refresh` 只接纳仍有效的同目标续租快照，保守保留旧流截止；已排队输出继续受原截止约束。换目标／instance 时创建新 Engine，旧流不迁移。拒绝包不建立或推进流状态。

## 候选受保护 circuit

`meshway-dataplane::secure::SecureNode` 是同一执行快照下的 circuit 和流状态 owner。`start` 发起握手，`receive` 验证承载帧，`send` 接收本机 IP 包；它们使用相同的 `Engine`／`meshway-policy`，中转使用通过认证的 `PacketFacts`，不构造伪造明文包来调用另一套转发实现。

| 边界 | 当前实现 |
| --- | --- |
| 端到端身份／加密 | Snow 的 `Noise_IK_25519_ChaChaPoly_SHA256`，静态 X25519 公钥取自本机 Server 执行配置；每次握手使用新 ephemeral |
| 握手绑定 | network、demand、完整有序 path、随机 circuit ID、精确 permit 摘要、路径上全部控制／数据面 key ID、generation 和公钥 |
| 早期数据 | 两个 Noise 握手消息的业务 payload 必须为空；握手不创建业务流 |
| 路径证明 | 按顺序的 Ed25519 逐跳证明，绑定原始 header、ciphertext、此前证明和当前 carrier；接收端验证全部证明 |
| 承载约束 | carrier 包含实际 dialer、listener、listener key 与协议；验证该连接在所选完整路径的候选集合内，拨号方向与业务方向分别处理 |
| 重放 | 每 circuit／方向独立的 64-bit nonce 和 1,024 包窗口；nonce 不回绕，只有通过认证与准入的包推进接收窗口 |
| 生命周期 | circuit、流、permit 与 Control 截止取交集；关闭、销毁、失效同时取消排队密文和待交付明文 |

**隐私范围：**IP 包通过 Noise 在业务两端之间加密。为执行中转检查，外层同时携带签名保护的五元组、有限 TCP／echo 状态和长度；这些元数据不具备机密性。中继不解密业务 payload，目标端解密后必须把真实包解析结果与签名元数据精确比较。

密钥通过 `Control::dataplane_identity` 从有效执行快照取得。包平面签名有独立的 purpose／network／node／key generation transcript，不开放任意控制认证消息签名。该句柄是受信进程内接口；密钥不进入 Debug 或序列化，不能视为恶意插件的内存隔离。

当前 wire 标识为 `MWCIR\0\0\x01`，是需继续测量的候选格式，不是稳定插件 ABI。UUID 编码为固定 36-byte 规范 ASCII；数值采用大端。最多 8 个 path 节点、128 KiB 帧、65,519-byte 内层包、255-byte listener key；默认最多 1,024 个 circuit 和 10 秒握手期限。关闭记录作为重放 tombstone 保留至对应 permit 过期，并计入容量。重启重新握手，不能恢复旧 cipher state。

同执行目标续租可更新 owner 的有效快照，旧 circuit 和已排队输出保持原截止；新建 circuit 使用新握手与 nonce 空间。当前逐跳签名优先建立可验证的路径保证，其 CPU／带宽成本尚未完成性能验收。

`carrier::send` 把完整 I/O lifetime 随已接纳工作交给正式 transport worker；即使完成等待者暂不被轮询，每次底层 write／flush 也检查同一许可期限和失效条件。`carrier::deliver` 在每次恢复 TUN 写入前检查所有限制。完成只表示本机写入完成，对端接收／应用回包另行证明。

## Linux TUN

`meshway-tun::Tun::create(Options)` 在宿主已经选择的 network namespace 中创建内核自动命名、非持久的 L3 TUN。安全 Rust 层使用固定版本 tun-rs；不接管已有名称，不导出 FD，drop 由内核回收设备。

`Options::new(mtu)` 接受 1280–65535 bytes，可为新设备设置 IPv4／IPv6 地址及前缀。地址带来的本地／直连路由由该 namespace 的内核产生。创建需要已启用 I/O 的 Tokio runtime、既有 `/dev/net/tun` 和相应设备权限。

`recv` 一次读取一个 IP 包，缓冲小于 MTU 时在读取前拒绝；`send` 一次写入一个包，短写报错，不续写为第二个报文。当前关闭 offload、multiqueue 和 packet-information header。外部修改该设备 MTU／offload 不受支持；IPv6 DAD 就绪由宿主等待。

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

各端只接触自己的私有 state；Node 不持有管理 token 或 Controller TLS 私钥。脚本不发布宿主端口，只给 TUN 端点精确的设备／NET_ADMIN 权限，并清理本轮容器和网络。源码指纹、实际二进制、进程结果及各角色报告保存在 `.meshway/validation/packet/`。此入口验证内层 UDP 和外层 Native TCP；daemon 安装、其他承载、多路径切换、完整 TCP conntrack、MTU／分片与性能需各自验证。

## DNS 查询 owner

`meshway-dns` 使用 Hickory 编解码、TCP framing 和 DNS 选择规则。`Owner` 持有同一个执行快照与有限 pending 请求；宿主用唯一 Control driver 执行 `Control::query_dns`，将类型化响应交回 `complete_discovery`。

Ticket 绑定 owner 代际与本次操作；DNS request ID 相同不表示同一任务。完成时检查规范名称、网络／源节点、配置 revision 和可验证的地址绑定。`asOf` 是 Server 查询时点，不以本机与 Server 时钟相等作为条件，也不据此制造 TTL。

当前已发布／发现记录均以 TTL 0 应答，没有正／负缓存。已知名称缺少对应地址族返回 NODATA；未知／不可见保持类型化 `NotVisible`，DNS wire 的负应答规则仍待确定。部分 `mesh_hosts` 缺席不能被当作权威 NXDOMAIN。

`refresh`、`replace`、`stop` 或 owner 销毁取消旧 ticket 与尚未发送的回答；失效 Control 快照同样阻止回答交付。普通 resolver 仍需获准的实际路径，exit DNS plan、上游执行／缓存和可信域名关联生产入口尚待接入；resolver 候选配置不直接授予 socket 或出口权限。
