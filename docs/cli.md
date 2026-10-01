# Agent CLI：保存状态与运行证据

CLI 与常驻 Agent 分工：`run` 持有身份锁并应用配置；只读诊断通过现有私有 Unix socket 查询该 daemon。命令不会启动第二个实例，也不会把缓存当作运行证明。

## 常用入口

```sh
# 保持兼容：读取本地已保存身份 / 配置 JSON
meshway-agent status --state /state/agent.json

# 查询运行中的 managed daemon
meshway-agent status --live --state /state/agent.json
meshway-agent status --live --json --peers --state /state/agent.json

# 解析当前可信 inventory 中的节点身份
meshway-agent whois demo-sha --json --state /state/agent.json
meshway-agent whois 100.96.0.4 --state /state/agent.json

# 本次真实的直接邻接 nonce 探测
meshway-agent ping demo-lax --count 3 --json --state /state/agent.json

# 查询同一 daemon 的 DNS 状态，再发起真实的本地 DNS 查询
meshway-agent dns status --json --state /state/agent.json
meshway-agent dns query demo-sha.meshway.test --type A --json --state /state/agent.json
meshway-agent dns query demo-sha.meshway.test --type AAAA --tcp --json --state /state/agent.json
meshway-agent dns query 100.96.0.4 --json --state /state/agent.json
```

可用 `MESHWAY_STATE` 设置默认路径，显式 `--state` 优先。Docker 开发镜像的 `meshway` 是相应启动器，可直接使用相同子命令。

## 输出如何理解

- 不带 `--live` 的 `status` 仍是保存状态；存储 revision 和 ACK 不证明 daemon 正在运行。
- Live status 返回同一次公开代际的安装版本、接受的配置来源版本、剩余 lease 和认证通道 registry。`installed` 不等于 TUN 已安装、邻接刚完成探测或业务可达；DNS 使用独立的 `dns status` 观测。
- `--peers` 支持明确分页，每页最多 64 项；跨页检查 generation，不能把多代结果拼成一次观测。
- `whois` 只查询身份，不是 DNS、探测或授权判定。名称有歧义时使用完整 ID 或 Mesh IP。
- `ping` 仍是邻接探测，不是 ICMP 或多跳 trace。JSON 带样本、整体结果和遇失败停止的说明，不混入普通日志。
- 无 daemon、lease 过期或配置撤回会明确失败 / unknown，不回退到 cached-live。JSON 与人类输出均不输出 bearer、私钥、实例秘密或 channel token。

当前退出码为成功 `0`、运行失败 `1`、参数错误 `2`。私有 IPC 请求最多 4 KiB、响应最多 64 KiB，同 UID 及文件权限检查保持不变；历史 ping 请求保留原响应形状，新的 enriched ping 使用版本化操作。

## DNS 状态与实际查询

`dns status` 从运行中的 DNS Guard 读取同一次快照：`active` / `disabled` / `expired` / `unavailable`、代际、revision floor、当前活动版本、zone / TTL、剩余 lease、listener 和已知网络 / 节点 ID。关闭状态也受 revision 和 lease 约束；已知新版本撤回旧状态，过期后显示 `expired`。旧 zone / TTL 可作为最后已知设置显示，不证明记录仍活动。

`dns query` 先读取 listener，再通过 UDP 或显式 `--tcp` 发一次真实事务。名称默认查 A，IP 默认转换为 PTR；名称参数不使用系统搜索域。metadata 是查询前的独立状态观测，wire result 另有证据来源，不把二者说成同一次原子快照。

- positive、NXDOMAIN 和 NOERROR/NODATA 是有效查询结果，退出 `0`。
- REFUSED / SERVFAIL 退出 `1`，JSON 的 `observation` 保留实际失败答复。
- 超时、缺少 daemon、非法响应或不兼容 IPC 明确失败；不从缓存或 inventory 伪造答复。
- DNS-only 与 managed 模式共用同一个私有 socket；DNS-only 的邻接 status / whois / ping 不可用。nameservice 状态不等于系统 resolver 已安装；容器实验 adapter 自有 status / down，见 [DNS](/dns#容器内系统解析接入)。

## 底层网络 netcheck

`meshway-agent netcheck --stun-server <数字IP:端口> --stun-server <另一数字IP:端口> --bind 0.0.0.0:0 --json` 使用当前进程的独立临时 UDP socket；不需要 `--state` 或 daemon。报告当前观察到的映射、逐次失败原因以及可空的目的相关变化，详见 [Netcheck](/netcheck)。这与通过已认证邻接进行的 `ping` 是两类证据。

精确字段和兼容行为见仓库 `apps/agent/CLI.md` 与 `apps/agent/NETCHECK.md`。服务安装、up/set/down/login/logout、真实承载上的发现与多跳 trace 按[后续里程碑](/milestones)继续推进。
