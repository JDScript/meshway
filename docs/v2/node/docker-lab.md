# 三节点 Docker 开发环境

三个常驻 Node 共用已构建的 Linux 二进制，各自持有独立状态卷。Docker Compose 只运行程序；Rust 编译由宿主机的固定工具链执行。

常规检查、隔离包流验收、开发服务重置和各类环境的归属／回收入口统一见[开发服务、测试与环境](/v2/development)。

## 节点与持久状态

| 容器 | hostname | Named volume |
| --- | --- | --- |
| `meshway-lax` | `meshway-lax` | `meshway-lax-state` |
| `meshway-sha` | `meshway-sha` | `meshway-sha-state` |
| `meshway-nyc` | `meshway-nyc` | `meshway-nyc-state` |

卷挂载在 `/var/lib/meshway`，保留 Node 密钥、接入请求、Server 绑定、缓存配置和 up/down 意图。容器停止、重启或替换程序均复用原卷。Ubuntu 24.04 工具镜像由 digest 固定的官方基础镜像和校验 SHA 的 snapshot 软件包构建，按不可变 image ID 部署；程序目录只读挂载。每个容器限制为 2 CPU、512 MiB 内存、64 个进程（与 L2／L3 harness 同配额，便于对照），使用独立 bridge 网络。

容器根文件系统可写，包含 `ping`（IPv4／IPv6）、`traceroute`、`iperf`、`iperf3`、`ip`、`wg`（`wireguard-tools`）、`dig` 和 `curl`。`wg` 用于[隔离性能参照](/v2/node/performance#内核-wireguard-参照)，需要 engine 内核提供 WireGuard；镜像不包含 wireguard-go 或 Go 工具链。writable layer 的修改在同一容器重启后保留，重建容器时重新来自锁定镜像；Node 身份另由持久卷保存。

Compose 显式使用 `--network-mode tun`，先 `cap_drop: ALL`，再声明本 namespace 的网络、私有 mount 初始化和系统服务所需 capabilities。`SYS_ADMIN` 只用于创建 private-propagation mount namespace，daemon 与服务子进程启动前从其 bounding set 移除；`SYS_PTRACE` 用于跨 resolved UID 验证真实进程 namespace。保持 `no-new-privileges`，不使用 privileged 或 host network。

- PID 1 owner 在私有 mount namespace 中管理 D-Bus、resolved 和 daemon，逆序停止并 join。
- 容器自己的 `/etc` 副本保存在 writable layer，隐藏 Docker 管理的网络文件 bind，原 bind 源不被写入；`/run` 使用容器私有 tmpfs。
- resolved 提供动态 stub 与 search domains，启动时核对 resolve1 owner PID 和监听契约。bootstrap DNS 显式使用 `1.1.1.1:53`，系统基线上游独立声明为 `1.1.1.1`／`8.8.8.8`。
- 源码修改不代表常驻容器已经部署新程序／参数，实际运行以 versions／status 及接口检查为准。

Node 使用开发服务的对外 Server 地址，批准链接指向本机 Console，地址见[本机服务](/v2/development#本机服务)。以其他入口启动 Server 时须配置 [Console origin](/v2/server#运行参数)，否则 Node 如实显示缺少批准链接。身份绑定仍按[Node 控制组件](/v2/node/control)检查。

## 首次启动

需要本机 Docker 和当前 Server／Console 可用。支持 Linux arm64／amd64 Docker engine；构建任务按 engine 架构选择 Rust musl target，并核对产物的 ELF 架构。

```sh
mise run node:lab image-build
mise run build:node:linux
mise run node:lab deploy
mise run node:lab versions
mise run node:lab status
```

`build:node:linux` 的 Cargo Zigbuild／Zig 由 `mise.toml` 和 `mise.lock` 固定，只在该任务激活。复用 `target/node-linux` 缓存；下载 Rust target 后，在宿主机交叉编译一次 daemon 和 CLI。

`image-build` 独立构建 Ubuntu 工具与 DNS userspace：校验锁定索引／deb 摘要，使用普通 Docker builder 在无网络构建步骤安装。镜像清单与选择保存在 `.meshway/node-lab/images/` 和 `selected-image.json`，源码、平台、label 或 image ID 不符则拒绝部署；deploy 不下载或构建镜像。

`deploy` 使用 `docker compose up --no-build`，校验产物后启动或更新三个容器，等待本机 IPC 健康检查。初始期望状态为 down；执行 up 后按链接在 Console 中选择网络批准：

```sh
docker exec meshway-lax meshway up
docker exec meshway-sha meshway up
docker exec meshway-nyc meshway up
```

三个 Node 都批准到 `.meshway/dev-login.json` 指定的 Development 网络后，应用可重复的开发运行配置：

```sh
mise run node:lab configure
mise run node:lab status
docker exec meshway-lax ip address show
```

`configure` 使用生成 SDK，核对实际 CLI 身份、Server 注册材料和容器归属，再按当前 revision 一次 CAS 提交：为三节点选择 Native UDP 并配置 `lab-udp` listener（`0.0.0.0:35000`），添加仅覆盖这三个 Node 的 `icmp:*` Access 规则。保留完整 Server DNS 配置，包括 MagicDNS、上游、路由和 search domains；发布当前 Docker bridge 地址作为显式端点。其他业务按既有 Access／topology／routing 执行。Development 初始 Access 为 deny，TUN、地址观测或 applied 本身不授予节点间业务权限。

Ubuntu 的 resolved 后端承担实际系统 DNS 安装。Server 提供 STUN 发现服务时，同一 UDP listener 的真实映射独立上报到 State；显式 bridge 端点和 STUN 映射分别保留来源。三个容器共用宿主出口，通常观测到相同的公网 IP 和不同的端口；不将 HTTP 代理来源地址作为映射观测。

`icmp:*` 同时约束 OS `ping` 的 ICMP echo 包流与 [`meshway ping` 的独立诊断](/v2/node/diagnostics)。首次访问需要完成需求发布与认证建连，初始 echo 可能在收敛期间超时；诊断命令将 preparing 与探测 RTT 分开展示，不能只以 TUN 存在判定可达。

也可以进入容器使用同一 CLI：

```sh
docker exec -it meshway-lax sh
meshway status --json
```

## 更新与核对

```sh
mise run node:lab image-build
mise run build:node:linux
mise run node:lab deploy
mise run node:lab configure
mise run node:lab versions
```

构建产物按不可变 release 目录保存在仓库公共目录 `.meshway/node-lab/releases/`；每份 `build.json` 记录 Git revision、dirty 标记、Rust source digest、target 和二进制 SHA-256。构建期间 Rust 源码变动则不发布该产物。`versions` 以 IPC Status 的真实 daemon processId 核验 `/proc/<pid>/exe`、启动时间、namespace 和只读 release，另报告实际 image ID；PID 1 是系统服务 owner，不用它的 executable 冒充 daemon。

程序更新由 Compose 重建需要更新的容器，身份卷继续保留。对需重建的已 up TUN 节点，部署器先通过 CLI down 清理旧 namespace 的所属资源，再重建并恢复原 up 意图；失败保留部署进度，不能把旧 namespace journal 当作新 namespace 的资源证明。唯一例外是 down 后 daemon 报告 `phase=blocked`、`dataPlane=failed`、`lastError=underlay`：该进程的 underlay 从未启动，namespace 内没有它所属的内核资源，重建即销毁该 namespace，新 daemon 仍按自己的 journal 规则（见[运行](/v2/node/runtime)的崩溃恢复）判断能否启动。初始 down 节点保持下线。同一配置未重建的容器不下线。重建可能改变 bridge IP，因此随后运行 `configure` 更新 Server 中的显式端点。`deploy` 不执行 Rust 编译或构建镜像。

## 状态、日志与停止

```sh
mise run node:lab status
docker logs --tail 100 -f meshway-lax
docker exec meshway-lax meshway down
mise run node:lab stop
```

`down` 保存业务下线意图。`node:lab stop` 通过同一个私有 `deploy.json` 保存原意图；TUN 先 down 确认清理，再等待容器正常停止。随后 `node:lab deploy` 核验停止证明、容器及状态卷身份，恢复原 up／down 意图；已完成的受管停止允许更新 release 或 Compose 配置。未知的已停止容器、卷替换或未完成计划冲突保留现场，不能靠清掉 journal 绕过。未完成 stop 应先重试 stop；未完成 deploy 应按原计划重试 deploy。

容器日志使用 JSON 和 Docker local driver，每个容器轮转保留最多三个 10 MiB 日志文件；字段规则见[运行日志](/v2/logging)。管理脚本核对精确资源名和所有权标签，遇到同名外部资源会停止操作。

健康检查表示 daemon 的本机 IPC 可响应；实际安装要求 `networkMode=tun`、`dataPlane=ready`、`phase=running` 和非空 `applied`，同时核对容器内真实 TUN 接口及地址。STUN 候选另查 Server State 的 socket、origin 和有效期；无观测保持为空。长时包流／双 NAT 的隔离验收见[验收范围](/v2/node/validation)，这张共享 bridge 的常驻环境不构成双 NAT 证明。SOCKS5 尚未实现，三个地区名称仅作为开发节点标识，不证明真实地理部署。
