# 多传输与故障切换

**已确认：每条逻辑邻接允许多个传输候选，首版采用优先级主备。** 平台提供默认优先级、生成下发参数与凭据，Web 支持允许协议 / 优先级 / 逐段覆盖，并保留可选 listener。按流负载分担、同流多路径及逐包聚合不属于首版要求。

**当前实现**：Native UDP / TCP、TLS-VLESS TCP 与 TLS-VLESS WebSocket 已接[托管运行层](/connection-runtime)。控制面管理 listener 与私有材料，Agent自动校验、保存并执行；旧 profiles 保留受控兼容与私有导入。精确字段、API与迁移见[托管承载对接](/managed-transports-api)。完整主备回切与按需自动建连仍未由候选优先级字段完成。

## 当前库级适配器

| Crate | 已验证能力 | 运行层接入状态 |
| --- | --- | --- |
| `meshway-transport-vless` | VLESS v0 TCP CONNECT、双向字节流、Xray TCP互通、基于tungstenite的有界WS字节流 | TLS TCP / WS已接托管执行与独立验收；未声称Xray WS外部互通已验收 |
| `meshway-transport-trojan` | Trojan TCP、校验 CA / server name 的 TLS、Xray 双向互通 | 库级完成，待 Agent 统一装配 |
| `meshway-transport-socks5` | TCP CONNECT、显式无认证 / 用户名密码、Xray 双角色互通 | 库级完成，待 Agent 统一装配 |
| `meshway-transport-shadowsocks` | 经典 AEAD TCP 三种 cipher、基于 shadowsocks-rust、sing-box 双角色互通 | 库级完成，待 Agent 统一装配 |

后三种尚未加入管理 API 的能力目录，控制台不会把它们显示成已部署的节点能力。各库的精确 API、版本和互通命令保存在各自 README。代理认证仍不替代 Meshway 身份认证；固定目标接收端也不是通用开放代理。

统一编译期 registry、控制面目录/配置校验及 Agent 版本/能力协商的当前差距与整理建议，
见[承载扩展架构审查](/transport-registry)。

## 相邻节点不变，承载可以变化

连接发起方独立于业务方向；SHA 主动 dial LAX 后，LAX 可以通过已认证的双向通道向 SHA 转发。无 listener 的节点只主动拨出，能力声明、listener、可达 endpoint 与运行通道分别建模；通道双向不表示业务授权双向。

```text
逻辑路径：NYC ─────────────→ LAX ─────────────→ SHA
               邻接 A                邻接 B

邻接 A 的候选：
  candidate-a    priority: 10
  candidate-b    priority: 20
  candidate-c    priority: 30
```

当前契约采用数字越小、优先级越高；上述数值仅作示意。每段独立选择满足健康、能力、身份与安全要求的候选，不把未认证连接提升为有效候选。

## 建连与插件边界

连接管理器请求“到指定身份、符合承载与安全要求的邻接”，可复用 dial 或 accept 通道，而不是每次无条件拨 IP。VLESS / Trojan 的握手有方向，完成后的 stream 全双工；载荷是 Meshway 帧，普通代理转发字节不自动成为 Meshway 节点。代理 UUID / password 不替代节点公钥。

编译期 registry 是插件边界，不承诺动态 Rust ABI。共同接口保留 stream / datagram、最大消息尺寸、可靠 / 有序性、buffer 所有权、有限批量、背压、错误和生命周期。`dial` 与 `accept` 产生同一种可用邻接，插件连接成功仍须核心认证和授权。

建连验证邻接密钥，建路检查路径与转发权限，逐包使用已认证会话进行 AEAD / 重放检查、查表和有界发送。中继验证直接发送者及转发上下文，终点验证 E2E 来源、目标、路径和内层 IP；外层 source 标识本身不是证明。

完整 IP 包经 Meshway 封装后可承载于 stream；普通 VLESS TCP / UDP 出口自身没有通用 ICMP 语义。userspace TCP/IP stack 可用于流代理端点，不是密文中继必需组件。ICMP 成功与原生 traceroute 显示所有 hop 是两个目标。

## 能力不能只写成“支持某协议”

| 能力维度 | 需要明确的问题 |
| --- | --- |
| IP packet | 能否承载完整 IPv4 / IPv6 包？有效 MTU 是多少？ |
| Datagram | 是否保留消息边界、是否有最大数据报限制？ |
| Stream | 只提供有序字节流，还是具备额外的 IP 封装？ |
| 安全 | 能否承载当前要求的 E2E 密文，外层认证如何完成？ |
| 连通性 | NAT、直连、中继、IPv4 / IPv6 和网络切换如何处理？ |
| 诊断 | 探测经过哪个真实连接，能返回哪些证据？ |

WireGuard、MASQUE CONNECT-IP、CONNECT-UDP 和 VMess / VLESS 类代理的接口语义不同。流代理并不会天然成为 IP 隧道；如需适配，应单独定义封装、背压、MTU 和失败语义。参见[源码与协议依据](/references)。

## 建议的选择流程

1. 从已授权的逻辑邻接中读取候选列表。
2. 过滤掉能力不足、身份不满足、安全模式不符或处于失败冷却期的候选。
3. 在健康候选中按显式优先级选择；同优先级的稳定排序方式待定。
4. 建立连接，给新流绑定候选与选择原因。
5. 持续观测连接状态与探测结果，失败后按同一邻接的候选策略处理。

建议首版在优先级主备基础上**按流固定已选传输**，避免健康的现有连接因主备回切而被反复打断。这不是按流分散到多个候选的负载均衡。同流多路径、逐包条带化与 bonding 属于后续研究，不能从“支持多传输”推导出来。

### 健康与防抖

仅能打开 socket 不代表业务数据可通。建议结合连接握手、主动探测与数据面反馈，加入连续失败阈值、恢复阈值、冷却时间和退避，防止候选反复切换。

高优先级候选恢复后，建议先用于新流，避免强制迁移健康的现有流。阈值、探测频率和回切策略需通过原型测试决定。

## 故障时能保证什么

- 同一邻接内回退，保持逻辑路径与安全模式不变。
- 候选耗尽后报告具体失败邻接，不能绕过必经节点。
- 连接失败不能触发 E2E → trusted-routing 的隐式降级。
- **现有 stream 重连不等于透明恢复。** 没有会话迁移或应用重试协议时，原连接可能终止。
- 数据报或 IP 流切换也需要考虑丢包、乱序、地址变化与出口状态；不能默认“无感”。

## MTU 与封装

多跳和多层封装会减少有效 MTU。建议记录每条候选的有效承载上限，并使用路径中最受限的一段指导包尺寸处理。

需要验证 IPv4 分片、IPv6 Packet Too Big、PMTUD 黑洞及 ICMP 返回。只做小包 ping 或 HTTP 成功测试不足以证明传输可用。采用 stream 承载 IP 时还要评估队头阻塞，不能只比较连接建立成功率。

## 包装成本的匹配基线

性能验收比较同一协议 / 实现、加密、分帧、连接和资源条件下，直接路径与 Meshway 适配路径的差值。原生 L4 流代理与完整 L3 E2E 隧道的速度差不能直接当作包装损失。

已完成的 VLESS 匹配实验只改变是否保留已经校验握手的 `ClientStream` 适配层，正向吞吐保留率为 99.47%；反向为 105.32%，短测试中的正增益不代表固有加速。本轮没有测出这个小适配层的明显吞吐损失，认证 / E2E、有序转发和 TUN 的增量成本尚未逐层隔离。方法、CPU/byte、延迟与原始证据见仓库 `experiments/transport/MATCHED.md`。
