# 连接应用、观测与地址

本切片把已保存的连接意图交给 Agent 运行层：开启获授权 listener、主动连接配置的对端，并通过实际加密邻接探测形成运行观测。`--apply-connections` 保持空的业务路径授权。另行启用的 [托管 Mesh IPv4 数据面](/managed-dataplane) 才会把路由与独立授权交给 TUN；建立邻接本身不开放业务 IP 流量。

## 四个状态维度

| 维度 | 数据来源 | 含义 |
| --- | --- | --- |
| 控制面联系 | 节点心跳 | Agent 最近联系过 controller |
| 配置已保存 | revision / digest ACK | Agent 校验并持久保存了该配置 |
| 连接配置已应用 | Agent 运行观测 | 期望的 listener 与连接管理任务已经安装 |
| 邻接已认证 | 实际 Noise 通道及新鲜邻接探测 | 对端密钥通过认证，并成功完成受限控制 nonce 往返 |

配置应用成功时，某个对端仍可能处于等待或不可达状态；空配置可以成功应用而没有任何连接。观测来自通过节点凭据认证的 Agent，controller 校验其配置范围和新鲜度，不将它描述成独立的远程证明。

运行观测与存储 ACK 使用不同 API；报告、会话 epoch 变化都不增加全局配置 revision。重试同一序号与同一内容保留原接收时间，不能借重试刷新旧观测。旧 epoch、旧序号、错误 revision / digest 和越权 peer / listener 声明被拒绝。

运行实例先读取当前 epoch，再携带固定的 `expected_epoch` 条件申请新实例。丢失响应后的重试保留原始期望值；过期的开始请求不能覆盖更新的实例。与配置有效期不同，epoch 用于隔离实例和报告顺序，不单独决定本地 socket 的存活时间。

## 应用与撤回

Agent 使用既有绑定私钥与完整 64 字符节点 ID。listener 的 peer 许可按入口限定：获准连接入口 A，不代表可以连接同协议的入口 B。VLESS 入口先完成校验证书的 TLS 与协议握手，再进行 Meshway Noise 身份认证。

应用前重新确认本地密钥和配置缓存的持久性。配置、使用中的本地 profile 或解析目标实质变化时，先撤回旧运行层，再安装新配置；只有全局 revision 变化而实际连接意图未变时，保留现有邻接。

只有成功获取并保存新的控制配置才续期本地配置有效期。控制面不可达、报告请求成功或重试退避不能无限延长它。有效期届满、权限被撤销、持久性错误及退出进程都会关闭 listener 和通道；启动时也不直接从旧缓存恢复网络权限。

缺失或无效 profile、DNS 失败、bind 失败与邻接认证 / 探测失败分别报告，诊断不包含密码、UUID、私钥或 profile 内容。

## 不必手填每一段 IP

需要区分三个地址概念：

1. **本机监听地址**：例如 `0.0.0.0:41001`，表示监听本机接口，不是其他节点应拨号的目的地。
2. **公告入口**：例如 `lax.mesh.test:41001`，可以是 DNS 名称或 IP。它描述可能的到达方式，不自行证明可达。
3. **实际 socket 地址**：DNS、NAT 或代理参与后，本次连接使用的物理地址。

拨号方可以选择目标节点和已公告的入口，由界面填入已有地址；Agent 在底层网络中解析 DNS，并用节点公钥认证实际对端。显式地址覆盖仍保留。注册节点的展示名不自动等于 DNS 名称，分配的 mesh IP 也不能用作尚未建立隧道的 bootstrap 地址。

Docker 验收使用 `lax.mesh.test` 作为真实的容器 DNS alias，因此配置无需写死 Docker 分配的 IP。这证明域名入口与反向连接，不代表已经实现自动 endpoint 发现或公网 NAT 穿透。

## TUN 与 NAT 穿透是不同层

```text
应用 IP 流量 → TUN / userspace stack → 分类与授权 → 逻辑路径
                                                     ↓
节点入口 / endpoint 发现 → NAT 连通性 → 已认证邻接 → 具体承载
```

TUN 将操作系统的 IP 包交给用户态程序，不会把 VLESS / Trojan 的 TCP/TLS 连接自动变成可打洞的 UDP 协议。当前可通过 NAT 后节点主动连接可达 listener，并复用双向通道来接收流量：

```text
建立连接：NYC → LAX ← SHA
认证邻接：NYC ↔ LAX ↔ SHA
```

两侧均不可直接接入时，UDP endpoint 探测、STUN/ICE 式穿透或显式允许的连通性中继属于后续连接管理能力。它们与业务的必经节点约束分开，不得因连接失败绕过指定节点或降低 E2E 语义。

## 运行和验收入口

CLI 与私有 profile 格式的精确契约保存在 `apps/agent/MANAGED_CONNECTIONS.md`。基本命令：

```sh
cargo run -p meshway-agent --features managed-connections -- run \
  --state .meshway/agent.json --apply-connections \
  --connection-profiles .meshway/profiles.json \
  --interval-seconds 5 --connection-lease-seconds 45
```

`--apply-connections` 包含连接配置同步；原有 `run --connections` 仍只负责保存。Native UDP / TCP 不需要 profile 文件。VLESS profile 文件私有保存 UUID，服务端引用证书与私钥，客户端引用信任的 CA 与 server name；managed VLESS 不提供跳过证书验证的选项。

隔离 Docker 验收入口为 `experiments/managed-connections/run.py`，它使用生成 SDK 配置真实注册节点，验证双向探测、SHA 发起 TCP 连接、配置撤回、profile 缺失、控制面停机后的有效期截止、恢复及撤销。结果以脚本生成的非秘密 `result.json` 为准，不以进程启动或配置保存充当连通证明。

## 已完成的 Docker 验收

原生 ARM64 Docker 镜像 `sha256:82cec63170602b8b75dad7a0a0a4815286e48d9614971be85a78f92ba03af074` 完成全链路验收，运行 ID 为 `muoz8o7j-613526f5`。每个容器限制 1 CPU / 256 MiB，控制配置轮询间隔 1 秒，测试中的本地配置有效期为 10 秒。

- 三个 Agent 真实注册，使用完整 64 字符节点 ID、各自私有密钥及 controller 分发的节点配置。
- NYC 经原生 UDP 连接 LAX；SHA 使用校验 CA / server name 的 VLESS/TLS 主动连接 LAX，两端均完成新鲜加密邻接 nonce 探测。
- 公告及拨号使用 `lax.mesh.test` DNS alias，而非写死容器 IP。
- 删除 SHA 本地 profile 后，报告应用错误并关闭通道；恢复文件后重新认证。
- 删除拨号候选后撤回相应邻接，恢复配置后重新建立。
- 停止 controller，配置有效期届满后全部 Agent-owned UDP socket、listener 和 VLESS 连接关闭，Agent 进程仍运行。
- controller 使用原数据库重启后，以相同 epoch、revision / digest 恢复邻接，报告序号继续增加。
- 撤销 SHA 后，节点 API 返回 403，进程及通道停止，旧认证状态不再显示为当前观测。

测试使用两个隔离数据 bridge；仅 LAX 额外连接用于主机 API 端口发布的管理 bridge，NYC / SHA 不加入它。内核转发关闭，跨端点 canary 无法直连。临时测试容器、网络及证书均已清理，完整非秘密结果保存在该运行的 `result.json`。

本轮还修复了两种恢复边界：成功同步配置后必须清除控制请求的旧退避，避免 30 秒退避令 10 秒配置有效期反复届满；DNS 与显式 IP 候选解析到相同 socket 时按 peer / carrier / 地址去重，保留优先候选，不因地址收敛撤回整份有效配置。

## 可查看的本地演示

本地 `demo-nyc`、`demo-lax`、`demo-sha` 使用 `.meshway/docker-demo` 内独立持久身份，连接开发 controller。控制台 `/nodes` 可进入节点详情与连接页，区分控制面联系、保存回执、应用阶段、主动建连方和真实探测 RTT。

```sh
python3 experiments/managed-connections/demo.py status \
  --data-dir .meshway/docker-demo --admin-token-file .meshway/controller/admin.token
python3 experiments/managed-connections/demo.py stop --data-dir .meshway/docker-demo
```

`stop` 保留节点身份和配置意图，供原身份恢复。演示的共享管理 bridge 用于连接主机 controller，因此它不替代隔离 no-bypass 验收。TLS 证书有效期为 30 天，尚不自动轮换，三地名称仅为本机容器标签。

持久 demo 已另行启用 [托管 Mesh IPv4 数据面](/managed-dataplane)：`demo-nyc` 为 `100.96.0.2`、`demo-lax` 为 `100.96.0.3`、`demo-sha` 为 `100.96.0.4`。六个方向都有显式授权；NYC 与 SHA 的正反路径经过 LAX，其余节点对使用直接逻辑路径。六个方向的普通 ICMP 各三次均成功。该能力来自路由、授权与 TUN 的共同配置，不是邻接自动授予的权限；本机 `nyc` 仍只保存配置。
