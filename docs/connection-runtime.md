# Agent 连接运行层

当前运行层应用已保存的 listener / 拨号候选，以实际 Noise 认证与加密 nonce 探测形成邻接观测。`--apply-connections` 不授予业务路径；另行启用[托管 IPv4 数据面](/managed-dataplane)才把授权和路径交给 TUN。

**已确认目标**是平台托管配置、按授权流量按需自动建连，并经允许连接自动中转。下方显式启动与本地 profile 是迁移前现状；自动化不能绕过必经节点或降低 E2E。

## 四个状态维度

| 维度 | 来源与证明 |
| --- | --- |
| 控制面联系 | 心跳：最近联系 controller |
| 配置已保存 | revision / digest ACK：校验并持久保存 |
| 连接配置已应用 | Agent 报告：listener 与连接任务已安装，可为空或仍等待对端 |
| 邻接已认证 | Noise 与新鲜 nonce：实际对端密钥认证和本次往返 |

运行报告是经节点凭据认证的自报告，不是 controller 独立证明。报告、epoch 和序号不增加配置 revision；精确重试不刷新接收时间。旧 epoch、序号、错误 revision / digest 或越权资源被拒绝。

运行实例携固定 `expected_epoch` 条件创建；丢响应重试保留原期望值，不能覆盖更新实例。epoch 隔离报告顺序，配置 lease 独立限制 socket 存活。

## 应用与撤回

- 使用既有持久私钥与完整节点 ID；peer 许可按 listener 入口限定。
- VLESS 先完成校验 CA / server name 的 TLS 与协议握手，再进行 Meshway Noise；没有跳过证书验证选项。
- 应用前确认本地密钥与配置持久性。配置、profile 或解析结果实质变化时先撤回旧运行层；仅 revision 变化且连接意图不变可保留邻接。
- 只有成功取得并保存控制配置才续期 lease，报告成功或退避不延长它。到期、撤销、持久性错误和退出关闭 listener / 通道，离线启动不从旧缓存恢复权限。
- profile、DNS、bind、认证和探测失败分别报告，诊断不携 UUID、密码或私钥。

## 地址与方向

监听 bind（`0.0.0.0:41001`）、公告入口（`lax.mesh.test:41001`）与实际 socket 地址是三件事。公告可以用底层 DNS 名称，Agent 用绑定公钥认证解析到的对端；Mesh IP 不用作尚未建立隧道的 bootstrap。

```text
业务：NYC → LAX → SHA
建连：NYC → LAX ← SHA
通道：NYC ↔ LAX ↔ SHA
```

NAT 后节点可主动连接可达 listener，另一方复用双向通道。TUN 不会把 TCP / VLESS 变为可 UDP 打洞的协议；两侧都不可直接接入时，认证候选分发 / 打洞 / 中转仍需独立实现。当前[STUN 观测](/endpoint-discovery)不等于 peer 已可达。

## 当前命令与验收

精确 profile 与 CLI 契约见 `apps/agent/MANAGED_CONNECTIONS.md`：

```sh
cargo run --locked -p meshway-agent --features managed-connections -- run \
  --state .meshway/agent.json --apply-connections \
  --connection-profiles .meshway/profiles.json \
  --interval-seconds 5 --connection-lease-seconds 45
```

`--apply-connections` 包含同步；`run --connections` 仅保存。Native UDP / TCP 不需 profiles；VLESS 当前私有文件引用 UUID、证书与信任 CA，待平台托管迁移。同一身份不可同时启动两个实例。

隔离入口 `experiments/managed-connections/run.py` 通过真实 controller / Agent / 生成 SDK 验证三节点注册、UDP 与反向 TLS-VLESS、DNS 公告入口、profile 缺失 / 恢复、配置撤回、控制面断联后 lease、原数据库恢复及节点撤销。数据 bridge 隔离且 kernel forwarding 关闭，结果以 runner 的非秘密 `result.json` 为准。

历史通过运行 `muoz8o7j-613526f5`（ARM64，每容器 1 CPU / 256 MiB，轮询 1 秒、lease 10 秒）验证上述生命周期，测试资源已清理。镜像与各阶段原始证据在该运行 artifact 中；重现使用目录 README 和当前 runner，不依赖曾运行的容器仍存在。

持久 demo 的身份、可写层与 binary 更新由[统一开发入口](/dev-services#已有-docker-demo)管理，生命周期恢复详见 `experiments/managed-connections/DEMO.md`。实际当前状态用 `pnpm dev:status` 检查，不把历史节点名、Mesh IP、镜像标签或旧报告当作当前现场。手动探测见[CLI](/cli#手动验证-三种不同的探测)。
