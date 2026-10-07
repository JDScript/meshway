# 网络模式与本机运行

## 启动与接入

`meshwayd`／`meshway` 提供 Server 控制接入与 Linux TUN 运行装配。以下本地示例显式选择 `control-only`，假定已有当前版本 Server 位于 `127.0.0.1:8430`（示例值；开发服务固定 8420，见[开发服务](/v2/development#本机服务)），Console 位于 `127.0.0.1:3001` 并连接该 Server，且 Server 已配置对应的 [Console origin](/v2/server#运行参数)：

```sh
mise run build:node
install -d -m 700 .meshway
./target/debug/meshwayd \
  --data-dir "$PWD/.meshway/node" \
  --network-mode control-only \
  --server http://127.0.0.1:8430
```

另开终端操作同一 daemon：

```sh
./target/debug/meshway --socket "$PWD/.meshway/node/control.sock" up
./target/debug/meshway --socket "$PWD/.meshway/node/control.sock" status --watch
./target/debug/meshway --socket "$PWD/.meshway/node/control.sock" status --json
./target/debug/meshway --socket "$PWD/.meshway/node/control.sock" down
```

- 首次 `up` 原样显示 Server 返回的浏览器批准链接并等待批准。Server 未配置 Console origin 时，JSON 状态保留 `enrollmentUrl: null`，人类输出提示联系 Server 运维，状态仍为等待批准。
- 在 Console 登录并选择网络批准后，daemon 自动兑换身份、认证、取得实例、保存配置和发送 saved 回执。
- 此示例的 `controlReady`／`dataPlane=controlOnly` 表示控制流程就绪，`applied=null`，不安装网络。Linux TUN 使用 `--network-mode tun`（默认值），还须满足下节安装范围与设备／路由权限。

`meshwayd` 前台运行，SIGINT／SIGTERM 执行关闭与 join。进程退出保留期望状态：退出前为 up，重启后自动重新取得运行资格；显式 down 后重启继续保持下线。CLI 的 Ctrl-C 仅停止等待，不撤回已由 daemon 接纳的持久操作。

| 参数 | 行为 |
| --- | --- |
| daemon `--data-dir` | 私有状态目录，默认 `/var/lib/meshway`；父目录须存在，Unix socket 位于其 `control.sock` |
| daemon `--server`／`MESHWAY_SERVER_URL` | 必须明确指定 Server；远端要求 HTTPS，数字 loopback 可使用 HTTP |
| daemon `--hostname` | 首次接入建议名；省略取系统 hostname 的首个 DNS label，既有业务名称仍由 Server 管理 |
| daemon `--network-mode`／`MESHWAY_NETWORK_MODE` | 本机启动参数 `tun`（默认）或 `control-only`；不是云端业务字段，失败不自动回退 |
| daemon `--bootstrap-host NAME=IP` | 可重复，为 Server 或配置中 carrier 名称提供固定 bootstrap 映射 |
| daemon `--bootstrap-nameserver IP:PORT` | 可重复，指定受保护 bootstrap DNS 的真实上游；IPv6 使用 `[IP]:PORT` |
| daemon `--dns-backend` | `resolved`（默认）或显式 `resolv-conf`；本机安装后端，失败不自动切换 |
| daemon `--dns-bind` | loopback `IP:PORT`；resolved 默认 `127.0.0.1:5354`，文件代理默认 `127.0.0.1:53` 且要求端口 53 |
| daemon `--dns-anchor-address` | resolved 的独立本机 DNS 接口 IPv4 地址，默认 `198.18.255.254`；归属／地址冲突拒绝安装 |
| daemon `--dns-resolv-conf` | 仅用于文件代理，当前 daemon 仅接受 `/etc/resolv.conf`；文件必须满足[系统 DNS 归属约束](/v2/node/dataplane#系统-dns-安装) |
| CLI `--socket` | 默认 `/var/lib/meshway/control.sock`，可显式选择本机 daemon |
| CLI `--json` | 输出类型化公开状态；不包含 Node 私钥、兑换码或会话 token |

默认目录适用于 Linux 系统服务；本地开发或其他 Unix 环境可像示例一样显式指定私有目录。安装参数不构成本机业务配置覆盖源。

daemon／CLI 共用[运行日志参数](/v2/logging)，日志位于 stderr；CLI 的 `--json` 状态结果位于 stdout。

本机多节点开发使用[三节点 Docker 环境](/v2/node/docker-lab)，统一投放预编译程序，并为每个 Node 保留独立状态卷。

## 安装范围与状态

当前 TUN 候选由 daemon 装配 `meshway-underlay::Host`、`meshway-connectivity::Manager` 和 `meshway-dataplane::secure::SecureNode`。TUN 源包按目标 IP、协议／端口和当前 profile 提交需求，续租复用原 demand ID；Server 将已分配 Mesh IP 规范化为 Mesh Node 目标。原路径回复查询 SecureNode 的唯一流表，不另建 daemon 反向流表。

| 范围 | 当前行为 |
| --- | --- |
| Linux Mesh-only | TUN capture、Native TCP／Native UDP 静态端点认证邻接、协调 UDP 穿透、双栈 UDP／TCP 多跳包流、持续续租和失效清理；验收状态见[验收范围](/v2/node/validation) |
| DNS | 已接入 loopback 服务、精确 resolver 需求、resolved／显式文件代理；所有必需组件安装成功才可 applied，后端约束见[系统 DNS 安装](/v2/node/dataplane#系统-dns-安装) |
| Exit／本机 egress | `exitEnabled`、非 Mesh permit、未由 DNS owner 持有的 local egress 或缺少明确 no-exit DNS 执行证明均拒绝安装；出口／NAT 尚未接线 |
| 诊断 | `meshway ping` 与 Server observation tasks 共用获准测量器，见[路径诊断](/v2/node/diagnostics) |
| 承载 | Manager 共用获准的 stream、Native UDP 静态端点和协调 UDP 穿透；会话与交接见[邻接管理](/v2/node/transport#邻接管理)，NAT 范围见[连接获取](/v2/node/connectivity) |
| 其他平台／入口 | 非 Linux 的 tun 返回 unsupported；userspace（见[规划](#规划-userspace-networking)）、动态插件和切网尚未实现 |

仅验证 Mesh IP、无需系统 DNS 的场景，在 Server 的 [DNS 配置](/v2/configuration-reference/dns)中明确设置 `magicDNS=false`、`overrideLocal=false`，并保持 `servers`、`default`、`routes`、`searchDomains` 为空。此时不创建 DNS 服务／锚点，也不要求机器运行 resolved。存在 DNS 需求时须完成对应安装；本机后端选择不覆盖云端 DNS 业务配置。`control-only` 不安装系统 DNS。

IPC v3 的公开运行状态：

| 字段 | 含义 |
| --- | --- |
| `networkMode` | `tun`／`controlOnly`，本机选择 |
| `dataPlane` | `controlOnly`、`stopped`、`preparing`、`ready`、`suspended`、`failed` |
| `saved`／`applied` | 独立的必填可空目标引用；saved 来自持久保存，applied 来自对应实例／目标的实际成功安装结果 |
| `phase` | 包含 `controlReady` 与 `running`；结合 dataPlane、当前资格和错误读取 |
| `lastError`／`retryAt` | 类型化故障与重试时点；安装问题可为 unsupportedMode、underlay、tun、dns、transport、apply |

ready／applied 只表示该目标的本机安装结果，不表示 peer 已建立认证通道或应用已收回业务包。Node State 中的 listener／link 来自组件观测；路径、RTT、丢包缺测保持空。安装与报告顺序见 [Control ApplicationTicket](/v2/node/control#安装回执与运行报告)。

## 业务数据面

统一业务数据面采用 **L3 IPv4／IPv6 IP 包**。系统 TUN 由 OS socket／路由把受管流量送入 TUN，需要 TUN／路由权限及所需 DNS 资源；它不隐含内核模块、内核内加密或 eBPF。当前目标不包含 Ethernet 透明桥接、MAC 学习或二层广播域。

内层 IP 业务与外层承载独立：TCP／UDP 业务不据此强制选择同名 transport，实际承载由配置与当前执行许可约束。本机权限或 TUN 安装失败不能静默切换模式后报告原目标 applied。同一设备一次只启用一张网络。

## CLI 与 IPC

Linux 首版每台机器运行一个系统级 `meshwayd`，同一时间只启用一张网络。CLI 连接该服务；不为每个本机用户创建独立 daemon 或自动复制 Node 身份。当前使用 owner-only 目录与 `0600` socket，在解析请求前后核验内核提供的 peer UID，只允许 root 或 daemon 同 UID。更细的操作员授权与系统服务安装策略仍待实现。

| 命令／操作 | 行为／状态 |
| --- | --- |
| `meshway up` | 首次浏览器接入，或用保存的身份重新认证并取得运行资格 |
| `meshway down` | 停止当前网络并终止相关资格；daemon 与私有身份保留 |
| `meshway ping <target>` | 获准的端到端 Mesh 探测；参数、授权与输出见[路径诊断](/v2/node/diagnostics) |
| `meshway status` | 区分进程、控制联系、saved／applied 和实际通道状态 |
| 切网 | 尚未实现；先停止旧网络、完成隔离清理，再运行新网络 |

CLI 通过 Unix domain socket 发命令、读结果；状态由 daemon／组件的唯一 owner 持有。`meshway-ipc` 是唯一的请求／响应定义，当前版本为 3，使用 JSON 与长度前缀，单条消息上限 64 KiB。每个连接处理一个请求，Ping 可返回有序的多个事件；连接数、请求等待、写入和队列均有上限。

up/down 按本机期望状态 revision 执行 CAS，并保留最后一次变更的请求身份以恢复不确定结果。本机 revision 与 Server 配置 revision 分开。已接纳命令的持久化由协调器完成，CLI 断开不会撤回它；读取 status 不写入状态或生成测量。

daemon 独占目录锁，启动时记录 socket 的文件身份；只有身份匹配且确认无监听者的旧 socket 可清理。未知文件、符号链接、活跃 socket 和损坏的状态文件都保留并报错。

CLI 生命周期不承载 Node 身份或 lease，不收集账户密码、不取得浏览器会话。业务 listener／DNS／策略无本地覆盖源；获准路由方案切换仍是使用选择，具体入口待定。`down` 不等于云端永久删除。

## 控制调度与恢复

Server HTTP 等待与本机命令处理独立。配置查询通常每 5 秒，实例续租与控制报告通常每 20 秒；真实安装完成、socket／邻接或业务依赖变化还会触发合并后的报告，最多每秒一次。仅观测时间变化不触发新事件，旧报告重试保留原序号／内容／时间，也不确认后来出现的新状态。故障采用有界退避，期限由控制组件独立校验。down 先持久保存本机下线意图，再停止当前资格并尽力释放远端实例；离线时也不能把旧 lease 展示为有效。当前 IPC 不承载业务包。

期望上线时，网络失败、HTTP 408／429／5xx 按 1–30 秒有界退避自动重试。不可解码或超限响应会撤回旧运行上下文后重新取得资格；运行中的 session／instance 拒绝、lease 到期或实例更换会重新持钥认证、取得有效 instance 并上报 State，不要求再次执行 `up`。新鲜身份证明遭拒、Server（`controllerId`）／key／目标绑定错误、本机记录损坏仍进入 `blocked`；后台残留请求不能绕过该停止状态。

控制失败日志包含实际 operation、错误类别、HTTP status、已解码的 problem code、Server request ID 和恢复决策；不记录响应原文或凭据。显式 `down` 的清理若失败，在外部冲突解决后重新执行 `down` 会触发新的清理尝试，同请求重放仍保持幂等。

## 持久与重建

| 私有保存 | 重新建立 |
| --- | --- |
| Server 信任绑定、Node 密钥、待完成接入／换钥 | 短期会话、instance、执行 lease |
| 完整已保存目标与 digest | 实际安装结果、通道、打洞和包会话 |
| 所选网络、期望上线及有效使用选择 | 在线／可达性证明、计数基线 |
| 系统资源归属 journal | 已认证的资源状态与运行代际 |

缓存只作恢复基准，不复活旧授权。重启、释放或过期后使用新 instance；配置历史不能覆盖永久吊销。运行目录／IPC 地址是安装参数，不是另一份网络业务配置。

## 生命周期

```text
取得私有目录／IPC → 装配组件与 provider → 接受上线目标
  → 持钥接入与执行目标 → 校验并保存（saved）
  → 先准备底层保护，再安装组件 → applied／failed 与真实 State
```

宿主协调一个活动网络及有效执行上下文，组件拥有其任务、buffer、socket 和系统资源。具体运行库不重复编译网络策略；saved／applied／业务连通的含义见[配置应用](/v2/configuration-reference/deployment)。

撤回、期限和 down 独立于挂起的 HTTP 生效。停止准入后取消并等待任务退出；显式 down 清理可证明所属的内核资源与 TUN，期望 up 时的过期／断连／退出保留负向 capture 边界。资源生命周期及 bootstrap 同 IP 隔离见 [Linux underlay](/v2/node/dataplane#linux-underlay)。

崩溃恢复只处理有可验证归属的 journal，不恢复旧流。未知格式或不明归属不能作为删除宿主规则的依据。journal 记录写入时的内核 boot id 与网络 namespace：boot id 不同（整机或容器虚拟机重启）时内核规则、路由、TUN 与 DNS 锚点不可能残留，journal 视为不存在并重新分配；同一 boot 下 namespace 不同（例如容器重建后挂载同一卷）仍是 `underlay` 冲突，因为旧 namespace 的资源可能仍然存在且无法从当前 namespace 证明已清理。

## 规划：userspace networking

以下为规划，不是可用功能，`--network-mode` 不接受 userspace：

- 无 TUN，应用通过本机 SOCKS5 接入，由用户态 netstack 处理流量，并接入同一 dataplane，共用身份、准入、端到端保护、完整路径与回复规则。
- 本机 SOCKS5 使用当前网络的业务权限、DNS 和路由，不能作为绕过准入的直拨通路；不包含应用内 socket SDK 或公网代理服务。
- 命令范围、监听地址／端口、本机访问认证、域名解析、UDP 关联生命周期、MTU／分片与出口能力尚待定版。
