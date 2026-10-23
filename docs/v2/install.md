# Linux 安装与系统服务

本页定义 Node（`meshwayd` 与 `meshway`）与 [Server](#server)（`meshway-server`）在 Linux 上作为 systemd 系统服务的安装、升级与卸载；两者由同一个 `install.sh` 安装（`--component`），下载与校验只有一份实现。daemon 参数与运行行为见[网络模式与本机运行](/v2/node/runtime)，Server 运行参数见 [Server](/v2/server#运行参数)，发布产物由 [Release 工作流](#发布产物) 生成。

## 要求

| 项 | 要求 |
| --- | --- |
| 架构 | x86_64 或 aarch64；产物为静态链接的 musl 程序，不依赖发行版 libc |
| init | systemd（`/run/systemd/system` 存在），unit 使用 `Type=notify` |
| 权限 | 安装与 CLI 需要 root；daemon 以 root 运行，但 capability 限于下表 |
| 设备 | `/dev/net/tun` |
| DNS | 默认 `--dns-backend resolved`：需要运行中的 systemd-resolved 253 或更高版本，或含 systemd commit 5f9041af 的稳定版（249.15、250.10、251.11、252.5 起；Debian 12 的 252.39 可用，**Ubuntu 22.04 的 249.11 不可用**，见 [Ubuntu 22.04](#ubuntu-2204)），且 `/etc/resolv.conf` 指向其动态 stub（`/run/systemd/resolve/stub-resolv.conf`）；见[系统 DNS 安装](/v2/node/dataplane#系统-dns-安装) |
| 出口 DNS | resolved 后端的出口节点为源解析普通名称需要 systemd ≥ 248 且 `DNSStubListener=yes`（默认），以提供代理 stub `127.0.0.54:53`；缺少时出口查询不可用（`exit.dnsUnavailable=proxyStubMissing`），IPv4 出口与本节点 DNS 不受影响。见[出口 resolver](/v2/node/dataplane#exit-resolver) |
| 下载 | 在线安装需要 `curl`、`sha256sum`、`tar`；私有仓库需 `GITHUB_TOKEN`（releases 读权限） |
| Server | 可达的 HTTPS Server origin；数字 loopback 可用 HTTP |

## 出口宿主

管理员为节点打开 `exitEnabled` 后，daemon 自动探测并安装出口角色（IPv4；`exitFamilies` 含 `ipv6` 时同一角色另承载 IPv6，见下文 [IPv6 出口宿主](#ipv6-exit-host)）；前提不满足时节点照常 running，只报告 `Exit unavailable (<原因>)`（State `exitCapability`），条件满足后约 20 秒内自动安装，无需重启或 `up`。daemon 从不修改宿主 sysctl 或防火墙。判定细节见[出口宿主前置探测](/v2/node/runtime#出口宿主前置探测)。

| 项 | 要求 | 不满足时 |
| --- | --- | --- |
| 工具 | `/usr/sbin/nft`（nftables）；已加载 legacy iptables 表时还需 `/usr/sbin/iptables-legacy-save` | `nftUnavailable`；legacy 表不可读为 `firewallConflict` |
| conntrack | 内核 ctnetlink（`nf_conntrack_netlink`，含 `CONFIG_NF_CONNTRACK_MARK`；发行版内核默认提供，首次使用时由内核自动加载）。daemon 直接经 netlink 读取与精确删除连接，**不需要** `conntrack` 命令行工具 | `conntrackUnavailable`；内核拒绝（缺 `CAP_NET_ADMIN`）为 `netAdminMissing` |
| 转发 | `net.ipv4.ip_forward=1`，且 `net.ipv4.conf.default.forwarding=1`（出口 TUN 是新接口，继承 default） | `ipForwardDisabled` |
| 反向路径 | 每个接口的有效 `rp_filter`（`max(all, 接口)`）为 0 或 2；strict（1）不支持 | `reversePathFilterConflict` |
| 防火墙 | 没有能挡住出口流量的规则：prerouting／forward／output／postrouting 上不限于其他接口或其他地址的 drop／reject，或 `policy drop` 的 PREROUTING／FORWARD／OUTPUT | `firewallConflict` |
| mark／offload | 外来规则不读写 packet／conntrack mark，不用 flowtable／ct helper；legacy iptables 不用 `MARK`／`CONNMARK`／`CT`／`NOTRACK`／`TPROXY` | `ownershipConflict` |

Docker 可以与出口共存：Docker 自己的 `DOCKER-*` 链只丢弃进出其网桥的包，不算冲突。Docker 启动时若由它打开 `ip_forward`，会把 `FORWARD` 置为 `policy drop`，这确实挡住出口流量；在 Docker 启动前由系统配置打开 `ip_forward`，或在 `/etc/docker/daemon.json` 中设置 `"ip-forward-no-drop": true`（[Docker 文档](https://docs.docker.com/engine/network/packet-filtering-firewalls/#docker-on-a-router)），FORWARD 保持 accept。也可在 `DOCKER-USER` 中显式放行 `-i tun+`／`-o tun+`（放行全部 `tun*` 接口的转发，是宿主管理员的决定）。

出口节点本身运行在 Docker 容器中时，容器需要 `NET_ADMIN`、`/dev/net/tun`、`--sysctl net.ipv4.ip_forward=1` 和镜像内的 nft（不需要 conntrack 工具）；容器继承的 loose `rp_filter`（2）可用。

### IPv6 出口宿主 {#ipv6-exit-host}

`exitFamilies` 含 `ipv6` 的出口另需（每项只影响 IPv6 族，IPv4 族照常就绪；判定见 [Exit 设计 · IPv6 出口](/v2/node/exit-design#ipv6)）：

| 项 | 要求 | 不满足时 |
| --- | --- | --- |
| 转发 | `net.ipv6.conf.all.forwarding=1` | `ipv6ForwardDisabled` |
| 出口地址与路由 | 至少一个可用的全局单播 IPv6 地址（如 VPS 的 on-link /64 内的地址），且主路由表有通往 Internet 的 IPv6 路由（默认路由或直连公网前缀） | `ipv6NoEgress` |
| 路由器通告 | 开启 forwarding 后，靠 SLAAC／RA 得到默认路由的接口须设 `net.ipv6.conf.<接口>.accept_ra=2`，或改为静态配置；否则路由与地址在寿命到期后消失 | `ipv6RouterAdvertisementsIgnored` |
| NAT66 | `ip6` 族 nat 链与 `masquerade`（Linux ≥ 5.1 自带） | `nftUnavailable` |
| 防火墙 | 同上表，按 IPv6 包判定 `ip6`／`inet` 族；已加载 legacy ip6tables 表时还需 `/usr/sbin/ip6tables-legacy-save`。Docker ≥ 27 默认管理 ip6tables，其 `FORWARD policy drop` 同样挡住 IPv6 出口；firewalld 的 `IPv6_rpfilter` 不算冲突 | `firewallConflict` |

出口地址由内核按 RFC 6724 选源：宿主启用 `use_tempaddr=2` 时多数出口流量以临时（privacy）地址出网，这是宿主自己的选择。容器中的出口另需 `--sysctl net.ipv6.conf.all.disable_ipv6=0`、`--sysctl net.ipv6.conf.all.forwarding=1` 与 IPv6 网络。

## 高吞吐宿主 {#high-throughput}

`meshwayd` 以 `CAP_NET_ADMIN` 运行，Mesh UDP socket 的默认缓冲（接收 4 MiB、发送 2 MiB）不受 `net.core.rmem_max`／`wmem_max` 限制，一般不需要改 sysctl；默认值与机制见 [UDP 批量接口](/v2/node/transport#udp-批量接口)。`meshway status` 的 SOCKET 表显示每个 socket 实际得到的大小，被截断时标注 `limited`（日志同时有 warn `UDP socket buffers limited by net.core.rmem_max/wmem_max`）。

只有在 daemon 没有 `CAP_NET_ADMIN`（例如自行改写 unit 去掉了它），或为某个 listener／dialer 显式配置了更大的 `socket.receiveBuffer` 而宿主不允许强制时，才需要提高上限。以下只是建议，daemon 不会自动修改；值需大于等于要请求的字节数：

```sh
# /etc/sysctl.d/90-meshway.conf
net.core.rmem_max = 4194304
net.core.wmem_max = 2097152
```

`sysctl --system` 生效，已有 socket 需要重启 daemon 才重新请求。出口或中继节点同时承载多条通道时，还应检查 `nstat -az UdpRcvbufErrors`：持续增长表示接收方处理不过来，见[接收缓冲实测](/v2/node/performance#接收缓冲)。

## 安装

```sh
curl -fsSLO https://github.com/JDScript/meshway/releases/download/v0.1.0-alpha.23/install.sh
sudo sh install.sh --version v0.1.0-alpha.23 --server https://controller.example.com
sudo meshway up --enrollment-key @/root/enrollment-key
sudo meshway status
```

`install.sh` 按 `uname -m` 选择 `meshway-<target>.tar.gz`，从同一 Release 下载 `SHA256SUMS` 并校验，解包后先运行一次 CLI／daemon 确认能在本机执行，再原子替换 `/usr/local/bin/{meshwayd,meshway}`、安装 unit 并 `systemctl enable` 与启动。`systemctl start` 在 daemon 发出 `READY=1`（本机 IPC 可用）后返回；随后用 `meshway up` 接入。省略 `--version` 安装最新的非预发布版本。不做自更新。

| 参数 | 行为 |
| --- | --- |
| `--version TAG` | 指定 Release tag |
| `--artifact PATH` | 使用本地 Release tarball（同目录的 `SHA256SUMS` 存在时校验）或含 `meshwayd`／`meshway` 的目录；目录中缺 unit 时使用脚本旁的 `systemd/meshwayd.service` |
| `--server URL` | 写入 `/etc/meshway/meshwayd.env` 的 `MESHWAY_SERVER_URL`；首次安装必填，升级省略时保留 |
| `--daemon-options S` | 写入 `MESHWAYD_OPTIONS`，作为额外 daemon 参数，例如 `--hostname web-1` 或 `--bootstrap-nameserver 1.1.1.1:53` |
| `--no-start` | 只安装并 enable，不启动／重启 |
| `--uninstall [--purge]` | 见[卸载](#卸载) |

环境变量 `MESHWAY_REPO`（默认 `JDScript/meshway`）选择仓库；`GITHUB_TOKEN` 存在时经 GitHub API 下载 Release 资产（私有仓库的唯一途径）。

脚本幂等：内容相同则不替换、不重启，重复运行只确认服务处于运行状态。

### 信任模型 {#trust-model}

安装信任的是三样东西：HTTPS 传输（`curl --proto '=https'`）、GitHub Release 的发布权限（只有仓库 `release.yml` 中唯一有写权限的 `publish` job 在推送 tag 时创建 Release，其余 job 与所有 workflow 默认只读，第三方 Action 钉到完整 commit），以及同一 Release 中的 `SHA256SUMS`。校验和与产物来自同一来源，所以它防的是传输损坏与本机文件被替换，不防 Release 本身被篡改；产物**没有独立签名**，独立签名（与发布权限分离的签名钥）在 v0.1.0 之后提供。`--artifact` 的本地 tarball 先复制到脚本的私有工作目录（0700），校验与解包都只用这一份副本，校验后不再读取原路径；旁边没有 `SHA256SUMS` 时只告警、不校验。

## systemd unit

unit 原文为仓库 `dist/systemd/meshwayd.service`，随每个 Release tarball 发布。

| 项 | 取值与理由 |
| --- | --- |
| 启动顺序 | `Wants=`／`After=network-online.target`，另在 systemd-resolved 与 D-Bus 之后启动 |
| 就绪 | `Type=notify`、`NotifyAccess=main`：本机 IPC socket 绑定后发送 `READY=1`，开始停止时发送 `STOPPING=1`。就绪只表示可接受 CLI 命令，不表示已 running |
| 命令 | `meshwayd --data-dir /var/lib/meshway $MESHWAYD_OPTIONS`；`EnvironmentFile=/etc/meshway/meshwayd.env` 提供 `MESHWAY_SERVER_URL` |
| 重启 | `Restart=on-failure`、`RestartSec=5s`；`KillMode=mixed`，SIGTERM 只发给 daemon，由它取消并等待自有 nft 子进程；`TimeoutStopSec=45s` |
| 状态目录 | `StateDirectory=meshway`（`/var/lib/meshway`，0700）：身份、`netmap.json`、journal 与 `control.sock`（0600，只允许 root 或 daemon 同 UID） |
| 运行目录 | `RuntimeDirectory=meshway`（`/run/meshway`，0700）且 `RuntimeDirectoryPreserve=yes`：namespace 级 DNS 归属记录需要跨 daemon 重启保留（DNS 锚点保留到显式 down），重启机器时随 tmpfs 清空 |
| 日志 | stderr JSON（`MESHWAY_LOG_FORMAT=json`，`RUST_LOG=info`）进入 journald；stdout 的就绪／停止行同样进入 journal。字段见[运行日志](/v2/logging) |
| capability | `CapabilityBoundingSet`＝`AmbientCapabilities`＝`CAP_NET_ADMIN`（TUN、路由、nft）、`CAP_NET_RAW`（受保护 socket）、`CAP_NET_BIND_SERVICE`（resolv-conf 后端的 53 端口）、`CAP_SYS_PTRACE`（跨 UID 读取 systemd-resolved 的 `/proc/<pid>/ns/net` 以核验同一 namespace） |
| 沙箱 | `ProtectSystem=strict`（除上述两目录外只读）、`ProtectHome`、`PrivateTmp`、`DevicePolicy=closed`＋`DeviceAllow=/dev/net/tun rw`、`ProtectKernelTunables`／`Modules`／`Logs`、`ProtectControlGroups`、`ProtectClock`、`ProtectHostname`、`RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6 AF_NETLINK`、`RestrictNamespaces`、`NoNewPrivileges`、`MemoryDenyWriteExecute`、`SystemCallFilter=@system-service` |

查看服务与日志：

```sh
systemctl status meshwayd
journalctl -u meshwayd -o cat -f
```

### 本机定制

用 drop-in 修改，不编辑安装的 unit（升级会替换它）：

```sh
sudo systemctl edit meshwayd
```

显式文件代理后端（`--dns-backend resolv-conf`）需要原子替换 `/etc/resolv.conf`，必须放开 `/etc` 的写入：

```ini
[Service]
ReadWritePaths=/etc
```

并用 `--daemon-options "--dns-backend resolv-conf"` 重新运行安装脚本。

### Ubuntu 22.04 {#ubuntu-2204}

Ubuntu 22.04 的 systemd-resolved 249.11 把发往链路 DNS 服务器的查询绑在该链路上发出，锚点链路的 loopback 服务器（Meshway 的 bind）收不到，resolved 后端不可用：Status 为 `lastError: dns`，`lastErrorDetail` 为 `DNS probe check failed (system DNS backend is unsupported): expected resolved sends a nonce query to the owned link's loopback DNS server over UDP then TCP (systemd-resolved 253 or later, or stable 249.15, 250.10, 251.11, 252.5 or later); found no query reached 127.0.0.1:<port> within 5 s; systemd 249.11-…`（`expected` 末尾带 `see install.md: Ubuntu 22.04`），phase 为 `retrying`；daemon 每次重试都重新探测，间隔 1 秒起翻倍到 60 秒，不会自动改用其他后端。按下列步骤显式选择文件代理：

```sh
sudo meshway down                      # 结束 resolved 后端的预约与锚点
# /etc/resolv.conf 改为普通文件，内容为 resolved 记录的真实上游（不是 127.0.0.53 stub）
sudo cp --remove-destination /run/systemd/resolve/resolv.conf /etc/resolv.conf
sudo systemctl edit meshwayd           # [Service] ReadWritePaths=/etc（见上）
sudo sh install.sh --daemon-options "--dns-backend resolv-conf"
sudo meshway up
```

文件代理的限制见[系统 DNS 安装](/v2/node/dataplane#系统-dns-安装)：上游固定为安装时文件中的 nameserver，DHCP 后来的变化不跟随，不支持漫游；libc 不再经 resolved 的按链路路由。恢复 resolved 时先 `meshway down`（还原文件），再 `sudo ln -sf /run/systemd/resolve/stub-resolv.conf /etc/resolv.conf`，去掉 `--dns-backend` 后重新运行安装脚本。隔离验收见[验收范围](/v2/node/validation)的 `jammy/resolv-conf` 行。

## 重启与离线

- 期望状态持久保存在 `/var/lib/meshway`：开机前为 up，开机后服务自动重新取得运行资格，不需要再次 `meshway up`；显式 `down` 后保持下线。
- 开机后 Server 不可达时，daemon 从 `netmap.json` 载入签名租约内的 netmap 并安装数据面；租约到期即全部撤回（fail closed），见[离线运行](/v2/node/netmap-design)。
- 上一 boot 的 Underlay journal 不属于当前 namespace，按[持久与重建](/v2/node/runtime#持久与重建)归档为 `underlay.json.stale-*` 后重新分配。

- 开机时 systemd-resolved 由 unit 排序先于 daemon 启动；DNS 归属记录随 `/run` 清空，按当前 resolved 实例重新安装。
- 上一 boot 的 journal 归档只保留最新三份 `underlay.json.stale-*`（每份约 4 KiB），更早的在归档时删除。
- daemon 被 `SIGKILL` 后留下的 `control.sock`／`underlay-bootstrap.sock` 按记录的 inode 认领回收，记录的 device 与当前状态卷不同（卷被重新挂载或重建）也不阻止回收；私有目录中没有记录的 socket 同样回收。

### systemd-resolved 重启与停止

resolved 后端的归属证明是同一 network namespace、Underlay 管理的 DNS 锚点接口，以及锚点链路上 daemon 写入并回读的 DNS／domain 设置（含安装标记域 `~i<id>.meshway-installation.invalid`），不依赖 resolved 进程的 D-Bus 唯一名。

| 事件 | 行为 |
| --- | --- |
| `systemctl restart systemd-resolved`（含软件包升级触发） | daemon 在新实例上重写并回读锚点链路设置；Mesh 名称解析恢复不超过 5 秒。实测五次重启中四次在 0.27 秒采样下无失败、`phase`／`dataPlane` 保持 `running`／`ready`；一次在 resolved 尚未响应锚点检查时短暂进入 `synchronizing`／`suspended`，Mesh 名称 0.6 秒内恢复 |
| `systemctl stop systemd-resolved` | 系统解析与 Mesh 解析均不可用；daemon 在 0.3 秒内进入 `blocked`／`dataPlane=failed`／`lastError=dns`，按"必需组件失败即全撤"撤回整个数据面（Mesh IP 也不可达） |
| 随后 `systemctl start systemd-resolved` | 无需 `up` 或重启服务，daemon 重新安装 netmap；Mesh 名称解析恢复不超过 10 秒（实测 1.1 秒），经 `synchronizing`／`preparing` 回到 `running`／`ready` |

### 当前限制

- 显式文件代理后端（`resolv-conf`）只在 `/etc/resolv.conf` 的 inode 与内容仍是自己安装的版本时恢复；外部改写该文件按[归属规则](/v2/node/dataplane#系统-dns-安装)报告冲突并阻断 applied，`down` 也不覆盖外部内容，需要管理员恢复文件后再 `down`／`up`。
- 系统 hosts 在每次 bootstrap 查询时按当时内容读取：Server／承载名称映射修改后对后续解析立即生效，已建立的连接不受影响。

实测结论与时间见[验收记录](#验收记录)。

## 升级

```sh
sudo sh install.sh --version v0.1.1
```

脚本替换二进制与 unit 后重启服务；daemon 在 SIGTERM 时保留负向 capture 边界与 DNS 锚点，新进程恢复原 up／down 意图与身份。CLI 与 daemon 同版本发布，IPC 版本不同的组合返回 `version` 错误，因此两者一起替换。核对版本：

```sh
meshway version
```

```text
meshway 0.1.0-alpha.23 (v0.1.0-alpha.23-0-g1234abcd)
target:   aarch64-unknown-linux-musl
ipc:      13
wire:     6
compiler: meshway-netmap-4
```

`--json` 输出同名字段（`version`、`git`、`target`、`ipc`、`wire`、`compilerVersion`）。`wire` 是受保护 circuit 版本，同一路径上的节点必须一致；`compilerVersion` 是 netmap 编译契约。该命令不联系 daemon。

### 与 Server 的顺序

先升级全部 Node（CLI 与 daemon 一起），再升级 Server；Server 的步骤见 [Server 升级](#server-upgrade)。

### compilerVersion 变化的版本 {#compiler-version-upgrade}

`compilerVersion` 变化时（[changelog](/v2/changelog) 中该版本的范围句以 “⚠ 断网窗口” 开头），新 Node 不接受旧 Server 签发的 netmap，旧 Node 也不接受新 Server 的：已升级的 Node 每次 `sync` 都以 `Contract` 失败并重试，`meshway status` 为 `synchronizing · Sync reconnecting (contract)`、Netmap none、数据面 suspended／stopped；本机缓存的旧 `netmap.json` 保留在磁盘但不载入。该节点从升级起到 Server 升级完成为止没有 Mesh。这是该类版本的固有窗口，产品不提供兼容轨，因此按以下方式把窗口压到最短：

1. **同一窗口升级全部**：所有 Node 与 Server 在同一个维护窗口内升级；不要只升级一部分后长时间停留。
2. **先备齐产物**：开始前确认该版本的全部产物已就绪——Release 的各架构 tar 与 `SHA256SUMS` 已发布并校验，Server 已按该版本构建，Docker lab 等需单独构建的环境（`mise run build:node:linux`）已构建完成。不要在产物仍在排队构建时就升级第一台节点。
3. **顺序不变**：仍先 Node（CLI 与 daemon 一起）后 Server；Server 升级后，各节点下一次 `sync` 即取得新 netmap 并恢复，无需 `up`，也无需改动配置（旧编译器留下的 delta 基准按[无基准](/v2/node/netmap-design#server-侧)处理）。
4. **窗口长度**＝从第一台节点升级到 Server 升级完成；节点升级本身只需数秒，窗口主要由节点之间的等待与 Server 升级决定。

**升级时的源端 IPv6 捕获**：同一次启动内由旧版本建立、未 `down` 的源端捕获表没有 IPv6 捕获（`underlay-egress.json` 的程序不含 `capture6`），新版本沿用它时把受管与 sink 的 IPv6 出口区间都拒绝（fail closed），IPv6 出口在该节点 `meshway down` 后再 `up`，或主机重启后才生效；IPv4 不受影响。

`wire` 变化时同一路径上新旧节点的帧互不兼容（[数据面 · wire](/v2/node/dataplane)），同样在一个窗口内全部升级。

## 迁到另一个 Server {#move-server}

一个 Node 身份只属于接入它的 Server。迁到另一个 Server 是先在原 Server 注销、再在新 Server 重新接入，得到新的 Node（新密钥、新 Node ID）；身份目录不需要手工删除或改名。

```sh
sudo meshway logout                    # 原 Server 确认删除；status 无 Identity、phase down
sudoedit /etc/meshway/meshwayd.env     # MESHWAY_SERVER_URL=https://new-server.example.com
sudo systemctl restart meshwayd
sudo meshway up --enrollment-key @/root/enrollment-key   # 新 Server 签发的接入密钥
```

也可以用 `sudo sh install.sh --version <当前已装 tag> --server https://new-server.example.com` 改写 `MESHWAY_SERVER_URL` 并重启（省略 `--version` 会安装最新版本）。

| 步骤 | 说明 |
| --- | --- |
| `logout` | 退出码 0 即原 Server 已删除本节点、本机身份已清除。退出码 1 且 phase 为 `leaving` 时原 Server 尚未回答：保持原 URL 等它完成（daemon 后台重试，`meshway status` 不再显示 Identity 即完成），再改 URL |
| 改 URL 并重启 | 已注销的身份目录不属于任何 Server，以新 URL 启动即为未接入、`down` |
| `up` | 与首次接入相同：`--enrollment-key` 或浏览器批准 |

注销未完成就改了 URL，daemon 为 `blocked · storage`，`meshway status` 的 Problem 下一行写明身份仍属于哪个 Server 及处理办法（改回原 URL 让注销完成；仍有接入中的请求时在原 URL 下 `meshway down` 取消它）。原 Server 已永久不可用时注销无法完成：停止服务、把 `/var/lib/meshway/identity` 改名存档后再启动，再 `up`；原 Server 上的旧 Node 记录不会被删除。语义见[自助注销](/v2/configuration-reference/nodes#node-self-leave)与[身份记录与 Server origin](/v2/node/runtime#identity-origin)。

容器节点同样处理：在容器内 `meshway logout`，修改 compose／`docker run` 中的 `MESHWAY_SERVER_URL`（或 `--server`）后重建容器（身份所在的状态卷保留），再接入。接入密钥经 stdin 交给容器内 CLI——`docker cp` 写不进容器的 `/run` 等 tmpfs 挂载，`@/run/...` 会读不到：

```sh
docker exec <container> meshway logout
# 修改 MESHWAY_SERVER_URL 后
docker compose up -d <service>
docker exec -i <container> meshway up --enrollment-key - < enrollment-key
```

## 卸载

```sh
sudo sh install.sh --uninstall           # 保留身份与设置
sudo sh install.sh --uninstall --purge   # 同时删除 /var/lib/meshway 与 /etc/meshway
```

卸载先执行 `meshway down`：显式 down 才会移除 TUN、DNS 锚点与负向 capture 规则；仅停止服务会保留它们供下次启动。down 失败时脚本停止并保留运行中的服务。随后停止并禁用服务，删除 unit、两个二进制和 `/run/meshway`。不带 `--purge` 时保留 Node 身份（`/var/lib/meshway`）、`/etc/meshway` 与 `systemctl edit` 建立的 drop-in；`--purge` 一并删除它们；保留身份时重新安装后仍是同一个 Node，`--purge` 后再次安装需要重新接入。

## Server {#server}

Server（`meshway-server`）作为 systemd 系统服务运行在一台 Linux 主机上。路径与 Node 完全分开（`/var/lib/meshway-server`、`/etc/meshway-server`），可与 `meshwayd` 同机。运行参数、退出状态与离线命令见 [Server](/v2/server#运行参数)。

### 要求 {#server-requirements}

| 项 | 要求 |
| --- | --- |
| 架构 | x86_64 或 aarch64；静态链接的 musl 程序 |
| init | systemd，unit 使用 `Type=notify` |
| 用户 | 安装脚本创建静态系统用户 `meshway-server`；不用 `DynamicUser`：它的 `StateDirectory` 经 `/var/lib/private` 符号链接暴露，与 data-dir 不接受符号链接的检查冲突，离线命令也需要以同一 UID 运行 |
| 前置代理 | Server 只监听明文 HTTP；TLS 由前置反向代理终结，Console 静态文件由同一代理或静态托管（Cloudflare Pages）提供，见 [Console 与 TLS](#server-proxy) |
| 工具 | `curl`、`sha256sum`、`tar`、`od`、`runuser`、`useradd`、`flock`（util-linux；Debian／Ubuntu 默认具备） |
| 磁盘 | 迁移需要 data-dir 所在文件系统有不少于库大小 110% 的余量 |

### 安装 {#server-install}

```sh
curl -fsSLO https://github.com/JDScript/meshway/releases/download/v0.1.0/install.sh
sudo sh install.sh --component server --version v0.1.0
sudoedit /etc/meshway-server/server.env    # MESHWAY_CONSOLE_ORIGIN、监听地址等
sudo systemctl restart meshway-server
```

`--component server` 选择 `meshway-server-<target>.tar.gz`，下载、`SHA256SUMS` 校验、架构识别与 `--version`／`--artifact`／`--no-start` 与 Node 共用同一实现；`--server`／`--daemon-options` 只属于 Node，与之同用即拒绝。脚本先运行一次新程序的 `--version` 确认能在本机执行，创建系统用户，首次安装时写出设置文件，安装程序与 unit，`systemctl enable` 并启动；`systemctl start` 在 Server 发出 `READY=1` 后返回，脚本再请求 `GET /api/v1/auth/options` 确认可用。内容相同则不替换、不重启（MainPID 不变）。

| 路径 | 内容 |
| --- | --- |
| `/usr/local/bin/meshway-server` | 服务程序，root 0755；升级时上一版保留为 `meshway-server.previous` |
| `/usr/local/libexec/meshway-server/{ip-datasets,reset-password}` | 离线命令，同上保留 `.previous` |
| `/etc/systemd/system/meshway-server.service` | unit，原文为仓库 `dist/systemd/meshway-server.service` |
| `/etc/meshway-server/server.env` | 实例设置（`EnvironmentFile`），首次安装创建，此后归运维、脚本不改写 |
| `/var/lib/meshway-server` | data-dir（`StateDirectory`，0700，属主 `meshway-server`）：`controller.lock`、`controller.sqlite`、`master-key`、迁移备份 `controller.sqlite.v<N>-<ms>` |

首次安装写出的设置文件：

```sh
# /etc/meshway-server/server.env
MESHWAY_LISTEN=127.0.0.1:8420
#MESHWAY_CONSOLE_ORIGIN=https://console.example.com
#MESHWAY_PUBLIC_ORIGIN=https://meshway.example.com
#MESHWAY_REGISTRATION_ENABLED=true
# Further arguments, e.g. --stun-listen 0.0.0.0:3478 --stun-server main=meshway.example.com:3478
MESHWAY_SERVER_OPTIONS=
```

变量即 [Server · 运行参数](/v2/server#运行参数) 的同名环境变量；只有命令行形式的参数（`--stun-listen`、`--stun-server`）放进 `MESHWAY_SERVER_OPTIONS`。脚本的健康检查读取其中的 `MESHWAY_LISTEN`（`0.0.0.0`／`[::]` 按 loopback 访问）。

### systemd unit {#server-unit}

| 项 | 取值与理由 |
| --- | --- |
| 就绪 | `Type=notify`、`NotifyAccess=main`：存储已打开（含迁移与启动重编译）且监听已绑定后发 `READY=1` 与 `STATUS=Serving on <地址>`，开始停止时发 `STOPPING=1`；发送实现与 `meshwayd` 共用 `meshway-service` |
| 启动超时 | `TimeoutStartSec=120s`；存储打开期间（迁移与启动重编译）由一个续期任务每 30 秒发 `EXTEND_TIMEOUT_USEC`（120 秒），步骤只更新 `STATUS=`（如 `Migrating the database from schema <N> to <M>: …`、`Compiling stored netmaps with this build: network <i> of <n>`），打开完成或失败即停止续期。单步耗时不受启动超时限制 |
| 重启 | `Restart=on-failure`、`RestartSec=5s`；`RestartPreventExitStatus=78`：存储持久阻断（[退出状态](/v2/server#运行参数)）不重启，`systemctl status` 显示 `status=78`，需要人工处理；其他失败为暂态，按 5 秒重启并重新执行同一检查与迁移 |
| 用户与权限 | `User=`／`Group=meshway-server`；`CapabilityBoundingSet=` 与 `AmbientCapabilities=` 为空（零 capability：默认端口 8420 与 STUN 3478 都在 1024 以上）；`NoNewPrivileges=yes` |
| 文件系统 | `ProtectSystem=strict`：程序与 `/etc` 只读，进程只可写 `StateDirectory=meshway-server`（`StateDirectoryMode=0700`）；`UMask=0077`；`ProtectHome`、`PrivateTmp`、`PrivateDevices` |
| 其余沙箱 | `ProtectHostname`、`ProtectClock`、`ProtectKernelModules`／`Logs`／`Tunables`、`ProtectControlGroups`、`RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6`、`RestrictNamespaces`、`RestrictRealtime`、`RestrictSUIDSGID`、`LockPersonality`、`MemoryDenyWriteExecute`、`SystemCallArchitectures=native`、`SystemCallFilter=@system-service` |
| 日志 | `MESHWAY_LOG_FORMAT=json`、`RUST_LOG=info`，stderr 进入 journald；迁移（`Database migration started`／`Database migrated`）、持久阻断原因与首次应答 sync（`First Node sync answered by this Server process`，仅诊断，回退依据是 [`served` 记录](#server-rollback)）各有固定消息，字段见[运行日志](/v2/logging) |

本机定制用 `sudo systemctl edit meshway-server` 写 drop-in，不编辑安装的 unit。

### Console 与 TLS {#server-proxy}

Server 不提供静态文件。Console 是纯 SPA，有两种托管形态；两者都把 `MESHWAY_CONSOLE_ORIGIN` 设为浏览器打开 Console 的 origin（设备批准链接也指向它）。

| 形态 | Console 调用 API 的 origin | Server 侧 |
| --- | --- | --- |
| 反向代理同源 | 页面自己的 origin（构建不设 `VITE_MESHWAY_API_ORIGIN`） | 不需要 CORS |
| 静态托管（Cloudflare Pages）＋ CORS | 构建期 `VITE_MESHWAY_API_ORIGIN` 指定的 Server origin | `--console-origin` 放行该 Console origin 的[跨源访问](/v2/server#cors) |

`VITE_MESHWAY_API_ORIGIN` 为 `https://host[:port]`（loopback 上可为 `http`），不含路径、查询、片段或凭据，格式不符时构建失败；未设置或为空时使用页面自己的 origin。它只用于 `/api/v1`（含 SSE），添加节点页的安装命令另由 `VITE_MESHWAY_SERVER_URL` 给出 Node 的 `--server`。

**反向代理同源**：前置反向代理终结 TLS，把 `/api/` 与 `/agent-api/` 转发到 Server，其余路径为 Console 静态文件、未知路径回退到 `index.html`。Caddy 示例：

```text
meshway.example.com {
	handle /api/* {
		reverse_proxy 127.0.0.1:8420
	}
	handle /agent-api/* {
		reverse_proxy 127.0.0.1:8420
	}
	handle {
		root * /srv/meshway-console
		try_files {path} /index.html
		file_server
	}
}
```

Console 静态文件来自 Release 的 `meshway-console.tar.gz`（解包到 `root` 目录，同源构建）。`MESHWAY_CONSOLE_ORIGIN` 设为该站点的 origin（`https://meshway.example.com`），Node 的 `--server` 用同一 origin。用户事件流（SSE）需要代理不缓冲响应（Caddy 对 `text/event-stream` 自动立即转发）。

**静态托管＋CORS**：Console 以 `VITE_MESHWAY_API_ORIGIN=<Server origin>` 构建后上传到静态托管；构建产物带 `_redirects`（`/* /index.html 200`），Cloudflare Pages 据它把客户端路由回退到 `index.html`（反向代理托管时它只是一个不被引用的文件）。Server 仍需前置代理终结 TLS（`/api/` 与 `/agent-api/` 转发到 Server），`MESHWAY_CONSOLE_ORIGIN` 设为 Pages 的自定义域名（如 `https://meshway.example.com`），Node 的 `--server` 用 Server 的 origin。仓库的构建与部署脚本、自定义域名绑定见 `dist/console-pages/README.md`（`mise run deploy:console-pages`）。`*.pages.dev` 预览部署的 origin 不等于 `MESHWAY_CONSOLE_ORIGIN`，因此能加载但不能登录该 Server。

**登录限速**：Server 对同一邮箱、同一来源连续 5 次登录失败后按 1 秒起倍增、最长 5 分钟退避（[认证 · 登录失败退避](/v2/api/authentication#login-backoff)），来源只取 TCP 直接对端，不读 `X-Forwarded-For`。经反向代理时所有客户端的来源都是代理地址，退避因此按账户生效（攻击者可让某个账户的登录最长 5 分钟内返回 `busy`，但不会永久锁定）。按客户端限速应在代理上做，例如 nginx `limit_req zone=$binary_remote_addr` 作用于 `/api/v1/auth/login`，或 Caddy 的 rate-limit 插件；代理同时应限制请求体大小与并发连接。

### 升级 {#server-upgrade}

先升级全部 Node（CLI 与 daemon 一起），再升级 Server。Server 停机期间节点按已持有的租约继续运行（默认 24 h），升级窗口远小于它。

```sh
sudo sh install.sh --component server --version v0.1.1
```

| 步骤 | 行为 |
| --- | --- |
| 1. 预检 | 下载并校验产物；新程序 `--version` 报告 `schema: <N> (migrates from <M>)`；从 `controller.sqlite` 的文件头读 `application_id` 与 `user_version`，不是 Server 库、比新程序新或早于 M 即失败；需要迁移时检查磁盘余量。**任何一项失败都在停止服务之前退出，什么也不改** |
| 2. 停止 | `systemctl stop meshway-server`（服务正在启动或按 `Restart=` 重启中时同样先停止，否则它持有 data-dir 锁），并等到 unit 不再处于 active／activating／deactivating／reloading（至多 60 秒）；停止失败或未停稳即退出，什么也不改。随后读取 data-dir 的 [`served` 记录](#server-rollback)作为回退基准，读不出即重新启动原服务并退出 |
| 3. 迁移 | `runuser -u meshway-server -- meshway-server.new migrate --data-dir /var/lib/meshway-server`：在发布前证明旧库与主密钥匹配，备份旧库、迁移、完整检查、启动重编译，不监听端口（库已是当前版本时只检查与重编译）。失败时按其 `Migration state:` 行（从 data-dir 读回）并核对库文件头处理，不凭退出状态推断：`unpublished`——删除新程序、重新启动原服务；`published <备份>`——持 data-dir 锁把备份放回、迁移后的库改名为 `controller.sqlite.failed-<秒>`，再启动原服务；其他（含两者不一致）——两个版本都不启动，交给运维。成功时脚本只打印迁移结果行，不显示 `Migration state:` |
| 4. 替换 | 原程序保留为 `meshway-server.previous`（离线命令同样），新程序原子替换，安装新 unit |
| 5. 启动与健康 | `systemctl start` 等到 `READY=1`，再请求 `GET /api/v1/auth/options`（最多 10 次，每次间隔 1 秒） |
| 6. 回退 | 第 5 步失败时按[回退边界](#server-rollback)自动回退 |

迁移成功时脚本输出 `migrate` 的结果行（`Database migrated from schema 8 to 9 in <ms> ms; the previous database is kept as <path>.`），升级完成为 `upgraded to <版本>; the previous program is kept as /usr/local/bin/meshway-server.previous, the previous database as <path>`。

Server schema 变化的版本（changelog 范围句写明）由新 Server 在打开数据库前迁移，保留旧库为备份、不需重置或重新接入，见 [Server · 存储迁移](/v2/server#存储迁移)。新 Server 启动时、受理请求之前，把每个网络已存的 netmap 按本构建重新编译（[Server 构建是编译输入](/v2/node/netmap-design#server-侧)）：内容因新构建而变化的节点得到新 serial，下一次 `sync` 即收到，内容不变的节点保留 serial；不需要为此改配置、改选择或手动触发重编译。有节点得到新 serial 时 Server 记 info `Stored netmaps compiled again by this build`（`build`、`networks`、`changed_nodes`、`elapsed_ms`）；没有变化（同一构建重启，或新构建编译出相同内容）只记 debug `Stored netmaps checked against this build; nothing changed`。启动耗时随节点数线性，1000 节点的网络约 2–3 秒。

### 回退边界 {#server-rollback}

**只有在新 Server 从未应答过任何 Node `sync` 时才能回退。** 新 Server 一旦签发过 netmap，节点就持有更高的 serial；回退后旧库签发的 serial 更低，节点拒绝（serial 回退），升级之后的接入、签名钥轮换与配置也随旧库丢失。此后只能前进修复（发布修正版本）。

依据是 data-dir 中的 `served` 记录（0600，属主 `meshway-server`）：每个 Server 进程在发出**第一个** sync 回答之前，把它原子替换为本进程的记录并 fsync，写不成功则这次 sync 失败（503）、下次重写，所以没有记录就没有回答：

```text
meshway-server served
boot=<本进程的随机 UUID>
build=<构建身份>
at_ms=<写入时刻，Unix 毫秒>
```

journal 中的 `First Node sync answered by this Server process` 只用于诊断：日志可能丢失、被轮转或查询失败，不作为依据。

自动回退（第 5 步失败）：

1. 读 `served`，与第 2 步旧服务停稳后读到的基准（文件不存在或其 SHA-256）比较；不同或读不出即不回退，报告 fix forward（journal 中若有上述日志一并提示）。
2. 停止服务并等它停稳；停止失败即退出、不回退。停稳后再比较一次 `served`。
3. 以 `flock` 取得 data-dir 的存储锁（每个 Server 进程与离线命令都持有它）；取不到即不回退。
4. 持锁放回 `.previous` 程序与原 unit；本次升级迁移过时，把新库及其 `-wal`／`-shm` 改名为 `controller.sqlite.failed-<Unix 秒>*` 留作诊断，把迁移备份改回 `controller.sqlite`；释放锁，启动原服务并做同样的健康检查。脚本以失败退出（`rolled back to <原版本>; <新版本> was not installed`）。

手动回退（例如升级完成后才发现问题）遵守同一边界：`served` 的 `build=` 若是新版本，新 Server 已应答过 sync，不得回退。

```sh
sudo systemctl stop meshway-server
systemctl is-active meshway-server                     # 须为 inactive 或 failed
sudo cat /var/lib/meshway-server/served                # build= 为新版本就不得回退
sudo mv /usr/local/bin/meshway-server.previous /usr/local/bin/meshway-server
# 仅当这次升级迁移了库（脚本输出过备份路径）：
cd /var/lib/meshway-server
sudo -u meshway-server flock -n controller.lock sh -c 'for f in controller.sqlite controller.sqlite-wal controller.sqlite-shm; do [ ! -e $f ] || mv $f $f.failed; done; mv controller.sqlite.v<旧版本>-<ms> controller.sqlite'
sudo systemctl start meshway-server
```

新库的 `-wal`／`-shm` 必须随新库移开：留在原名下会被旧版本当作旧库的日志重放。`flock -n` 失败说明仍有进程持有 data-dir，先查明并停止它。

### 备份与恢复 {#server-backup}

迁移前的旧库由迁移自动保留（data-dir 内最新 3 份）。完整的离线备份用 `meshway-server backup`，需要短暂停服务（秒级，节点凭租约不受影响）：

```sh
sudo install -d -o meshway-server -g meshway-server -m 0700 /var/backups/meshway-server
sudo systemctl stop meshway-server
sudo runuser -u meshway-server -- meshway-server backup --data-dir /var/lib/meshway-server \
  --output /var/backups/meshway-server/$(date -u +%Y%m%dT%H%M%SZ)
sudo systemctl start meshway-server
```

输出目录本身就是一个 data-dir（`controller.lock`、`controller.sqlite`、`master-key`），行为见 [Server · 备份](/v2/server#备份)。

**风险：库加主密钥等于这台 Server 的全部秘密**——配置中的协议材料（listener 密码、私钥等）与接入设置等密封内容，以及密封在库中的 netmap 签名钥（可为全网签发 netmap）；账户密码哈希与会话摘要也在库中。只有库没有主密钥的备份无法解密，等于无效备份；所以 `backup` 总是两者一起复制。备份离开主机时必须由运维另行加密（如 `age`），并与 Server 主机分开保管；迁移产生的备份留在 data-dir 内，与主密钥同一保护边界。

恢复到一个新的 data-dir：

```sh
sudo systemctl stop meshway-server
sudo mv /var/lib/meshway-server /var/lib/meshway-server.replaced-$(date -u +%Y%m%dT%H%M%SZ)
sudo cp -a /var/backups/meshway-server/<stamp> /var/lib/meshway-server
sudo systemctl start meshway-server
```

`cp -a` 保留 0700／0600 与属主。恢复的备份与回退受同一 serial 边界约束：备份之后与 Server 同步过的节点持有更高的 serial，拒绝恢复后的 netmap，直到租约到期 fail closed，需要重新接入；备份点之后的接入、配置与签名钥轮换也不在其中。因此备份用于主机丢失后的灾难恢复与离线快照，不用于日常回滚。备份不含回退判据用的 `served` 记录：恢复出的 Server 在第一次应答 Node sync 前重新写入它，之前的状态按“未签发”处理。

### 卸载 {#server-uninstall}

```sh
sudo sh install.sh --component server --uninstall           # 保留库、主密钥与设置
sudo sh install.sh --component server --uninstall --purge   # 同时删除 /var/lib/meshway-server、/etc/meshway-server 与用户
```

卸载先禁用并停止服务，等到 unit 不再处于 active／activating／deactivating／reloading；停止失败或 60 秒内未停稳即退出、什么也不删。随后删除 unit、程序及其 `.previous`。不带 `--purge` 时 data-dir、设置文件、drop-in 与系统用户保留，重新安装后是同一个 Server；`--purge` 只在以 `flock` 取得 data-dir 的存储锁后删除库与主密钥（全部秘密，不可恢复，先做[备份](#server-backup)），锁被占用（例如有离线命令在运行）即不删除。

## 发布产物

tag `v*` 触发 `.github/workflows/release.yml`：

1. tag 必须等于 workspace `Cargo.toml` 的 `version` 加 `v` 前缀，且本页同级的 [changelog](/v2/changelog) 有同名 `## <tag>` 段，否则失败且不构建。
2. 以 `workflow_call` 复用 `build.yml`，交叉编译两个 musl target 的 release 程序（Node 与 Server 分两次 cargo 调用，依赖 feature 不互相合并）；`meshway version` 与 `meshway-server --version` 的 git 描述来自 `git describe --tags`。
3. 发布门槛 `server-upgrade`：`scripts/ci/server-upgrade.py` 用上一 release 的 Server（取其 `meshway-server-<target>.tar.gz`；该 release 没有发布 Server 时按其 tag 源码构建）经真实 HTTP 建库——账户、网络、已应用的配置、一个以接入密钥接入并保持运行的 control-only Node（本次构建；Node 先于 Server 升级）——停机后以本次 x86_64 构建 `migrate` 同一 data-dir 并在同一地址启动，要求同一密码登录、revision 与配置原文不变、之前的活动记录都在、该 Node 不重新接入且接受新 Server 签发的租约，schema 变化时上一版本拒绝新库且不修改它。不通过则不发布。Check 在存储相关路径变化时运行同一测试（`mise run test:server-upgrade`）。
4. 打包 `meshway-<target>.tar.gz`（`meshwayd`、`meshway`、`meshwayd.service`）与 `meshway-server-<target>.tar.gz`（`meshway-server`、`ip-datasets`、`reset-password`、`meshway-server.service`），以及 Console 静态文件 `meshway-console.tar.gz`；附 `install.sh`，生成 `SHA256SUMS`。x86_64 的 CLI 与 Server 在 runner 上各执行一次，报告的版本必须等于 tag；`export-openapi` 是开发工具，不发布。
5. 创建 GitHub Release（非 draft），说明取 changelog 对应段；版本含 `-`（如 `-alpha.1`）时标记为 prerelease。

`workflow_dispatch` 在任意分支上做同一构建、门槛与打包的演练：版本取 workspace 版本，不检查 tag 与 changelog，资产作为 workflow artifact `release-assets-<sha>` 上传，不创建 Release，也不推送 meshway-rules 转换器。

## 验收记录

Node 系统服务在 OrbStack Ubuntu 24.04 machine（aarch64，systemd 255，systemd-resolved 动态 stub；常驻 `meshway-vm`，见[常驻 systemd 验收 VM](/v2/node/docker-lab#systemd-vm)）上实测，程序为 Release aarch64 产物或同源本地 release 构建。OrbStack 对所有服务注入关闭沙箱的 drop-in，验收时以更高优先级 drop-in 恢复本 unit 的取值，`systemctl show` 确认 `ProtectSystem=strict`、`NoNewPrivileges=yes` 与四项 capability 生效。machine 重启产生新的 `boot_id` 与 network namespace，但与宿主共享内核，不等同物理机冷启动。

| 场景 | 结果 |
| --- | --- |
| `install.sh --artifact` 首次安装 | 0.9 s；daemon 启动到 `READY=1` 20 ms；`/var/lib/meshway`、`/run/meshway` 为 0700，socket 0600 |
| `install.sh --version`（GitHub API＋`GITHUB_TOKEN`） | 8.6 s（含下载）；校验 `SHA256SUMS` 后升级并保留身份 |
| 重复运行／错误校验和 | 内容相同不替换、不重启（MainPID 不变）；`SHA256SUMS` 不符时拒绝安装 |
| `meshway up --enrollment-key @file` | 约 3 s 后 `running`／`dataPlane=ready`／applied；Mesh 名称经 resolved 解析 |
| 重启（Server 可达，三次） | 开机到就绪 1.4–1.8 s，首个状态读取即 `running`（缓存 netmap 已安装）；首次控制请求在网络就绪前失败并按 1 s 退避；无需 `up`；旧 journal 以 `changed_namespace` 归档 |
| 重启（Server 不可达） | 开机到就绪 1.6 s，`running`／`ready`／applied 来自 `netmap.json`，`sync=disconnected`；Mesh 名称、系统解析与 `meshway ping` 可用 |
| `systemctl restart meshwayd` | 1.3 s 内重新 sync，DNS 归属记录跨重启保留 |
| `systemctl restart systemd-resolved`（五次） | 四次无失败采样；一次 Mesh 名称中断 0.6 s 后恢复，期间 `synchronizing`／`suspended` |
| `systemctl stop systemd-resolved`，随后 `start` | 0.3 s 内 `blocked`／`dataPlane=failed`／`lastError=dns`，系统与 Mesh 解析均失败；`start` 后 1.1 s Mesh 解析恢复、`running`／`ready`，无需 `up` |
| `SIGKILL` 后改写记录的 socket device | systemd 5 s 后重启，两个 stale socket 按 inode 回收并重新绑定，记录更新为当前 device |
| `--uninstall`（保留身份）／`--purge` | 无 meshwayd 进程、TUN、DNS 锚点、私有 RPDB 规则或 resolved 链路；`/run/meshway`、unit 与二进制删除，保留 `/var/lib/meshway` 与 `/etc/meshway`；`--purge` 后这些目录与 drop-in 均不存在 |

原始记录 `.meshway/node-lab/s1/`、`.meshway/node-lab/f3/`。

### Server（MIG-2） {#server-validation}

2026-10-22，常驻 `meshway-vm`（Ubuntu 24.04 aarch64，systemd 255）上另起的独立实例（`127.0.0.1:18420`，与 8420 的 dev Server 及现场节点无关），一个同机 control-only Node（独立 data-dir，以 Tag 接入密钥接入该实例）。产物为本分支 `v0.1.0-alpha.21-15-g3f7df74e` 的 aarch64 musl release 构建，按 Release 的打包方式做成 `meshway-server-<target>.tar.gz`＋`SHA256SUMS`，经 `install.sh --component server --artifact` 安装；alpha.20／alpha.21 没有发布 Server，用其 tag 源码构建的程序同样打包模拟。这两个版本不发 `READY=1`，在它们运行期间以 drop-in `Type=exec` 模拟。OrbStack 注入的关闭沙箱 drop-in 与 Node 相同，以更高优先级 drop-in 恢复。

| 场景 | 结果 |
| --- | --- |
| 首次安装（alpha.21）、建库与接入 | 3.1 s；创建 `meshway-server` 用户（无登录 shell），data-dir 0700、文件 0600，属主 `meshway-server` |
| 升级 alpha.21 → 本构建（schema 9 → 9） | 5.9 s：预检、停止、`migrate`（`nothing migrated`）、保留 `.previous`、`READY=1` 与健康检查；同一密码登录、revision、配置原文摘要、活动数与节点不变；Node 不重新接入，3 s 内接受新进程签发的租约（journal 有 `First Node sync answered by this Server process`） |
| 沙箱（`systemctl show` 与 `/proc/<pid>/status`） | `Type=notify`、`NotifyAccess=main`、`RestartPreventExitStatus=78`、`TimeoutStartUSec=2min`、`ProtectSystem=strict`、`ProtectHome`、`PrivateTmp`、`PrivateDevices`、`NoNewPrivileges`、`MemoryDenyWriteExecute` 等全部生效；`CapEff`／`CapBnd`／`CapAmb` 全 0，`Seccomp: 2`，UID 999；进程视角 `/usr/local/bin` 与 `/etc` 只读，只有 data-dir 可写，`/root`、`/home` 不可见，`/dev` 只有伪设备 |
| 同版本重装 | 不替换、不重启（MainPID 不变） |
| 持久阻断 | `user_version` 改为 99 后启动：`status=78`，12 s 后 `NRestarts=0`、`failed`，库哈希不变；改回后正常启动 |
| 迁移失败 | 新程序 `migrate` 退出 1：脚本删除新程序、重新启动原服务（新 MainPID），库与程序不变 |
| `READY=1` 超时（drop-in `TimeoutStartSec=20s`，新程序不发 READY） | 21 s 后自动回退：`.previous` 放回、原服务启动并健康，Node 照常 sync |
| 迁移后启动失败（alpha.20 → 新程序，迁移 8 → 9 成功、服务进程立即退出） | 2.0 s 内回退：迁移后的库改名为 `controller.sqlite.failed-<秒>`，v8 备份放回，alpha.20 启动；登录与配置不变 |
| 回退被拒 | 新程序的 `READY=1` 不来自主进程（超时），但其间已应答 Node sync：脚本不回退、报告 fix forward；随后以修正产物前进升级（服务处于 `activating` 时同样先停止） |
| 升级 alpha.20 → 本构建（schema 8 → 9） | 0.96 s；`Database migrated from schema 8 to 9`，备份 `controller.sqlite.v8-<ms>`，`schema_migrations` 一行；登录、配置、Node 不变 |
| 迁移延长启动超时 | 1.3 GiB 的 v8 库，drop-in `TimeoutStartSec=3s`，由 `systemctl start` 自动迁移：迁移 62.1 s，启动 68.5 s 后 `active`，`NRestarts=0`；迁移期间 `StatusText` 依次为 `… checking the database`、`… copying rule_set_artifacts` 等 |
| `backup` 与恢复 | 停服务后 0.03 s；输出目录 0700、三个文件 0600，主密钥与原文件相同；输出已存在或服务在运行时以 1 拒绝；复制到新 data-dir 后另起实例，原密码登录、网络与 revision 一致 |
| 卸载 | 保留数据：程序与 unit 删除，data-dir／设置／用户保留，重新安装后同一 Server、Node 照常 sync；`--purge`：data-dir、设置、drop-in 与用户均删除 |
| `scripts/ci/server-upgrade.py`（同一 VM，普通用户） | alpha.21 → 本构建、alpha.20 → 本构建（含旧版本拒绝新库）全部检查通过 |

恢复一个较旧的库（迁移延长测试之后放回之前的 v9 库）后，该 Node 的 Node session 不在库中，`sync` 先报 `reconnecting (credentials)`，约 30 s 内重新认证并恢复，未重新接入。证据 `.meshway/validation/mig2/vm/`。

### Server（F8b） {#server-validation-f8b}

2026-10-23，同一 `meshway-vm` 上的独立实例（`127.0.0.1:18420`，与 dev Server 及现场节点无关）与一个同机 control-only Node（独立 data-dir）。程序为本分支 `92330a8c` 的 aarch64 musl release 构建，打包成 `meshway-server-<target>.tar.gz`＋`SHA256SUMS` 经 `install.sh --component server --artifact` 安装；对照组为 main `975e19b5` 的 `install.sh` 与 Server、alpha.20（schema 8）Server。故障产物：W1 让真实 Server 作 shell 的子进程运行（`READY=1` 不来自主进程，unit `TimeoutStartSec=20s`，`LogLevelMax=notice` 使全部日志行丢失）；W3 在 18421 端口服务（健康检查失败），unit 带 `RefuseManualStop=yes`（停止失败）。

| 场景 | 结果 |
| --- | --- |
| 回退依据，日志丢失（W1，Node 在新进程上 sync） | 旧脚本：`served` 已换成 W1 进程的 `boot`、journal 中 0 行，仍回退。新脚本：`served` 变化即拒绝回退（`not rolled back. Fix forward`），`.previous` 保留；随后以修正产物前进升级成功 |
| 回退依据，无 sync（W1，Node 停止） | 新脚本：`served` 不变，回退并重新启动原程序 |
| 停止失败（W3，回退时 `systemctl stop` 被拒） | 新脚本：`did not stop; not rolled back, nothing else was changed`，程序、unit、库与 `served` 不变。旧脚本：照常放回 `.previous` 程序与原 unit，而 W3 进程（同一 MainPID）仍在运行 |
| 停止失败时卸载（`--uninstall --purge`） | 新脚本：`nothing was changed`，unit 仍 enabled，data-dir、设置与用户都在。旧脚本：删除 unit 与程序后报 `still active`，进程仍在运行 |
| 错配主密钥的 v8 库（alpha.20 → 本分支） | 新脚本：`migrate` 以 78 失败、`Migration state: unpublished`，文件头仍为 8、库 SHA-256 不变、无备份或 staging 文件，新程序未安装；放回正确主密钥后 alpha.20 启动，同一密码登录、revision 不变。旧程序与旧脚本：先发布 schema 9 再报主密钥不匹配，脚本却输出 `the database is unchanged`；放回正确主密钥后 alpha.20 以 `Unsupported` 失败，需手动放回备份 |
| 正确主密钥的 v8 → 9 升级 | 0.65 s，`Database migrated from schema 8 to 9`，备份 `controller.sqlite.v8-<ms>`，登录与配置不变 |

证据 `.meshway/validation/f8b/vm/`，构建脚本与故障产物生成脚本 `.meshway/validation/f8b/vm-artifacts.sh`。

### OrbStack 与 WARP 出网的路由 MTU {#orbstack-warp}

常驻验收 VM `meshway-vm` 的环境 workaround，不是产品要求。OrbStack（2.2.4）machine 的出网 NAT 不保留 DF：VM 发出的带 DF 外层报文在 Mac 上照常转发；Mac 经 Cloudflare WARP（2026.7.1343，MASQUE，`utun` MTU 1,300）出网，超过 1,300 B 的报文在 Mac 分片、由 Cloudflare 重组后送达对端（对端 `IpReasmReqds` 为 0）。VM 的路由 MTU 仍是 eth0 的 1,500，认证路径 MTU 探测因此报 `1500/1386`，满长 Mesh 报文全部被分片，见[路径 MTU 限制](/v2/node/connectivity#path-mtu)。

VM 以 systemd-networkd drop-in 只给 DHCP 默认路由加 MTU，同网段（lab 容器 `192.168.139.0/24`）不受影响：

```ini
# /etc/systemd/network/10-netplan-eth0.network.d/meshway-route-mtu.conf
[DHCPv4]
RouteMTUBytes=1300
```

| 项（2026-10-15，alpha.12，VM→it7） | 默认（路由 1,500） | 默认路由 MTU 1,300 |
| --- | --- | --- |
| `meshway status` 通道 | `1500/1386` | `1300/1186`（lab 同网段对端仍 `1500/1386`）；machine 重启后保持 |
| Mesh UDP 30 Mbit/s 乱序 | 外层 1,342 B：18,748 个中 10,541 个乱序；外层 ≤1,292 B：0 | 20,423 个中 11 个 |
| 内层 TCP | `reord_seen` 数千，cwnd 20–30 段 | MSS 1,134，`reord_seen` 83 |
| Mesh TCP 单流（Mbit/s，与改动穿插） | 14.4–19.2（4 次） | 49.4–62.0（6 次；同期 `-M 1140` 49.7–56.8） |

吞吐受 WARP 上行（约 95 Mbit/s，满载丢 1–2%）与 VM 内层 cubic 的丢包敏感度限制，同期 Mac 原生单流 TCP 81–116 Mbit/s；VM 直接对公网的 TCP 由 OrbStack 在 Mac 终结，其 0 重传不代表路径无丢包，不能作 Mesh 基线。证据 `.meshway/validation/m5/`。
