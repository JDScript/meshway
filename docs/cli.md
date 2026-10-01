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
```

可用 `MESHWAY_STATE` 设置默认路径，显式 `--state` 优先。Docker 开发镜像的 `meshway` 是相应启动器，可直接使用相同子命令。

## 输出如何理解

- 不带 `--live` 的 `status` 仍是保存状态；存储 revision 和 ACK 不证明 daemon 正在运行。
- Live status 返回同一次公开代际的安装版本、接受的配置来源版本、剩余 lease 和认证通道 registry。`installed` 不等于 TUN 已安装、邻接刚完成探测或业务可达；DNS 未由这一 IPC 接口观测。
- `--peers` 支持明确分页，每页最多 64 项；跨页检查 generation，不能把多代结果拼成一次观测。
- `whois` 只查询身份，不是 DNS、探测或授权判定。名称有歧义时使用完整 ID 或 Mesh IP。
- `ping` 仍是邻接探测，不是 ICMP 或多跳 trace。JSON 带样本、整体结果和遇失败停止的说明，不混入普通日志。
- 无 daemon、lease 过期或配置撤回会明确失败 / unknown，不回退到 cached-live。JSON 与人类输出均不输出 bearer、私钥、实例秘密或 channel token。

当前退出码为成功 `0`、运行失败 `1`、参数错误 `2`。私有 IPC 请求最多 4 KiB、响应最多 64 KiB，同 UID 及文件权限检查保持不变；历史 ping 请求保留原响应形状，新的 enriched ping 使用版本化操作。

精确字段和兼容行为见仓库 `apps/agent/CLI.md`。DNS status/query、服务安装、up/set/down/login/logout、STUN/netcheck 与多跳 trace 按[后续里程碑](/milestones)继续推进。
