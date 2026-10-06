# 验收范围

本页是 Node 隔离验收范围的唯一定义位置。其他页面只描述行为与限制，并引用这里的状态。入口命令、参数与报告位置见[开发服务、测试与环境](/v2/development#隔离-linux-验收与镜像)。

## 状态含义

| 状态 | 含义 |
| --- | --- |
| 当前 wire 已通过 | 使用当前数据面 wire（版本见[受保护 circuit](/v2/node/dataplane#候选受保护-circuit)）完成整组验收 |
| 历史通过未重验 | 曾在先前 wire 或先前实现上通过整组验收；结论不自动扩展到当前 wire |
| 未验收 | 没有整组通过证据；不能声明已支持 |

库测试、单独内核测试、镜像构建成功或常驻 lab 的部分功能检查都不替代对应行的整组结论。

## 验收矩阵

| 入口 | 承载 | DNS | NAT | 覆盖行为 | 状态 |
| --- | --- | --- | --- | --- | --- |
| `test:tun:linux` | — | — | — | 真实 TUN 设备创建与双栈包 I/O | 历史通过未重验 |
| `test:packet:linux` | Native TCP | — | — | A→R→B 内层双栈 UDP echo、R 只转发受保护帧、原路径回复 | 当前 wire 已通过 |
| `test:runtime:linux` | Native UDP 静态端点 | off | — | daemon 整组：下文[共同场景](#runtime-共同场景) | 当前 wire 已通过 |
| `test:runtime:linux` | Native TCP | off | — | daemon 整组：共同场景 | 历史通过未重验 |
| `test:runtime:linux` | Native UDP 静态端点 | resolv-conf | — | 共同场景＋[DNS 场景](#dns-场景) | 历史通过未重验 |
| `test:runtime:linux` | Native UDP 静态端点 | resolved | — | 共同场景＋DNS 场景 | 历史通过未重验 |
| `test:runtime:linux --traversal nat` | Native UDP 协调穿透 | off | 双 IPv4 端口保持型 SNAT | 共同场景＋[穿透场景](#穿透场景) | 历史通过未重验 |
| `test:runtime:linux --traversal nat --traversal-fallback` | Native UDP＋A–R Native TCP 退路 | off | 双 IPv4 端口保持型 SNAT | 共同场景＋穿透场景＋同路径 TCP 退路 | 历史通过未重验 |
| `test:runtime:linux --traversal nat` | Native UDP 协调穿透 | resolved | 双 IPv4 SNAT＋TCP 退路 | 整组组合 | 未验收；已知缺口为共享观察切换后的通道重新获取或结果发布停滞 |
| [常驻三节点 lab](/v2/node/docker-lab) | Native UDP 显式端点 | resolved | — | 六向双栈系统解析／echo、路径诊断、关闭插桩的同路径 TCP 吞吐对照 | 当前 wire 已通过（共享 bridge，不构成 NAT 证明） |

以下范围未验收：外层 IPv6 NAT、端点相关映射、映射轮换、NAT 与 resolv-conf 的组合、VLESS／Trojan 承载上的 daemon 包流、TLS／WS 材料轮换、多路径切换、出口转发／NAT、完整 TCP conntrack、MTU／分片、整机重启与系统 resolver 服务停机间隙、userspace／SOCKS5。

## 入口

### TUN

```sh
mise run test:tun:linux
```

宿主交叉编译，容器仅运行预编译测试；使用固定 digest 镜像、独立 network namespace，只授予 TUN device 与 `NET_ADMIN`。报告位置见 [development](/v2/development#数据、报告与回收)。

### 受保护包流

```sh
mise run test:packet:linux
```

宿主编译 `meshway-packet-fixture`，使用 Server、A、R、B 四个独立容器／network namespace 和一个 internal bridge。Server 角色运行真实 Router／HTTPS／生成管理客户端；各 Node 独立持钥接入、取得执行快照，经正式 Native TCP provider 连接。Node 不持有管理 token 或 Server TLS 私钥。报告位置见 [development](/v2/development#数据、报告与回收)。

### daemon 与内核资源

```sh
mise run test:runtime:linux --build-only
mise run test:runtime:linux --artifacts /absolute/private/bin --transport native-udp --dns-backend off
```

宿主编译实际 `meshwayd`、`meshway` 和验证 actors。Node 经 CLI 上线并由真实 Server 批准；验证程序使用 OS UDP／TCP socket，包处理由 daemon 的正式运行组件执行。容器使用独立 IPv4／IPv6 internal bridge，不发布宿主端口，不使用 host network 或 privileged。参数与报告位置见 [development](/v2/development#隔离-linux-验收与镜像)。

#### runtime 共同场景

- 关闭系统 DNS 的 Mesh-only 网络，强制 A→R→B 及原路径回复。
- 内层双栈 UDP／TCP；同一 socket 跨需求添加；至少 125 秒持续续租；独立 UDP 流停止发包后的入站活动续租。
- Access 撤销／恢复；中转取消 IPv6 地址后继续转发 IPv6 密文；拒绝路由保留；离线 `down` 清理。
- 控制中断恢复：关闭 Node HTTP 接纳直至实际 instance／execution 到期，再恢复联系；无需再次 `up`，原身份、进程和 operator revision 保持，取得新 instance、重新 applied 并上报 State，以新的 OS socket 恢复原路径包流，过期的旧 TCP／cipher 被拒绝。
- `meshway ping`：hostname／双栈 Mesh IP 选择、direct 与有序 via、独立 ICMP 权限、用户与 Node 共享任务及独立取消、SIGINT、控制中断后的订阅终态、新 instance 下重新探测。
- 源、终点的新交付计数及中转 TUN 的零明文交付分别记录。

单独的内核测试另检查同目的地址的普通／受保护 socket、旧池拒绝边界、journal 恢复、外部同优先级规则冲突，以及 HTTPS bootstrap 在显式 down 前后的联系。

#### DNS 场景

```sh
mise run test:runtime:linux --artifacts /absolute/private/bin --transport native-udp \
  --dns-backend resolved --dns-image-manifest /absolute/private/os/image-manifest.json
```

覆盖真实 `getaddrinfo`、短名称、可见性 REFUSED、分域 UDP／TCP、系统 UDP→TCP 截断重试、进程重启、长时包流、失效阻断及 DNS／接口恢复。DNS 场景在测试 namespace 内使用私有 mount namespace 与私有 D-Bus／resolver，不改写 Docker 的 resolver bind 源；固定 OS 资产见 `dev/dns-fixture/README.md`。

#### 穿透场景

```sh
mise run test:runtime:linux --artifacts /absolute/private/bin --transport native-udp --traversal nat \
  --traversal-image-manifest /absolute/private/os/image-manifest.json
```

- 两个独立 IPv4 端口保持型 Linux conntrack SNAT；UDP 发布端点为空，STUN、互探及认证使用实际承载 socket。
- 内层双栈 UDP／TCP 强制沿 A→R→B，持续与纯接收业务跨越至少 125 秒及原需求期限。
- `--traversal-fallback` 仅预授权 A→R 的 Native TCP 拨号，R→B 保持 Native UDP；TCP 入口初始不可达，业务持续 30 秒后开放该入口并阻断精确 A–R UDP 地址对；原 TCP socket、两个内层 TCP circuit 的 CID 与完整路径跨切换保留。
- State、双方接受的固定 offer、双侧抓包、conntrack、防火墙计数和应用回包共同构成证据。拓扑与固定 OS 资产见 `dev/traversal-fixture/README.md`。
