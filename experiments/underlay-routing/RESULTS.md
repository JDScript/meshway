# Socket underlay routing · 2026-10-01 验收

最终独立 Docker 验收 **148 checks 全部 ok**，`status=ok`、`cleanup_errors=[]`；host-only unittest **10 tests 全部通过**。本目录 [`result.json`](./result.json) 是原始最终结果的逐字节副本，包含每项证据、真实 RPDB/route 快照、命令、observer `recvfrom`/accept、源码 hash 与清理结果。

## 可追溯产物

- Run：`mw-underlay-112dee961b18`
- 时间（UTC）：`2026-10-01T23:05:23.568983+00:00` → `2026-10-01T23:05:43.161968+00:00`
- Linux：`7.0.14-orbstack-00380-ga7e0a2dc9535`、`aarch64`
- Image ID：`sha256:0390c13f8bdd110214767dd0d55e67143f4ecc069f8188b9bb8f3e524039c48c`
- result SHA256：`23b67d83218ad846f934521e3f20390fd982359fe429e05c0339c0f2f4c99f0d`
- 原始目录：`/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/meshway-underlay-artifacts/mw-underlay-112dee961b18/`
- 原始目录另保留 `client.stdout`、`client.stderr`、`observer.jsonl`。

源码 SHA256：

```text
helper.py  aaacd7487796d7c02f4ec1b24a09e80bd8eb0e6544b7adffcc49b968310e2a30
fixture.py d2d92bf5f6bc4be9db0a26cad7fc669eb64bd8b9503b5afcfea3e67b31fa4092
```

构建、验收、host-only tests 全部使用现有 `performance_lock.locked_launch` / 继承 FD 契约；无嵌套锁、host network、privileged、宿主配置改变或 live demo 接触。

## 验证范围

IPv4 与 IPv6 均通过：

| 场景 | 实际证据 |
| --- | --- |
| 具体前缀、默认捕获、profile B 表 | protected HTTP、STUN UDP、验证证书的 TLS、bootstrap DNS UDP/TCP、nonce echo 成功；每种同进程/host/port 的普通 socket 失败且包被真实 TUN 捕获 |
| 不循环 | protected 请求实际源端口没有出现在 TUN；无全目标 `/32` bypass |
| 多出口模拟 | STUN 分别实际观测模拟 E `.13/::13`、F `.14/::14`；profile E→F 的 protected STUN 仍为 underlay `.10/::10` |
| 原有系统 policy | 保留 low mark `0x20/0xff` 与独立 table `18550`；marked STUN 的真实源为 `.11/::11`，与 main 的 `.10/::10` 不同 |
| 资源冲突 | live table、priority 与重叠 mark owner 拒绝且原资源不变；host tests 另覆盖目标 priority、未知 selector 与配置边界 |
| 缺底层路径 | marked STUN 被系统拒绝，不输出 mapped candidate，且没有 unmarked TUN fallback |
| 缺权限 | 降为 UID/GID 65534 的子进程真实收到 SO_MARK permission denied，无候选/无 fallback |
| 换网代际 | prepare 新 guard → 旧观测失效/socket close → 新 socket 激活；新 STUN 观测 `.11/::11`，旧 generation 不再 fresh |
| 未发送/已发送边界 | 旧 FD 新 send 实际报错；observer 仍实际收到退休前 transaction `6f6c642d73656e742d313233`，close 不召回已发送字节 |
| TUN 退出 | dev routes 消失，unreachable 留存；普通业务失败、protected controller 仍成功 |
| 精确清理 | 原有系统 rule/table 经 profile/TUN-loss 保持不变；最终 RPDB **及全部原有 route 快照**恢复，容器/网络按 owner 精确删除 |

E/F 是同隔离 namespace 的不同源地址对照，业务只进入 sink；不是完整出口转发或公网 NAT。nonce echo 不是 authenticated probe；plain TCP/TLS 不是 VLESS+WS。IPv6 仅证明 underlay，未验 IPv6 overlay。

## 关键内核发现

### `not` 会反转整个 RPDB selector

真实 IPv4 反例：

```text
not fwmark 0x4d570000/0xffff0000 iif lo lookup 18401
route get 172.30.247.20 from 172.30.247.21 iif eth0
→ dev mwua table 18401  # 非 lo 转发也被捕获
```

IPv6 得到同样反例。修正为：

```text
18000: fwmark 0x4d570000/0xffff0000 iif lo goto 18003
18001: iif lo lookup 18401
18002: iif lo lookup 18402  # 仅在切换时存在
18003: nop
```

相同 forwarding lookup 变为 `dev eth0`；protected socket 只跳过自有连续 block，保留既有 RPDB。

### goto 需要真实的 exact-priority 目标

开发中只填不存在的 `goto 18003` 导致 unresolved、protected HTTP fall-through 到捕获并 timeout。最终先安装自有 NOP，核对 goto resolved 后才安装 capture；清理相反排序。结果内分别记录 v4/v6 实际 rule JSON，不把“下一 priority 数字”当作有效目标。

### IPv6 快照须等 DAD 稳定

开发中 secondary IPv6 地址仍 tentative 时 bind 失败；尚未完成 link-local DAD 就保存快照会把正常新增的 kernel route 错报为清理失败。最终 fixture secondary 使用 `nodad`、主地址/link-local 等待真实 DAD 结束后保存完整快照，最终恢复检查通过。

这些失败 run 的真实 JSON 仍保留在批准 TMP，均没有被当作通过结果：`cc07bfe368f7`（tentative bind）、`ed21ac848cf5`（IPv6 快照 race）、`d8f79f92ed5b`（无 NOP 的 unresolved goto）。各 run `cleanup_errors` 如原始 JSON 所示；最终结果只对应上述最终 run。

## 生产接入仍待完成

推荐接口、socket 创建点、reqwest 0.13.5 的 pre-connect mark 缺口、bootstrap DNS、UDP listener/outgoing、TCP listener 继承、VLESS/WS、cap、ownership/恢复与 R3 交界见 [`docs/underlay-routing.md`](../../docs/underlay-routing.md)。本分支没有把机制实验接到生产 Agent，也没有修改 P1c 原型、root manifest/lockfile 或 CI。
