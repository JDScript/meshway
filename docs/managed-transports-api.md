# 托管 listener / VLESS API

平台托管listener/材料、VLESS TCP/TLS与WebSocket/TLS已合入，既有Web节点配置与接入可选listener已接通。本页保留公开配置、私有材料、CAS/权限合同；精确类型/schema由Rust实际routes生成，不另维护TS DTO。

## 实际支持

- Native UDP、Native TCP 的平台托管 listener。
- **VLESS TCP + TLS** 和 **VLESS WebSocket + TLS**：真实顺序为
  TCP → TLS →（可选 WebSocket）→ VLESS → Meshway Noise。
- 平台生成随机 UUID、证书 / 私钥；节点无需维护 profiles / PEM。
- TLS 必须校验证书、信任根与 SNI；TLS 或 WS 失败不会改拨明文 TCP。
- VLESS 固定目标是 Meshway 内部标记，不能任意 CONNECT；UUID 不替代节点公钥、
  incoming candidate approval、Noise 或完整业务路径授权。
- 按需图编译、完整主备回切、其他代理协议和公网入口自动映射属于后续切片。
  当前拨号候选仍在高级 `connections` API 配置。

## 管理 API（当前网络 membership 鉴权）

前缀 `/api/v1/networks/{network_id}`。当前采用 `managementBearer`：
用户 session 的当前网络 membership，或既有 global bootstrap 兼容入口。
GET 允许本网 Member / Admin / Owner；两个 POST 只允许本网 Admin / Owner。
所有检查与实际保存处于同一根 IMMEDIATE 事务，合法 NetworkId 本身不是权限。
Console mutation 显式携带所选网络ID和CAS，Controller复核真实身份与权限。

| 方法 / 路径 | 生成 operation ID / SDK 导出 | 成功响应 |
| --- | --- | --- |
| GET `/managed-listeners` | `getManagedListeners` / `getManagedListeners` | `ManagedListenersSnapshot` |
| POST `/managed-listeners` | `saveManagedListener` / `saveManagedListener` | `ManagedListenersSnapshot` |
| POST `/managed-enrollment-tokens` | `createManagedEnrollmentToken` / `createManagedEnrollmentToken` | 既有 `CreateEnrollmentTokenResponse` |

SDK：`(baseUrl, userSessionOrBootstrapToken, networkId[, body])`，由生成 Zod 校验响应。
底层生成操作名为同名加 `Operation`，位于
`packages/control-client/src/generated/operations.gen.ts`。

### 节点页保存示例（无秘密）

```json
{
  "expected_revision": 42,
  "settings": {
    "id": "vless",
    "node_id": "<所选网络内的64位节点ID>",
    "enabled": true,
    "transport": "vless",
    "bind_address": "0.0.0.0",
    "bind_port": 443,
    "advertised_endpoints": [{ "host": "node.example.com", "port": 8443 }],
    "vless": {
      "stream": "websocket",
      "server_name": "meshway.local",
      "websocket_path": "/meshway",
      "websocket_host": "node.example.com"
    }
  }
}
```

响应示意：

```json
{
  "revision": 43,
  "listeners": [{
    "settings": "<规范化后的上述settings对象>",
    "material_version": 1,
    "certificate_sha256": "<证书DER的SHA256>",
    "compiled_capability_available": true
  }]
}
```

示意占位不是有效请求；真实代码使用生成的 `ManagedListenerSettings` /
`SaveManagedListenerRequest`，不得把示意字符串当 DTO。
管理员读不到 UUID、证书 PEM 或私钥。指纹与版本表示已保存材料的公有状态，
不证明证书已安装、入口可达或外部公共 CA 签发；默认是平台自签、精确信任。

### 表单字段 / 建议默认值

| 字段 | 默认 / 约束 |
| --- | --- |
| `id` | 建议 `vless`、`native-udp` 或 `native-tcp`；1–48 ASCII 字符；稳定后不通过改名替换资源 |
| `node_id` | 节点页固定当前节点；所属网络、节点和 transport 创建后不可换绑 |
| `enabled` | 建议 true；false 保留材料和意图，但撤回该 listener 的有效执行 |
| `transport` | `native_udp`、`native_tcp`、`vless` |
| `bind_address` | 建议 `0.0.0.0`；IPv4 / IPv6 字面量，与公告 host 独立 |
| `bind_port` | VLESS 建议 443，原生建议 8421；1–65535，平台不伪称端口一定空闲 |
| `advertised_endpoints` | **不猜默认公网地址**；可为空；最多16；明确 host/port，NAT 外部端口可不同 |
| `vless` | 原生必须 null；VLESS 必须对象 |
| `vless.stream` | `tcp` 或 `websocket`；默认建议 tcp，WS 为显式便捷选项 |
| `vless.server_name` | 建议 `meshway.local`；平台证书 SAN / 客户端 SNI 校验名，与 HTTP Host 独立 |
| `websocket_path` | WS 建议 `/meshway`；准确路径，1–1024，不能含 query / fragment / 控制字符 / 空格 |
| `websocket_host` | WS 默认建议所填公告 hostname；若未填公告，建议 `meshway.local`；1–253，精确 Host 匹配 |
| TCP 的 WS 字段 | 两者必须显式 null；错误组合 422 |

这些是表单建议默认值，不是设备配置参数。平台保存后生成并下发 UUID / TLS
材料；监听端私钥永远不经普通 Web 读取。证书/SNI 变更创建新证书和 UUID，
必须通过新 revision 撤退旧代际；单纯 bind / endpoint / WS path / Host 变更
保留 UUID / 证书 / 私钥、推进材料版本与网络 revision。

### 首次接入可选 listener

`createManagedEnrollmentToken` 请求：

```json
{
  "expires_in_seconds": 600,
  "initial_listener": {
    "id": "vless", "node_id": "", "enabled": true,
    "transport": "vless", "bind_address": "0.0.0.0", "bind_port": 443,
    "advertised_endpoints": [],
    "vless": { "stream": "tcp", "server_name": "meshway.local", "websocket_path": null, "websocket_host": null }
  }
}
```

**初始意图的 `node_id` 必须空字符串**：消费令牌时由同一注册事务填入新身份，
调用方不能指定另一身份。令牌原值仍只在创建时返回一次；只创建令牌不推进
业务 revision。接入使用既有 `enrollAgent`，App兑换、心跳继续使用各自现行合同，库存改为纯元数据schema2。
首次消费同时绑定节点、地址、令牌请求摘要和 listener/material；失败整体回滚。
同绑定恢复不重新生成材料；撤销不能恢复；同网络 ID 冲突不会吞掉旧 listener。
无需 listener 时继续既有 `createEnrollmentToken`。

## 错误与 CAS

- `400`：JSON / 未知字段 / TTL / 初始 node_id 错误。
- `401`：管理员或节点凭据无效；管理员凭据不能读取节点秘密接口。
- `403`：节点已撤销。
- `404`：网络不存在，或节点不属于所选网络 / 已撤销。
- `409`：过期 `expected_revision`、listener ID 冲突、企图换 owner / transport。
- `422`：bind / endpoint / SNI / WS 组合无效、预算耗尽或托管 listener 经旧连接编辑器被修改。
- `413` / `415`：32 KiB 管理请求预算 / JSON Content-Type。
- `500`：净化的存储错误，不包含输入、UUID、私钥或远端 WS 错误 body。

无语义变化的保存不推进 revision；有变化在同一事务推进网络 CAS。
UI 保留失败草稿及原网络/CAS。旧高级 connections 编辑器可以维护 candidates，
但托管 listener 只能经本 API 增改停用。目标为托管 listener 的 VLESS candidate
引用会规范化为 `managed.<listener-id>`；不需要让用户编辑 profile 名。
停用但仍被旧 candidates 引用时保留源计划并明确失效、发布安全撤回，不能偷偷
删候选或绕过授权改走另一条路径。重新启用可恢复原候选；endpoint 变化后原
candidate 必须仍明确引用新的公告地址，否则计划失效，供高级入口修复。

## 状态来源：配置、材料、保存、运行和可达性

| 显示状态 | 权威来源 / 正确解释 |
| --- | --- |
| bind / 公告地址 / WS / TLS 意图 | `getManagedListeners`；公告为管理员明确填写，不是实测 |
| 编译能力 | 原公钥/承载能力声明 + 新 `publishManagedTransportCapabilities`；不是正在监听 |
| 材料版本 / 证书公有状态 | `material_version` / `certificate_sha256`；不显示秘密 |
| 已校验保存 ACK | 原 `getConnections.nodes[].acknowledged_*`；Agent已校验并私有落盘，不等于 active |
| 实际 listener 绑定 / 错误 | `getConnectionRuntimes` 中 listener ID / bound / reason；结合 current / stale / epoch / revision |
| 实际认证邻接 | 同报告中 listener ID、candidate ID、peer、transport、initiator、`probe_fresh`；只对该 peer / transport 有效 |
| socket 的 bind、候选、来源、过期 | 现有 `getEndpointDiscoveries`；保留 resource kind（Listener / OutgoingCandidate）、resource_id、socket_id、generation、epoch 和 revision 绑定 |
| 未知 / 过期 | 无当前报告或观测时显示 unknown/stale，不补节点级 global reachable 布尔值 |

M2 的 UDP STUN 观测不变；**UDP 映射不证明 TCP / VLESS / WS 可达**，不会修改
管理员 bind / 公告。WS/TLS 没有新增 STUN、UPnP、NAT-PMP、PCP 或公网可达证明。
当前M2的socket明细仅覆盖Native UDP；VLESS/WS的listener绑定取实际runtime报告，
其TCP拨号socket的ephemeral本地地址尚未加入M2明细，UI必须显示未知，不能从listener
配置或UDP观测补造。外层socket工厂保留candidate/listener用途绑定，供后续扩展。
无 listener 的拨号 socket 保持 OutgoingCandidate 关联；通道双向不制造 listener
或双向业务授权。后续位置服务应从带来源/时间的真实底层公网 IP 观测做 controller
GeoIP；不得使用 Mesh IP、选中出口 IP 或管理员 country；缺测/过期用 auto fallback。

## 节点专属下发与私有保存

| NodeBearer API | 生成操作 |
| --- | --- |
| POST `/api/v1/agent/managed-transport-capabilities` | `publishManagedTransportCapabilities` |
| GET `/api/v1/agent/managed-connections` | `getAgentManagedConnections` |
| POST `/api/v1/agents/mesh-execution` | `negotiateMeshExecution` |

新快照显式包含 network ID、原公共 snapshot、获准材料和合并摘要。
一个 Store 请求独占作用域读同 revision；无别的写入可穿插材料投影。
仅 listener 所属节点取得私钥；直接 outgoing candidate 的拨号节点只取得
该 listener UUID / 证书信任根 / SNI / WS 参数；其他节点收不到其材料。
Agent验证角色、摘要、绑定、证书有效性/SAN/指纹、私钥匹配、版本下限与本机能力，
原子保存 `agent.json.transport-materials.json`（0600）并 fsync，再发送原 saved ACK。
该 journal 只能提供恢复/回滚下限，**没有 fresh HTTP 不能启动执行或续租**。

控制面 `transport-material.key` 是独立0600加密密钥；恢复需要它和 SQLite一起备份。
材料以 ChaCha20-Poly1305 加密，AAD绑定网络/listener/owner/版本。缺钥且已有材料
时拒绝生成替代密钥。完整当前schema在IMMEDIATE事务中初始化；普通启动拒绝损坏或未知结构，开发重置须显式执行。

运行命令沿用 `run --apply-connections` 或显式 IPv4 `run --apply-dataplane`，
新托管流程不传 `--connection-profiles`。平台生成并下发材料，不再提供旧profiles导入。
心跳更高revision、源撤回、ACK冲突及lease期限独立 fence 旧代际；WS仍受5秒联合
TLS/WS/VLESS/Noise握手deadline、32并发、256KiB消息/frame限额与backpressure约束。
HTTP upgrade累计读取另限制64KiB，避免单个未完成header无限增长。
取消握手或部分写入时关闭整个carrier，不在截断流上重试；核心重放检查不变。

外层 TCP 工厂在 connect / listen 前提供用途保护边界，TLS / WS共用它。
用途受限SO_MARK/RPDB、protected HTTP与owning工厂模块已合入，**正常主循环尚未启用生产owner**。全IP出口必须先完成真实保护；accept后标记不能补救SYNACK，不能用目的IP或整个进程豁免替代。见[系统保护合同](/underlay-routing)。

TLS/WS/VLESS共用protected外层stream注入边界，不重写各自握手。Manager prepared owner换代先withdraw/join；生成器secret Debug适配与HTTP transport注入分别维护，不能相互覆盖。

## 验证

`python3 experiments/managed-transports/run.py` 使用正常 Controller、唯一policy与Agent生命周期，独立Docker namespace、仅容器NET_ADMIN/TUN、共享performance lock与精确清理。执行前须完成当前消费者整合；本次源码与契约检查不声称已经重跑真实VLESS包流。
