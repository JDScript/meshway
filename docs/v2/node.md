# Node：节点端

Node 由长期运行的 **`meshwayd`** 和本机 CLI **`meshway`** 组成，Linux 首版采用单机系统服务。Server 决定许可和有界候选；Node 在获准集合内结合实时状态运行。连接方向不等于业务方向，承载切换不等于逻辑改路，回复沿原会话完整路径返回。

## 组成

下图中的 plugin host 与 netstack 为规划项，尚未实现；实际 crate 清单、职责与跨入口复用见[总体结构](/v2/architecture#node-组件库)。

```text
meshway CLI ── 本机 IPC ──► meshwayd
                            ├─ plugin host（规划） / 私有持久化 / 生命周期
                            ├─ control          接入与 Server 同步
                            ├─ dataplane        准入、加密、路径与包流
                            ├─ connectivity     邻接、主备与打洞
                            ├─ underlay         底层 socket／系统资源保护
                            ├─ transport
                            │   ├─ udp / tcp    标准发行必带
                            │   └─ vless / trojan / ...
                            ├─ tun / netstack（规划）  包接入
                            ├─ dns              业务解析
                            └─ diagnostics      状态与获准测量
```

## 当前范围

已实现（Linux）：

- Server 持钥接入、自动认证、实例／lease 续租、配置保存与安装结果上报，见[网络模式与本机运行](/v2/node/runtime)。
- 默认 Mesh-only 系统 TUN，可显式选择 `control-only`；失败不回退。
- 用途受限 underlay、认证 stream／Native UDP 邻接、协调 UDP 穿透与受保护 L3 IP 包流，见[数据面](/v2/node/dataplane)与[连接获取](/v2/node/connectivity)。
- 系统 DNS（resolved 或显式 resolv.conf 代理）与 `meshway ping`，见[路径诊断](/v2/node/diagnostics)。

未实现：Exit／NAT、出口 resolver、userspace networking（netstack／SOCKS5）、动态插件、切网、非 Linux 平台的 TUN。安装门槛见[安装范围与状态](/v2/node/runtime#安装范围与状态)，验收状态见[验收范围](/v2/node/validation)。

## 规划方向

- CLI 提供 up／down／ping／status 与网络切换；业务配置在云端管理。
- 业务数据面统一采用 L3；userspace 首版通过本机 SOCKS5 接入应用流量，见[网络模式](/v2/node/runtime)。
- 首版动态插件边界是 transport providers，启动时加载，随发行包一起安装、校验、升级；其余组件作为 Rust 库由 daemon 链接，见[插件与能力（规划）](/v2/node/plugins)。
- UDP／TCP 实现必带不授予业务使用权限，VLESS-only 配置继续有效。
- 本机操作员授权、native ABI 精确接口、MTU／分片与 TCP 性能优化尚待定版。

## 相关页面

- [Server 接入组件](/v2/node/control) · [网络模式与本机运行](/v2/node/runtime) · [Linux 安装与系统服务](/v2/install) · [路径诊断与 ping](/v2/node/diagnostics)
- [数据面、TUN 与 DNS](/v2/node/dataplane) · [承载与邻接模型](/v2/node/carriers) · [承载会话](/v2/node/transport) · [连接获取与 UDP 穿透](/v2/node/connectivity)
- [三节点 Docker 环境](/v2/node/docker-lab) · [验收范围](/v2/node/validation) · [分层性能测量](/v2/node/performance)
