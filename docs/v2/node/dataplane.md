# 数据面、TUN 与 DNS 组件

当前库可独立验证策略执行、IP 包处理、DNS 查询生命周期和 Linux TUN I/O。daemon 仍处于控制接入阶段；Mesh 端到端身份握手、加密封装和实际网络转发尚未接线。

## 策略与包准入

`meshway-policy` 是 Access、主体匹配、routing、topology 和编译规则的实现归属。Server 对可信完整输入编译；Node 使用自己的 `NodeConfiguration`／`PathPermit`，通过 `admission::admit_forward`／`admit_reply` 检查业务元数据与完整路径。

执行发布中的 `mesh_hosts` 也按该节点作为请求源的 Access 裁剪。拥有中继／回复资格、获知路径 peer 的地址，不自动获得其 MagicDNS 名称可见性；预需求发现仍通过精确名称接口完成。

判定检查当前 permit、业务源／目的地址、协议／端口、完整路径、实际相邻 peer 和出口资格。回复必须对应数据面已有的正向流，并沿原完整路径反向运行。规则语义分别见 [Access](/v2/configuration-reference/access) 和 [Routing](/v2/configuration-reference/routing)；NodeID 或 pure predicate 的调用不证明身份认证。

`meshway-dataplane::Engine` 持有一个[执行快照](/v2/node/control#执行快照)和唯一有界流表。`Packet::parse(Vec<u8>)` 验证包并保留原始 buffer；`process` 产生 `PendingAction`。宿主必须在最终同步发送／交付边界调用其 `release`，等待期间保留 wrapper 并监听 `invalidated()`。

`Ingress::Peer` 的 original origin、packet producer、previous peer 必须由受信身份层提供。当前明文核心不解析或证明这些身份，不能把未经认证的 transport payload 直接包装成该输入。

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

## Linux TUN

`meshway-tun::Tun::create(Options)` 在宿主已经选择的 network namespace 中创建内核自动命名、非持久的 L3 TUN。安全 Rust 层使用固定版本 tun-rs；不接管已有名称，不导出 FD，drop 由内核回收设备。

`Options::new(mtu)` 接受 1280–65535 bytes，可为新设备设置 IPv4／IPv6 地址及前缀。地址带来的本地／直连路由由该 namespace 的内核产生。创建需要已启用 I/O 的 Tokio runtime、既有 `/dev/net/tun` 和相应设备权限。

`recv` 一次读取一个 IP 包，缓冲小于 MTU 时在读取前拒绝；`send` 一次写入一个包，短写报错，不续写为第二个报文。当前关闭 offload、multiqueue 和 packet-information header。外部修改该设备 MTU／offload 不受支持；IPv6 DAD 就绪由宿主等待。

显式 Linux 验证入口：

```sh
mise run test:tun:linux
```

脚本在宿主交叉编译，容器仅运行预编译测试。使用固定 digest 镜像、独立 network namespace、单独的 TUN device／NET_ADMIN 权限和资源预算，输出二进制哈希、源码指纹及测试报告到仓库私有 `.meshway/validation/tun/`。常规检查不隐式申请设备权限。

## DNS 查询 owner

`meshway-dns` 使用 Hickory 编解码、TCP framing 和 DNS 选择规则。`Owner` 持有同一个执行快照与有限 pending 请求；宿主用唯一 Control driver 执行 `Control::query_dns`，将类型化响应交回 `complete_discovery`。

Ticket 绑定 owner 代际与本次操作；DNS request ID 相同不表示同一任务。完成时检查规范名称、网络／源节点、配置 revision 和可验证的地址绑定。`asOf` 是 Server 查询时点，不以本机与 Server 时钟相等作为条件，也不据此制造 TTL。

当前已发布／发现记录均以 TTL 0 应答，没有正／负缓存。已知名称缺少对应地址族返回 NODATA；未知／不可见保持类型化 `NotVisible`，DNS wire 的负应答规则仍待确定。部分 `mesh_hosts` 缺席不能被当作权威 NXDOMAIN。

`refresh`、`replace`、`stop` 或 owner 销毁取消旧 ticket 与尚未发送的回答；失效 Control 快照同样阻止回答交付。普通 resolver 仍需获准的实际路径，exit DNS plan、上游执行／缓存和可信域名关联生产入口尚待接入；resolver 候选配置不直接授予 socket 或出口权限。
