# 网络模式与本机运行

## 启动与接入

当前 `meshwayd`／`meshway` 可完成 Server 控制接入。以下示例假定已有当前版本 Server 位于 `127.0.0.1:8430`，Console 位于 `127.0.0.1:3001` 并连接该 Server：

```sh
mise run build:node
install -d -m 700 .meshway
./target/debug/meshwayd \
  --data-dir "$PWD/.meshway/node" \
  --server http://127.0.0.1:8430 \
  --console-origin http://127.0.0.1:3001
```

另开终端操作同一 daemon：

```sh
./target/debug/meshway --socket "$PWD/.meshway/node/control.sock" up
./target/debug/meshway --socket "$PWD/.meshway/node/control.sock" status --watch
./target/debug/meshway --socket "$PWD/.meshway/node/control.sock" status --json
./target/debug/meshway --socket "$PWD/.meshway/node/control.sock" down
```

首次 `up` 输出公开的浏览器批准链接；在 Console 登录并选择网络批准后，daemon 自动兑换身份、认证、取得实例、保存配置和发送 saved 回执。`up` 返回批准链接时只表示正在等待批准。`controlReady` 表示控制流程就绪，`dataPlane=notImplemented`、`applied=null` 明确表示尚无 TUN／SOCKS5 或实际包流。

`meshwayd` 前台运行，SIGINT／SIGTERM 执行关闭与 join。进程退出保留期望状态：退出前为 up，重启后自动重新取得运行资格；显式 down 后重启继续保持下线。CLI 的 Ctrl-C 仅停止等待，不撤回已由 daemon 接纳的持久操作。

| 参数 | 行为 |
| --- | --- |
| daemon `--data-dir` | 私有状态目录，默认 `/var/lib/meshway`；父目录须存在，Unix socket 位于其 `control.sock` |
| daemon `--server`／`MESHWAY_SERVER_URL` | 必须明确指定 Server；远端要求 HTTPS，数字 loopback 可使用 HTTP |
| daemon `--console-origin` | Console 独立部署时指定；省略时使用 Server origin |
| daemon `--hostname` | 首次接入建议名；省略取系统 hostname 的首个 DNS label，既有业务名称仍由 Server 管理 |
| CLI `--socket` | 默认 `/var/lib/meshway/control.sock`，可显式选择本机 daemon |
| CLI `--json` | 输出类型化公开状态；不包含 Node 私钥、兑换码或会话 token |

默认目录适用于 Linux 系统服务；本地开发或其他 Unix 环境可像示例一样显式指定私有目录。安装参数不构成本机业务配置覆盖源。

daemon／CLI 共用[运行日志参数](/v2/logging)，日志位于 stderr；CLI 的 `--json` 状态结果位于 stdout。

## 两种网络模式

| | 系统 TUN | Userspace networking |
| --- | --- | --- |
| 应用接入 | OS socket／路由把受管流量送入 TUN | 无 TUN，首版应用通过本机 SOCKS5 使用 Mesh |
| TCP/IP 栈 | 本机应用通常使用 OS 栈 | 相应流量由用户态栈处理 |
| 系统条件 | TUN／路由权限，以及所需 DNS、出口转发／NAT 资源 | 用户态 socket／栈资源；不自动接管任意宿主应用 |
| Meshway 逻辑 | 两种模式共用身份、准入、连接、加密与完整路径规则 | 同左 |

```text
OS 应用／网络栈 ⇄ TUN ───────┐
                            ├⇄ dataplane ⇄ 获准连接／transport ⇄ peer
本机 SOCKS5 ⇄ netstack ─────┘
```

统一业务数据面采用 **L3 IPv4／IPv6 IP 包**。TUN 直接交付 IP 包，SOCKS5 经 netstack 接入同一数据面，共用准入、端到端保护、路径执行和回复规则。当前目标不包含 Ethernet 透明桥接、MAC 学习或二层广播域。

内层 IP 业务与外层承载独立：TCP／UDP 业务不据此强制选择同名 transport，实际承载仍由配置与当前执行许可约束。L3 范围不自动扩展控制面允许的业务协议；Mesh 封装、流／报文适配、MTU／分片与可靠性策略仍待定。

系统 TUN 模式不隐含内核模块、内核内加密或 eBPF。无 TUN 的纯中转也不等于已提供用户态端点栈。Userspace 首版应用入口为本机 SOCKS5，不包含应用内 socket SDK 或公网代理服务。

SOCKS5 的命令范围、监听地址／端口、本机访问认证、域名解析和 UDP 关联生命周期仍待定；这里只确定入口形状，不声明代理已经可用。本机 SOCKS5 使用当前网络的业务权限、DNS 和路由，不能作为绕过准入的直拨通路。

模式的默认选择与云端字段尚待定。本机权限或 TUN 安装失败不能静默切模式后报告原目标 applied；两种模式的出口能力分别验证。实现差异不产生服务器／终端节点类别，同一设备一次只启用一张网络。

## CLI 与 IPC

Linux 首版每台机器运行一个系统级 `meshwayd`，同一时间只启用一张网络。CLI 连接该服务；不为每个本机用户创建独立 daemon 或自动复制 Node 身份。当前控制阶段使用 owner-only 目录与 `0600` socket，在解析请求前后核验内核提供的 peer UID，只允许 root 或 daemon 同 UID。更细的操作员授权、TUN 权限和安装服务策略留待网络运行接线。

| 命令／操作 | 行为／状态 |
| --- | --- |
| `meshway up` | 首次浏览器接入，或用保存的身份重新认证并取得运行资格 |
| `meshway down` | 停止当前网络并终止相关资格；daemon 与私有身份保留 |
| `meshway ping <target>` | 尚未实现；需由 daemon 执行获准测量，报告真实路径／来源／结果 |
| `meshway status` | 区分进程、控制联系、saved／applied 和实际通道状态 |
| 切网 | 尚未实现；先停止旧网络、完成隔离清理，再运行新网络 |

CLI 通过 Unix domain socket 发命令、读结果；状态由 daemon／组件的唯一 owner 持有。`meshway-ipc` 是唯一的请求／响应定义，使用版本化 JSON 与长度前缀，单条消息上限 64 KiB。每个连接处理一个请求，连接数、请求等待和命令队列均有上限。

up/down 按本机期望状态 revision 执行 CAS，并保留最后一次变更的请求身份以恢复不确定结果。本机 revision 与 Server 配置 revision 分开。已接纳命令的持久化由协调器完成，CLI 断开不会撤回它；读取 status 不写入状态或生成测量。

Server HTTP 等待与本机命令处理独立。配置查询通常每 5 秒，实例续租与控制报告通常每 20 秒；故障采用有界退避，期限由控制组件独立校验。down 先持久保存本机下线意图，再停止当前资格并尽力释放远端实例；离线时也不能把旧 lease 展示为有效。当前 IPC 不承载业务包。

daemon 独占目录锁，启动时记录 socket 的文件身份；只有身份匹配且确认无监听者的旧 socket 可清理。未知文件、符号链接、活跃 socket 和损坏的状态文件都保留并报错。

CLI 生命周期不承载 Node 身份或 lease，不收集账户密码、不取得浏览器会话。业务 listener／DNS／策略无本地覆盖源；获准路由方案切换仍是使用选择，具体入口待定。`down` 不等于云端永久删除。

ping 的默认服务范围和 Node 凭据诊断合同仍需补齐。不能冒用 Console 会话调用观察接口，也不能用 HTTP 耗时或未建立路径的等待时间伪造 RTT／丢包。

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
取得私有目录／IPC → 启动加载 provider → 接受上线目标
  → 持钥接入与执行目标 → 校验并保存（saved）
  → 先准备底层保护，再安装组件 → applied／failed 与真实 State
```

宿主协调一个活动网络及有效执行上下文，组件拥有其任务、buffer、socket 和系统资源。具体运行库不重复编译网络策略；saved／applied／业务连通的含义见[配置应用](/v2/configuration-reference/deployment)。

撤回、期限、down、切网独立于挂起的 HTTP 生效。停止准入后取消并等待任务退出，再释放自己的资源；保留必要 capture／sink 边界，普通业务不能因目的地址与 Server 相同而获得底层豁免。

崩溃恢复只处理有可验证归属的 journal，不恢复旧流。未知格式或不明归属不能作为删除宿主规则的依据。
