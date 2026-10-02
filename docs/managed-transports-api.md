# P2a 托管 listener / VLESS API 对接

状态：后端实现分支 `feat/managed-vless`；具体验收记录见
[`managed-transports-progress.md`](./managed-transports-progress.md)。Web 表单尚未接入。
本页的类型、schema 和操作均由 Rust / 实际路由生成，不另维护 TS DTO。

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

## 管理 API（服务端管理员鉴权）

前缀 `/api/v1/networks/{network_id}`。当前仍使用 global admin bearer。
服务端先验证管理员，再解析网络；合法 NetworkId 本身不是权限。
Web action 固定渲染时网络 ID 和 CAS，不能在 mutation 时改用选择 cookie。

| 方法 / 路径 | 生成 operation ID / SDK 导出 | 成功响应 |
| --- | --- | --- |
| GET `/managed-listeners` | `getManagedListeners` / `getManagedListeners` | `ManagedListenersSnapshot` |
| POST `/managed-listeners` | `saveManagedListener` / `saveManagedListener` | `ManagedListenersSnapshot` |
| POST `/managed-enrollment-tokens` | `createManagedEnrollmentToken` / `createManagedEnrollmentToken` | 既有 `CreateEnrollmentTokenResponse` |

SDK：admin `(baseUrl, adminToken, networkId[, body])`，由生成 Zod 校验响应。
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
业务 revision。接入使用既有 `enrollAgent`，原 App 兑换、心跳、库存完全兼容。
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
| GET `/api/v1/agent/managed-dataplane` | `getAgentManagedDataplane` |

新快照显式包含 network ID、原公共 snapshot、获准材料和合并摘要。
一个 Store 请求独占作用域读同 revision；无别的写入可穿插材料投影。
仅 listener 所属节点取得私钥；直接 outgoing candidate 的拨号节点只取得
该 listener UUID / 证书信任根 / SNI / WS 参数；其他节点收不到其材料。
Agent验证角色、摘要、绑定、证书有效性/SAN/指纹、私钥匹配、版本下限与本机能力，
原子保存 `agent.json.transport-materials.json`（0600）并 fsync，再发送原 saved ACK。
该 journal 只能提供恢复/回滚下限，**没有 fresh HTTP 不能启动执行或续租**。

控制面 `transport-material.key` 是独立0600加密密钥；恢复需要它和 SQLite一起备份。
材料以 ChaCha20-Poly1305 加密，AAD绑定网络/listener/owner/版本。缺钥且已有材料
时拒绝生成替代密钥。新增表与 v9→v10 DDL 均在 IMMEDIATE 事务中完成，失败回滚；
旧身份、公钥、地址、JSONC、ACK、lease和revision不变。

运行命令沿用 `run --apply-connections` 或显式 IPv4 `run --apply-dataplane`，
新托管流程不传 `--connection-profiles`。保留旧 profiles 仅供兼容/迁移。
心跳更高revision、源撤回、ACK冲突及lease期限独立 fence 旧代际；WS仍受5秒联合
TLS/WS/VLESS/Noise握手deadline、32并发、256KiB消息/frame限额与backpressure约束。
HTTP upgrade累计读取另限制64KiB，避免单个未完成header无限增长。
取消握手或部分写入时关闭整个carrier，不在截断流上重试；核心重放检查不变。

外层 TCP 工厂在 connect / listen 前提供用途保护边界，TLS / WS共用它。
当前仅Mesh网段接管使用系统underlay；**本切片没有实现SO_MARK/RPDB或reqwest
pre-connect保护**。全IP出口接入必须先完成平台保护；accept后标记不能补救SYNACK，
不能用目的IP或整个Agent进程豁免替代。参照主线 `docs/underlay-routing.md` 实验。

### 与 protected-underlay 并行任务的最小整合点

- `apps/agent/src/managed_connections/sockets.rs`：`Protection::prepare` 接收
  `Purpose::Listener { listener_id }` 或 `Purpose::Dial { candidate_id, peer_node_id }`，
  先创建/保护 `TcpSocket`，然后公共 `listen` / `dial` 才调用 bind/listen/connect。
  `accepted` 仅作接受后补充检查，不能代替listen前保护。
- `Session::start_with_packet` 已接收 `Arc<dyn Protection>`；Manager保存工厂，
  当前由 `Manager::new` 使用 `SystemUnderlay`。协调者可在整合时增加模块可见性/
  constructor或setter，把真正平台工厂注入；不需要重写TLS/WS/VLESS握手。
- 生成器秘密保护独立在
  `apps/controller/src/bin/generate-rust-client/secret_debug.rs`，只适配生成models。
  `generate-rust-client.rs` 调用 `secret_debug::adapt`；transport注入模板仍针对
  client构造，两者不要互相覆盖。该adapter没有改变request序列化。
- `init/ready`、packet `Generation` 和 writer退休继续使用最新主线 Runtime /
  packet_io；本切片未改它们，不能为适配WS回退旧接口。

## 旧 profiles 私有导入 / 切换

旧节点继续原 profiles，迁移不会扫描或替换它们。协调者仅对自己的 demo进行：

1. 停止该私有 controller（导入命令取得原目录独占锁），保留原SQLite与材料备份。
2. 在0700目录生成0600的私有JSON输入：`{ network_id, legacy_profiles, request, material }`。
   `legacy_profiles` 为显式指定的原server profiles私有副本绝对路径；关联原证书/私钥
   在同目录保留路径关系，并使用0600。导入器从旧listener引用定位该server profile，
   必须证明输入UUID、原certificate前缀和private key与这些文件完全一致，才允许切换；
   不自动扫描主机或以调用方声称“原材料”代替对比。
   `request` 为上述SaveManagedListenerRequest，使用当前CAS；TCP模式、listener ID、
   owner、bind、公告端点必须与原监听严格一致。`material` 用生成权威
   `ManagedVlessMaterial` 形状，version=1、credential_ref=`managed.<id>`，填入**原**
   UUID、原server证书链（必要时附原CA）、原private key、原SNI和leaf DER指纹。
   这是私有本地迁移文件，不能送入Web、普通日志或本任务产物。
3. 执行（不会输出秘密）：

   ```sh
   cargo run -p meshway-controller -- import-managed-vless \
     --data-dir /absolute/private/controller --input /absolute/private/import.json
   ```

4. 输入先验证 UUID、证书有效期/SAN、key匹配、fingerprint和原listener等价性，
   然后同一CAS事务删除源中该legacy listener、改候选引用、保存**原材料**并发布。
   任一无法证明的部分拒绝导入；原材料和源不变。不得为“迁移成功”新建替代UUID/key。
5. 重新启动controller；Agent以原state/key运行，移除原profile参数后fresh同步。
   检查current revision、saved ACK、listener bound、peer probe和真实业务；确认后
   才清理由协调者拥有的私有导入文件与旧副本。第一轮导入限TLS/TCP；之后再通过
   Web/API显式切WS，保留材料并发布新代际。

协调者不要直接对live套本页示意命令。独立验收入口：
`python3 experiments/managed-transports/run.py`，自带同一performance lock、独立
Docker namespace、仅容器NET_ADMIN/TUN、所有权精确清理与私有外部产物。
