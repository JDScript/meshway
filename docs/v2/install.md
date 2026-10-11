# Linux 安装与系统服务

本页定义 Node（`meshwayd` 与 `meshway`）在 Linux 上作为 systemd 系统服务的安装、升级与卸载。daemon 参数与运行行为见[网络模式与本机运行](/v2/node/runtime)，发布产物由 [Release 工作流](#发布产物) 生成。

## 要求

| 项 | 要求 |
| --- | --- |
| 架构 | x86_64 或 aarch64；产物为静态链接的 musl 程序，不依赖发行版 libc |
| init | systemd（`/run/systemd/system` 存在），unit 使用 `Type=notify` |
| 权限 | 安装与 CLI 需要 root；daemon 以 root 运行，但 capability 限于下表 |
| 设备 | `/dev/net/tun` |
| DNS | 默认 `--dns-backend resolved`：需要运行中的 systemd-resolved，且 `/etc/resolv.conf` 指向其动态 stub（`/run/systemd/resolve/stub-resolv.conf`）；见[系统 DNS 安装](/v2/node/dataplane#系统-dns-安装) |
| 下载 | 在线安装需要 `curl`、`sha256sum`、`tar`；私有仓库需 `GITHUB_TOKEN`（releases 读权限） |
| Server | 可达的 HTTPS Server origin；数字 loopback 可用 HTTP |

## 安装

```sh
curl -fsSLO https://github.com/JDScript/meshway/releases/download/v0.1.0-alpha.1/install.sh
sudo sh install.sh --version v0.1.0-alpha.1 --server https://controller.example.com
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

## systemd unit

unit 原文为仓库 `dist/systemd/meshwayd.service`，随每个 Release tarball 发布。

| 项 | 取值与理由 |
| --- | --- |
| 启动顺序 | `Wants=`／`After=network-online.target`，另在 systemd-resolved 与 D-Bus 之后启动 |
| 就绪 | `Type=notify`、`NotifyAccess=main`：本机 IPC socket 绑定后发送 `READY=1`，开始停止时发送 `STOPPING=1`。就绪只表示可接受 CLI 命令，不表示已 running |
| 命令 | `meshwayd --data-dir /var/lib/meshway $MESHWAYD_OPTIONS`；`EnvironmentFile=/etc/meshway/meshwayd.env` 提供 `MESHWAY_SERVER_URL` |
| 重启 | `Restart=on-failure`、`RestartSec=5s`；`KillMode=mixed`，SIGTERM 只发给 daemon，由它取消并等待自有 nft／conntrack 子进程；`TimeoutStopSec=45s` |
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

## 重启与离线

- 期望状态持久保存在 `/var/lib/meshway`：开机前为 up，开机后服务自动重新取得运行资格，不需要再次 `meshway up`；显式 `down` 后保持下线。
- 开机后 Server 不可达时，daemon 从 `netmap.json` 载入签名租约内的 netmap 并安装数据面；租约到期即全部撤回（fail closed），见[离线运行](/v2/node/netmap-design)。
- 上一 boot 的 Underlay journal 不属于当前 namespace，按[持久与重建](/v2/node/runtime#持久与重建)归档为 `underlay.json.stale-*` 后重新分配。

- 开机时 systemd-resolved 由 unit 排序先于 daemon 启动；DNS 归属记录随 `/run` 清空，按当前 resolved 实例重新安装。

### 当前限制

- **systemd-resolved 重启或停止后不会自动恢复。** DNS 后端把 resolved 的 D-Bus 唯一名作为归属证明；`systemctl restart systemd-resolved`（含软件包升级触发的重启）或 `stop` 后，daemon 在 1 秒内进入 `blocked`／`dataPlane=failed`／`lastError=dns`，并按"必需组件失败即全撤"撤回整个数据面（Mesh IP 也不可达）。resolved 恢复后，`systemctl restart meshwayd`、`meshway down`／`up` 仍报归属冲突；当前只有重启机器（清空 `/run/meshway`）能恢复，此时 `install.sh --uninstall` 的 `down` 也会失败。系统解析本身不受 Meshway 影响。
- 系统 hosts 只在 Underlay 打开时读取一次：运行中修改 `/etc/hosts` 中 Server／承载名称的映射，须重启服务才生效。
- 每次开机归档一份 `underlay.json.stale-*`（约 4 KiB），目前不自动清理。

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
meshway 0.1.0-alpha.1 (v0.1.0-alpha.1-0-g1234abcd)
target:   aarch64-unknown-linux-musl
ipc:      6
wire:     6
compiler: meshway-netmap-1
```

`--json` 输出同名字段（`version`、`git`、`target`、`ipc`、`wire`、`compilerVersion`）。`wire` 是受保护 circuit 版本，同一路径上的节点必须一致；`compilerVersion` 是 netmap 编译契约。该命令不联系 daemon。

## 卸载

```sh
sudo sh install.sh --uninstall           # 保留身份与设置
sudo sh install.sh --uninstall --purge   # 同时删除 /var/lib/meshway 与 /etc/meshway
```

卸载先执行 `meshway down`：显式 down 才会移除 TUN、DNS 锚点与负向 capture 规则；仅停止服务会保留它们供下次启动。down 失败时脚本停止并保留运行中的服务。随后停止并禁用服务，删除 unit、两个二进制和 `/run/meshway`。不带 `--purge` 时保留 Node 身份（`/var/lib/meshway`）与 `/etc/meshway`，重新安装后仍是同一个 Node；`--purge` 后再次安装需要重新接入。

## 发布产物

tag `v*` 触发 `.github/workflows/release.yml`：

1. tag 必须等于 workspace `Cargo.toml` 的 `version` 加 `v` 前缀，且本页同级的 [changelog](/v2/changelog) 有同名 `## <tag>` 段，否则失败且不构建。
2. 以 `workflow_call` 复用 `build.yml`，交叉编译两个 musl target 的 release 程序；`meshway version` 的 git 描述来自 `git describe --tags`。
3. 打包 `meshway-<target>.tar.gz`（`meshwayd`、`meshway`、`meshwayd.service`），附 `install.sh`，生成 `SHA256SUMS`；x86_64 CLI 在 runner 上执行一次，报告的版本必须等于 tag。
4. 创建 GitHub Release（非 draft），说明取 changelog 对应段；版本含 `-`（如 `-alpha.1`）时标记为 prerelease。

## 验收记录

2026-10-11，OrbStack Ubuntu 24.04 machine（aarch64，systemd 255，安装 systemd-resolved 并使用其动态 stub），程序为本分支 `aarch64-unknown-linux-musl` release 构建。OrbStack 对所有服务注入关闭沙箱的 drop-in，验收时以更高优先级 drop-in 恢复本 unit 的沙箱取值，`systemctl show` 确认 `ProtectSystem=strict`、`NoNewPrivileges=yes` 与四项 capability 实际生效。machine 重启产生新的 `boot_id` 与 network namespace，但与宿主共享 OrbStack 内核，不等同物理机冷启动。节点以 Tag 接入密钥加入开发网络，对端为常驻 lab 的 `meshway-lax`。

| 场景 | 结果 |
| --- | --- |
| `install.sh --artifact` 首次安装 | 0.9 s 完成；daemon 启动到 `READY=1` 20 ms；目录 `/var/lib/meshway`、`/run/meshway` 均为 0700，socket 0600 |
| 重复运行／错误校验和 | 内容相同不替换、不重启（MainPID 不变）；`SHA256SUMS` 不符时拒绝安装 |
| `meshway up --enrollment-key @file` | 约 3 s 后 `running`／`dataPlane=ready`／applied；Mesh 名称经 resolved 解析 |
| 重启（Server 可达） | 开机到 daemon 就绪 1.4 s；首个状态读取即为 `running`（缓存 netmap 已安装），2.5 s 内 sync 连接；无需 `up`；旧 journal 以 `changed_namespace` 归档 |
| 重启（Server 不可达，hosts 指向黑洞地址） | 开机到就绪 1.6 s，`running`／`ready`／applied 来自 `netmap.json`，`sync=disconnected`；Mesh 名称解析、系统解析和 `meshway ping` 均可用 |
| `systemctl restart meshwayd` | 1.3 s 内重新 sync，DNS 归属记录跨重启保留 |
| `systemctl restart systemd-resolved` | 系统解析不中断；Mesh 名称与 Mesh IP 立即失败且不恢复，见[当前限制](#当前限制) |
| `systemctl stop systemd-resolved` | 系统与 Mesh 解析均失败（fail closed），数据面撤回；`start` 后不恢复，重启机器后恢复 |
| 卸载 | 见下文命令；`down` 后无 TUN、DNS 锚点、私有路由规则，unit 与二进制删除 |

lab 对端 `meshway-lax` 位于 OrbStack Docker bridge 后（对 machine 单向 NAT），只能由 lax 拨入；该边上 lax 每 4–9 s 换一个源端口重新建立 Native UDP 通道，`meshway ping` 与 OS ping 有 60–80% 丢包。这是连接组件在此拓扑下的行为，与安装无关，另行跟踪。原始记录位于仓库私有目录 `.meshway/node-lab/s1/`。
