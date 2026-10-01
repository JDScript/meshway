# 托管 Mesh IPv4 数据面

本切片把已注册节点的身份、路由策略和独立业务授权接入 Linux TUN。应用访问目标节点的 Mesh IPv4，Agent 按有序路径转发端到端密文；中间节点只处理获准路径的密文。

本页描述**当前旧版策略 / 显式启动的实现与验收**。已确认目标改为连接图、叠加 grants、Mesh 内具体度路由和出口路由方案，全部业务配置由平台下发；下方 first-match / reject / 本地 profiles 仅为迁移前现状。出口与域名分流是首版待交付闭环，见[产品概念](/concepts)。

## 三项条件分别满足

以 `NYC → LAX → SHA` 为例：

1. **连接配置**建立 NYC–LAX、LAX–SHA 的认证邻接。SHA 可以主动连接 LAX；LAX 复用该双向通道转发。
2. **路由策略**对 NYC 到 SHA 的 Mesh IPv4 选出有序路径 `NYC → LAX → SHA`。
3. **访问授权**允许 NYC 访问 SHA，或由双向规则同时允许往返方向。

控制面将端点授权和路由结果联合编译为内部精确路径，再分发到沿途节点。`direct` 本身不授予访问权限；Access 的 `allow` 也不改写 Routing 的 reject 或 `via`。缺少通道时失败，不换逻辑路径、不跳过 LAX，也不降低加密模式。

当前管理员授权模型支持 `*` / node / tag、默认 allow/reject、有序 first-match 和显式双向规则，详见[端点访问授权](/access-policy)。内部路径仍包含源节点、必经节点和目标节点且不得重复；它授予两个端点 Mesh IPv4 之间的包访问，不是互联网出口、子网路由或端口级 ACL。

管理员在 `/access` 保存授权，使用与路由相同的全局 revision CAS。安装业务路径前，参与节点须具备不可替换绑定的公钥。撤销导致显式节点引用无效时保留源文档供修复，生效授权整体为空；通配符和标签则按活动成员更新。Agent 的 `getAgentDataplane` 接口一次读取一致的配置、连接、内部授权和公钥，不向 Web 暴露节点凭据或本地私钥。

### 回程也是一个方向

当前不是状态型防火墙：ICMP echo reply、TCP 回包和反向主动访问使用同一套方向检查。要让上述 `ping` 或 HTTP 请求完成，还需要：

```text
SHA → LAX → NYC
```

回程需要路由，并由双向 Access 规则、反向单向规则或默认 allow 授权。双向物理通道不代表双向业务许可；打开双向授权开关也不会自动修改路由规则。

## 哪些路由规则执行

| 条件 | 托管 IPv4 数据面 |
| --- | --- |
| 来源节点、来源国家 | 使用控制面身份及管理员维护的国家元数据 |
| 目标节点、目标国家、IPv4 CIDR | 使用已注册节点的真实 Mesh IPv4 和清单 |
| 有序 first-match、`via`、`direct/reject` | 与 Rust 策略编译器一致，并与独立授权取交集 |
| 域名 suffix | TUN 包不提供可信域名；影响当前 Mesh 流量且无法确定的条件拒绝执行 |
| 互联网目标、独立出口 | 本切片未接通 |
| IPv6、子网转发 | 本切片未接通 |

不能把域名条件简单忽略后继续匹配一个更宽松的 allow/default-direct；可信 DNS 上下文接通之前，这种不确定性必须失败关闭。Web 路由预览仍可处理显式输入的域名，但预览不是业务授权或运行证明。

## 运行边界

Agent 需编译 `managed-connections` feature。在具备 TUN 设备与所需网络权限的 Linux 环境中显式运行：

```sh
meshway-agent run --state /state/agent.json \
  --apply-dataplane --tun-name meshway0 --tun-mtu 1000 \
  --connection-profiles /state/profiles.json \
  --interval-seconds 2 --connection-lease-seconds 15
```

没有 VLESS 配置时不需要 `--connection-profiles`。`1000` 是这个切片的保守内层 MTU 默认值，不是所有路径的通用 PMTU；启动仍须检查完整路径的封装预算。

- 通过显式 `--apply-dataplane` 启用，普通 `--apply-connections` 继续只安装认证邻接。
- 读取同一全局 revision 的节点配置、连接配置、路径授权及公钥，避免混合新旧快照。
- 使用原节点凭据和已绑定私钥；E2E 端点公钥不额外授予直接建连权限。
- 配置撤回、拒绝、有效期结束和代际更换必须阻止旧路径继续发送或注入包。
- 接收方在 E2E 验证后检查完整路径、认证来源及内层源 / 目标 IP，才向 TUN 注入。
- 只接管控制面 Mesh IPv4 网段，transport endpoint、controller 和 DNS 的底层连接不能递归进入自己的 TUN。
- 首个实现不接收 IP 分片。MTU 必须为完整节点 ID、多跳封装和底层 UDP 头留出空间，不能沿用短实验 ID 的 MTU 计算。

配置 ACK 仍只表示持久保存。认证邻接状态不能代替 TUN 安装、业务授权或实际 IP 往返的证明。

## 验收方法

应使用普通工具访问目标 **Mesh IPv4**：

```sh
ping -c 3 <目标 Mesh IPv4>
curl --fail --max-time 5 http://<目标 Mesh IPv4>:<测试服务端口>/
```

服务应绑定目标 Mesh IPv4，避免底层地址或别的默认路由产生假成功。验收同时需要覆盖：

- 有路由但无授权，失败。
- 只有正向授权、无回程授权，往返失败。
- 路由与双向授权一致，ICMP 和 HTTP 成功。
- 更靠前的 `reject` 规则阻止实际包；删除该规则后恢复。
- 失去必经节点时失败，不能自动直连终点。
- 控制面断联超过 lease 后失败；恢复后只能通过新鲜配置重新启用。

普通 `ping lax.mesh.test` 继续测试 Docker 底层网络；`meshway ping demo-lax` 继续测试邻接控制往返。它们与上述 TUN/IP 验收是不同入口。

### 隔离的三节点验收

仓库的 `experiments/managed-dataplane/run.py` 使用新的 controller、真实注册身份、两个数据 bridge 和限定目标的控制面代理。NYC 与 SHA 无法通过 Docker 底层网络直接互访，内核转发关闭，HTTP 测试服务只绑定实际分配的 Mesh IPv4。

```sh
python3 experiments/managed-dataplane/run.py \
  --image meshway-managedtest:access-v2-ubuntu-20261001-a
```

镜像构建与运行方式见该目录 README。脚本使用共享 Docker performance lock，但只验证功能，不给出吞吐基准结论。准备就绪要求配置回执、实际 TUN / 路由、与当前快照 revision / digest 对应的本地运行代际以及新鲜邻接报告；最终成功仍须由实际 ICMP / HTTP 往返证明。

原生 ARM64 镜像 `sha256:fda7fa9b50fe64d41f62d6a72745bc0d501f6236ef6333840e071d1b0e7864eb` 已通过完整验收，运行 ID 为 `mup6m9qh-539ecd0828df0f80`，输出 21 项通过证据且临时资源清理完成。除授权、规则、国家、必经节点、lease、恢复和撤销外，还实际验证了：以暂不存在的域名启动 NYC，保留拒绝路由并持续重试；随后添加对端 DNS alias，在不重启 NYC 的情况下重新取得新鲜配置并恢复 ICMP / HTTP。

实现回归另外覆盖已知新 revision、ACK 409 与不完整 HTTP 403 响应体：Agent 必须及时撤回旧权限，不能等阻塞请求或普通 lease 自行结束。Linux TUN 初始化的临时 DNS 故障可重试；解析到了 Mesh 网段、身份不一致或缺少网络权限仍会失败关闭。

Access v2 的 Ubuntu ARM64 镜像 `sha256:505e2e5beca37ed0449120cddc14d77b1b0ad784a13e215e21d20caa8c143984` 已通过更新后的隔离验收（运行 ID `mupb51em-eaecbd9d255f1984`），共 33 项证据、清理无错误。覆盖单条双向规则、tag 成员变化、已分配标签的目录更新、ACL 双向 first-match、default-allow 仍受路由 reject / via 约束，以及原有 lease、DNS 恢复和撤销测试。

### 持久 demo 的启用方式

先在 `/routing` 设置路径、在 `/access` 添加对应的有向授权，再使用 demo 工具更新原容器。既有身份、私钥、profile 与用户配置保留，不通过重新注册取得新身份：

```sh
python3 experiments/managed-connections/demo.py upgrade-image \
  --data-dir .meshway/docker-demo \
  --image meshway-managedtest:dataplane-20261001-a \
  --admin-token-file .meshway/controller/admin.token

python3 experiments/managed-connections/demo.py enable-dataplane \
  --data-dir .meshway/docker-demo \
  --admin-token-file .meshway/controller/admin.token
```

工具先持久记录模式切换意图，再重建属于该 demo 的容器；重试沿用已有身份并核对新进程 epoch。进入容器时使用原状态所有者 UID/GID：

```sh
docker exec --user "$(id -u):$(id -g)" -it <容器名称> sh
```

Linux 启动器把权限收敛到原非 root UID 和 `NET_ADMIN`，不需要 `NET_RAW` 或 privileged 模式。`enable-dataplane` 本身不修改路由或添加授权。恢复为只运行邻接使用 `disable-dataplane`，完整生命周期与恢复说明见 `experiments/managed-connections/DEMO.md`。

当前持久 demo 已启用可写 Ubuntu 开发 shell，可直接 `docker exec -it <容器名称> bash` 后使用 `apt-get`。预装 `traceroute`，根 shell 的 `meshway` 启动器会自动切到原 Agent UID；Agent 进程仍只保留 `NET_ADMIN`。手动安装的软件属于容器可写层，重建时会丢失；常用工具可加入 `Dockerfile.dev`。

### 已启用的本机 demo

| 节点 | Mesh IPv4 | 物理邻接 |
| --- | --- | --- |
| `demo-nyc` | `100.96.0.2` | Native UDP 到 LAX |
| `demo-lax` | `100.96.0.3` | 接受 NYC 和 SHA 的连接 |
| `demo-sha` | `100.96.0.4` | 主动建立到 LAX 的 TLS-VLESS |

初始 demo 配置保留 NYC→LAX→SHA 路由，并增加 SHA→LAX→NYC 回程；另外四个方向使用 default-direct。升级后六个方向的访问许可折叠为三条双向端点规则，未授权的本机 `nyc` 不因此获得业务访问权。原三个节点的身份文件、私钥、profiles、证书、公钥绑定和监听配置均已比较保留；之后的用户编辑以控制台当前版本为准。

```sh
docker exec --user "$(id -u):$(id -g)" meshway-demo-c011ae657303-nyc \
  ping -c 3 100.96.0.4
docker exec --user "$(id -u):$(id -g)" meshway-demo-c011ae657303-sha \
  ping -c 3 100.96.0.2
docker exec --user "$(id -u):$(id -g)" meshway-demo-c011ae657303-nyc \
  curl --noproxy '*' --fail --max-time 5 http://100.96.0.4:8080/
```

本机实测六个方向各三次 ICMP 均收到、零丢包；`ip -4 route get <对端 Mesh IPv4>` 显示 `meshway0`。这些是同一台机器里的功能测试，不是跨地域延迟或性能基准。普通 ping 的 TTL 不能表示加密路径中继数，逐跳 trace 仍未实现。

SHA 的临时 HTTP 演示服务仅绑定 `100.96.0.4:8080`，返回固定文本 `Meshway TUN demo: SHA`，不读取或提供任何文件目录。NYC 和 LAX 的真实 HTTP 请求已验证成功；容器重建后该临时服务需要重新启动，普通 ICMP 不依赖它。
