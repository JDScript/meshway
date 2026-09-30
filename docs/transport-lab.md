# Linux 传输实验

本切片用可运行的独立实验验证“业务方向与建连方向分离”、逐跳认证、端到端密文转发和协议适配边界。使用 **`transport-lab` feature** 显式启用，不改变现有 Agent 的注册、心跳、配置同步或 Web 的状态语义。

## 组成与边界

| 部分 | 实验实现 |
| --- | --- |
| `crates/dataplane` | Snow Noise IK 身份认证、独立 E2E 会话、原生 UDP / TCP、受限帧与批量接口、方向性路径授权 |
| `crates/transport-vless` | VLESS v0 TCP CONNECT、固定目标接收端、外部代理延迟响应适配；UUID 不替代 Mesh 身份 |
| `apps/agent/src/transport_lab` | 私有配置、命令行、VLESS 生命周期、真实 endpoint echo、Linux 固定 IPv4 对 TUN |
| `experiments/transport` | 隔离 Docker 拓扑、裸 iperf3 基线及实际传输比较工具 |

原生 UDP 使用本实验的 Noise 封装，**不是 WireGuard 实现或兼容协议**。VLESS 适配器当前提供无 addons 的 TCP CONNECT，不宣称支持 TLS / Reality / Vision / XUDP / mux / Trojan。实验承载中的业务与 Meshway 握手由 Noise 保护；VLESS 接入 UUID 的保密性需要外层 TLS 等独立机制，原始 VLESS 实验仅用于隔离环境。

## 三节点与反向建连

```text
业务：NYC ── UDP ──→ LAX ── VLESS stream ──→ SHA
建连：NYC ─────────→ LAX ←────────────────── SHA
```

SHA 不开放 VLESS listener；它主动连接 LAX。LAX 完成 SHA 的 Noise 密钥认证后，通过该已接受通道把 NYC 发来的 E2E 密文转给 SHA。

每个节点仅持有自己的私钥；受信任公钥列表还需明确每个相邻节点可以使用的承载。空 `transports` 表示只信任该身份用于 E2E，不允许其直接建立邻接。比如 NYC 信任 SHA 的终点身份，但二者没有直接邻接权限；LAX 对 SHA 只允许 VLESS 时，SHA 不能用 UDP 替换该通道。

路径授权采用精确有序列表。允许 `nyc,lax,sha` 会允许同一次请求沿原路返回，不自动允许 SHA 发起新的业务。生成的双向实验显式包含 `sha,lax,nyc`，供反向 probe 和 TUN 回程使用。

## 本机进程验证

```sh
cargo build -p meshway-agent --features transport-lab
LAB_DIR="$(mktemp -d)"
target/debug/meshway-agent transport-lab fixture --directory "$LAB_DIR"
target/debug/meshway-agent transport-lab check --config "$LAB_DIR/nyc.json"
```

在两个终端分别启动 LAX 和 SHA（使用同一 `LAB_DIR` 值）：

```sh
target/debug/meshway-agent transport-lab run \
  --config "$LAB_DIR/lax.json" --status-file "$LAB_DIR/lax-status.json"
target/debug/meshway-agent transport-lab run \
  --config "$LAB_DIR/sha.json" --status-file "$LAB_DIR/sha-status.json"
```

另一个终端运行 NYC：

```sh
target/debug/meshway-agent transport-lab probe \
  --config "$LAB_DIR/nyc.json" --path nyc,lax,sha --bytes 1280 --count 3
```

默认接收端口是 LAX 的 UDP `41000` 和 VLESS TCP `41001`。生成器可通过 `--lax-nyc`、`--lax-sha` 指定实际 endpoint；`--second-leg udp` 将第二段也改为原生 UDP，SHA 仍是连接发起方。

配置目录与文件分别要求私有权限 `0700` / `0600`，写入使用已有的原子私有存储原语；生成器拒绝覆盖现有配置。运行中的节点持有配置侧锁，同一身份不能同时被另一个本地实验进程使用。勿输出或提交配置中的私钥与代理凭据。

### Probe 的含义

成功输出代表：真实数据面上完成 endpoint 认证，并将指定字节沿获授权路径往返。测量包含本次 E2E 会话建立的成本。

结果中的 `authorized_path` 是经过校验的路径上下文，不是逐跳测量证据；当前命令不叫 `meshway trace`，也不把 echo 速率称为 iperf3 吞吐。

## Linux TUN

`fixture --tun` 给 NYC / SHA 添加实验 TUN 配置，LAX 只做密文中继。固定 IP 为 `198.18.0.1` 与 `198.18.0.2`，内层 MTU 为 1280。端点在注入前校验 E2E 来源、精确路径、IPv4 头、源 / 目标地址、长度与分片标志；不允许这个实验接口转发到任意地址。

当前 TUN 只覆盖固定 IPv4 对上的 TCP / UDP / ICMP，不接受 IPv4 options、分片及其他 IP 协议；不是生产 IPAM、任意子网、NAT、双栈接口或 userspace TCP/IP stack。IPv6 字节可以作为核心的 opaque payload，但不构成 Linux TUN IPv6 支持的验收。

在三节点短 ID 路径上，1280 字节内层包产生 1393 字节 UDP payload，外层 IPv4 共 1421 字节。路径更长时封装开销也会变大；配置检查使用实际路径计算 1500 外层 MTU 下的预算。当前没有 Meshway 分片重组或 PMTU 发现。

Docker 镜像构建：

```sh
docker build -f apps/agent/Dockerfile.lab -t meshway-agent-lab:local .
```

容器按需提供 `/dev/net/tun` 与 `NET_ADMIN`。测试必须把各节点私有配置分别隔离，中继容器不能挂载端点私钥。根 `.dockerignore` 排除 `.meshway`、现有凭据、参考 checkout 与构建缓存。

## 性能与故障验收

裸网络基线：

```sh
python3 experiments/transport/benchmark.py --output "$LAB_DIR/baseline.json"
```

工具比较两容器 bridge 与 NYC → LAX → SHA 内核路由，记录真实 iperf3 数据、原生 CPU 架构、镜像、内核及资源限制。相同 VM 中的虚拟网络结果不代表公网、多区域或物理网卡性能；ARM64 主机上的 amd64 模拟结果不充当 x86 性能结论。

实际 Meshway 测试使用相同三节点网络，验证双向 IP 流量及 ICMP；停止 LAX Agent 后，即使底层接口仍在，业务也必须失败。具体命令与输出字段见仓库 `experiments/transport/README.md`。

批量接口以有限数量的 owned frame 传递数据；当前仍有复制，UDP 收发尚未使用 `sendmmsg` / `recvmmsg`，TUN offload 未开启。本轮测量用于定位实现成本，不能作为最终优化上限。

### 2026-09-30 最终复验

环境为 OrbStack 原生 Linux ARM64，同一镜像、iperf3 3.18，每节点 1 CPU 配额 / 256 MiB，每项 3 秒、一次采样。镜像 ID：

```text
sha256:d9bce9aad7687f81733fb2d64a68b8671bcc8751bda72da8007f6cdb99e1d823
```

下表为接收端吞吐，单位 Mbit/s：

| 路径 | TCP 单流正向 | TCP 单流反向 | TCP 四流正向 | TCP 四流反向 |
| --- | ---: | ---: | ---: | ---: |
| 裸内核三节点路由，链路 MTU 1280 | 72,898.08 | 74,888.74 | 67,428.15 | 70,594.30 |
| Meshway，两段原生 UDP | 329.14 | 336.32 | 340.75 | 348.69 |
| Meshway，UDP + 反向建连 VLESS | 314.67 | 317.96 | 324.11 | 325.68 |

Meshway 使用 1280 内层 / 1500 外层 MTU；裸基线为 1280 链路 MTU。相同镜像与载荷预算有助比较，但上述差值不是独立的“加密成本”，尤其裸基线在同一 VM 内利用内核及虚拟网络批处理，未经过物理网卡。

6 项裸基线与 12 项 Meshway iperf3 均完成。四项 Meshway UDP 测试在 20 Mbit/s、1200 字节载荷下接收约 19.98–19.99 Mbit/s，报告 0% 丢包。两种承载均通过双向 ICMP，以及仅停止 LAX Agent 后双向业务失败、底层接口与健康服务仍可用的故障检查；实验资源已清理。

TCP 单流正向的 cgroup 采样显示各节点平均约 31.6–40.9% 单核 CPU，未发生 CPU quota 节流；测量窗口包含辅助进程。TCP 压测出现重传，Agent 聚合丢弃计数也非零，不能把 UDP 定速测试的零丢包推广到所有压力场景。后续应先细分丢弃原因、分析队列与调度，再用 profiling 指导批量 syscall、buffer pool 和 offload 优化。

本地完整原始结果保存在临时目录的 `meshway-bare-final-20260930.json` 与 `meshway-tun-final-20260930.json`，运行 ID 分别是 `meshway-lab-20260930234154-e74bcd1c` 与 `meshway-lab-20260930234231-e86ddcbd`。这些私有 JSON 不进入 Git；可用上述命令重新生成证据。CI 的手动 `Linux transport laboratory` workflow 上传其自身运行的非秘密结果。

### 认证与缓存复核

独立审查后的修复包含：

- 邻接必须完成本次新会话的加密密钥确认与 ACK 后才发布，捕获的旧 Noise IK 首报文不能替换已建立通道；旧确认在重启后也无效。
- E2E packet context 允许一个活动密钥与一个待确认替换密钥；仅新密钥下的有效 AEAD 数据能完成替换。配置上限为 L 时，接收状态有界为 2L packet keys 加 L 独立 echo setups。
- 待确认 packet key 保留 60 秒；发送端缓存截止时间从 Init 发起时计 59 秒。第一包丢失后仍能继续发送，延迟 Ready 不延长缓存预算，过期 Ready 不进入发送缓存。

丢失 Ready 或来自已授权邻居的重放可以占用一个 pending slot 最多 60 秒，但不能在缺少新密钥证明时提升或驱逐活动密钥。对应首报文重放、容量一的换钥、第一包丢失、延迟 / 丢失 Ready 回归位于 `crates/dataplane/src/runtime/tests/security.rs`。

## 后续接入条件

- 静态实验密钥与 allowlist 尚未接入 controller：需另行完成生产公钥绑定、轮换、撤销、租约与动态授权。
- 原生 UDP 握手和数据为 best effort；缓存的 E2E packet session 约 60 秒过期，终点重启后可能在过期前出现黑洞。
- 原生 UDP 邻接有经过认证的心跳与过期；stream 的静默黑洞检测目前依赖操作系统，后续需业务探测与防抖。
- 原生承载有同邻接优先级选择；实验 CLI 的 VLESS 每个 peer 仅维护一个候选，并拒绝与 native outgoing 混配，避免声称完整 multi-transport 回切已交付。
- 多队列、buffer pool、平台批量 syscall 与 TUN GSO/GRO 的优化应由本轮 profiling 推进。
- 接入控制台后，配置保存、ACK、数据面应用和实际通道状态继续分开显示。
