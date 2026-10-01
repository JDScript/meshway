# 公钥绑定与连接配置

连接配置将“节点身份”“谁监听”“谁主动建连”和“保存回执”分别表达。业务路由仍由独立策略决定；配置一个邻接不自动授予转发、出口或业务访问权限。

## 交付边界

这一切片交付控制意图链路：Agent 私有生成与保存 X25519 密钥，向 controller 绑定公钥及声明编译能力；管理员维护监听器和拨号候选；Agent 拉取本节点配置、校验保存并回执。

这些阶段分别意味着：

| 阶段 | 已证明的内容 |
| --- | --- |
| 公钥已绑定 | 该节点 bearer 向 controller 建立了不可任意替换的公钥关联 |
| 能力已声明 | Agent 声明该构建实现了指定承载与建连角色 |
| 配置已保存 | controller 已校验并保存管理员的连接意图 |
| Agent 已确认保存 | Agent 已校验配置、私有落盘并提交匹配摘要 |

公钥绑定不是实际邻接握手证明，能力声明不是已开启的 listener，保存回执不是配置应用或连接成功。现有传输实验与这里的控制身份、配置分发继续保持明确边界。

已另行接通[托管连接运行层](/connection-runtime)：使用 `--apply-connections` 明确应用配置，并以独立 API 上报实际邻接探测。`run --connections` 和本页的保存 ACK 语义保持不变。

## 公钥与持久身份

数据面公钥通过节点凭据派生的身份绑定，请求不能指定另一个节点 ID。首次绑定后，同一公钥可以幂等发布；不同公钥返回冲突，首版不提供隐式轮换。公钥在全部节点间唯一，撤销节点保留绑定，不能将其公钥重新分配给新节点。

输入为规范 X25519 公钥：64 个小写十六进制字符，对应规范 u-coordinate 编码；拒绝低阶点及同一有效公钥的非规范别名。私钥只存在 Agent 的私有 sidecar 中，不进入 controller、Web 或普通日志。原有 Agent 注册身份及注册令牌绑定不因发布公钥而改变。

能力由 Agent 按实际编译功能生成，包含协议、`can_dial`、`can_accept` 和 stream / datagram 语义。当前管理契约的承载为 `native_udp`、`native_tcp` 与 `vless`；它们的实际自动应用仍需后续接通。这里的原生 UDP 是 Noise 实验承载，不称作 WireGuard。

## 监听器与拨号候选

### Listener

监听器包含稳定 ID、所属节点、承载、bind 地址、advertised endpoints 和可空的本地凭据引用。

- bind 可使用 `0.0.0.0` 等未指定接口地址，但端口必须非零。
- advertised endpoint 是独立的 host / port，不能把 wildcard bind 当成可达入口；局域网与 loopback 可用于开发。
- hostname 规范化为小写 IDNA，不保留末尾点；发布地址不证明真实可达性。
- VLESS 使用本地 profile 引用；原生 UDP / TCP 的该字段必须为 null。

### Dial candidate

拨号候选包含稳定 ID、主动发起节点、目标 listener、选定的 advertised endpoint、优先级和本机 profile 引用。接受方身份由 listener 所属节点确定。候选不能自连，其 endpoint 必须匹配该 listener 的公告地址。

优先级较小者优先，这是主备意图，不表示按流分担或聚合。VLESS 两侧可以使用不同的本地引用名；真实 UUID、TLS 私钥与密码不通过配置 DTO 传播。本切片不解析这些 profile 或配置外部代理。

```text
业务路径：NYC ──→ LAX ──→ SHA
建连方向：NYC ──→ LAX ←── SHA
```

SHA 的候选引用 LAX 的 listener，明确由 SHA 发起连接。未来运行层可复用该已认证双向邻接向 SHA 发送业务，但业务权限仍需独立授权。

## 全局版本与两种回执

公钥、能力及连接计划的实际变化使用现有全局 revision，与节点清单、国家和路由策略共享 CAS 基准。连接保存要求 `expected_revision`，过期返回 `409`；语义上无变化的成功保存不增加版本。

现有 IP / 路由配置及其 ACK 保持原有契约。连接配置使用独立的节点快照、缓存和 ACK：

- IP / 路由 ACK 不代替连接 ACK。
- 连接 ACK 必须匹配当前节点、当前全局 revision 和本节点快照摘要。
- 两种 ACK 都只表示对应内容已保存，不增加 revision，也不代表实际网络生效。
- 摘要使用固定对象字段顺序的规范 JSON 与 SHA-256；联系时间、回执时间等观测信息不进入 hashed payload。

管理员响应包含源计划、诊断和节点公开身份 / 回执。节点响应只投影自己的 listener、自己的 outgoing 候选、允许连入自己的候选，以及对应直接邻居的公钥。它不会得到其他节点的本地 profile 引用。

## Agent 与控制台操作

控制台入口是 `/connections`。在已有注册身份上发布公钥与当前构建能力：

```sh
cargo run -p meshway-agent --features transport-lab -- connections publish --state .meshway/agent.json
cargo run -p meshway-agent --features transport-lab -- connections sync --state .meshway/agent.json
cargo run -p meshway-agent -- connections status --state .meshway/agent.json
```

长期运行时使用：

```sh
cargo run -p meshway-agent --features transport-lab -- run --connections --state .meshway/agent.json --interval-seconds 15
```

同一 state 的原有 `run` 进程需以新参数重启，不能同时启动第二个实例。`run --connections` 先发布身份，再同步 IP / 路由及连接配置，分别返回保存回执。未带 `--connections` 时仍采用原有同步流程；未编入承载功能的默认构建只绑定公钥，声明的能力列表为空。

原有身份文件保持不变；新增私有文件为 `<state>.connection-key.json` 和 `<state>.connections.json`。发布前必须确认密钥已持久保存，ACK 前必须确认缓存文件及目录已同步。写入失败不会以旧文件或重新生成的密钥冒充成功，`connections status` 明确返回 `applied: false`。

## 无效源配置与恢复

保存时检查节点、公钥、能力、listener 引用、endpoint 和角色。配置因撤销或能力收回而失效时，controller 保留源计划供管理员修复，向节点发布空的有效执行意图并附带无效标记；不会删除失效候选后悄悄改走另一条路径。

Web 保留失败保存时的草稿及其原始 revision。成功保存或明确重新读取才更换编辑基准。后续运行层还需要定义配置应用、撤销传播、现有流处置及真实连接观测。

## 源契约

Rust 权威类型与路由声明生成 OpenAPI、Rust Agent 客户端、TypeScript 操作与运行时 schema。精确字段、限制及操作见仓库 `crates/protocol/CONNECTIONS.md` 与生成的 `packages/protocol/openapi.json`；不在客户端另写一套 HTTP DTO 或 endpoint。
