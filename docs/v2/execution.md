# 按需执行许可

Server 按需计算获准候选／完整路径并补齐一致配置，Node 须在获准范围内结合 State 选择和拨号。不预展开全网节点对，也不让设备凭自构造图获得新许可。

## 需求与发现

```text
Node 产生业务目标需求
  → Server 从凭据确定 source，检查当前身份与 access
  → 根据 topology / routing / 允许方案计算有界候选及完整路径
  → 确定相关 peer / 中转 / 出口依赖与协议材料
  → 向参与节点发布相容的执行配置
  → 各 Node 独立校验／保存／应用，接收方检查相容的执行许可
  → Node query/watch 已获准依赖的 State
  → 本地选择当前可用候选并认证建连
```

请求者提交业务目标和必要协议／端口等输入，不通过自行填写中转列表或 Tag 扩大可见范围。Server 返回明确的需求决定，当前离线与未授权分别处理。

例如 A 获准访问 B，但两者需经 C：Server 决定 C 属于执行依赖，补齐 A/C/B 所需身份、材料和许可。A 不必先猜出 C 的 ID，更不能通过 State query 自授读取或使用 C 的权限。

## 需求续租

`POST /agent-api/v1/demands/renew`（生成操作 `renewNodeDemand`，请求 `RenewDemandRequest`）使用当前 Node bearer，对同一活动实例的已有需求续租。

| 字段 | 行为 |
| --- | --- |
| `instance` | 必填 InstanceRequest，含 instanceId／generation；须匹配当前有效实例 |
| `requestId` | 已有需求的 ID，不创建新 intent，不改变原目标／协议／端口／profile |
| `expiresAt` | UTC RFC 3339 绝对期限；实际延长不得超过现有 instance 期限或 Server 当前时刻后 300 秒 |

Server 在同一事务重新检查当前主体、网络、instance 和需求存活状态。缺失、已取消、过期或另一实例的需求不能续活；不会为了满足请求自动延长 instance。延长保留原 create digest，创建请求的重放也不延长期限。

重复或更早的 expiresAt 不延长、不为本次重放写入新 publication／执行 lease／历史；同一事务仍可清理其他到期需求并因此更新发布。真正延长后按网络内**全部有效需求**重新合成完整目标，不是 last-demand-wins。响应仍为 publication 和 decision；调用方须重新同步保存目标、取得匹配执行快照后才可使用许可，HTTP 反馈本身不授予 socket 或包流执行权。

Node 的本机取消与连续性门槛见 [Control](/v2/node/control#执行快照)，活流／circuit 续租规则见[数据面](/v2/node/dataplane#包与流边界)。

## 原子执行刷新

`POST /agent-api/v1/configuration/refresh`（`refreshNodeConfiguration`）使用 Node session 和既有 `NodeConfigurationRequest { instance, base }`，返回 `NodeConfigurationResponse { lease, full, delta }`。Server 在同一个 SQLite owner 事务和时点重新鉴权、清理到期需求、续租、检查执行资格，再读取该精确目标的完整内容或 delta。响应中的 lease 与目标引用不能来自不同发布。

这是显式续租操作。`queryNodeConfiguration` 仍只读取／同步目标，不因读取而延长 lease。Node 必须验证响应、应用 delta 并原子保存后再取得执行快照；响应不表示已经安装。

Node execution lease 最长为 Server 当前时点后 60 秒，并受当前 instance 截止约束。每条业务／DNS／诊断许可另保留自己的截止，数据面、邻接与排队输出分别检查；Node lease 有效不能续活过期 permit。一条需求到期只结束相应许可，其他活流和空闲 TUN 安装仍受各自期限约束。停用、身份／密钥失效、不可用发布与地址退出窗口仍阻止 execution grant。

## 各方职责

| 参与方 | 职责 |
| --- | --- |
| Server | 身份解析、业务授权、候选／完整路径计算、依赖闭包和一致发布 |
| Source Node | 在获准集合内结合当前 State 选路、选 endpoint／承载并发起业务 |
| 中转 Node | 验证自己被允许承担的完整路径角色，只转发对应流量 |
| Target／Exit | 根据自身可信配置验证认证 origin、目的范围、完整 path 及流资格 |

业务 A→B 不等于 A 必须拨 B。可以由 B 拨 A、复用双向认证通道；协议的 client/server 角色需与外层建连方式明确绑定，不能只按业务方向生成参数。

## 许可与 State

候选身份、材料、业务范围和允许路径进入受保护的 NodeConfiguration。临时地址、可达性、连接结果走 State，State 不夹带新的访问授权或秘密材料。

取得一个新中转／出口依赖时，先使所需配置完整，再使用其 State。尚未获得相容配置的接收方拒绝／等待，不信发送端附带的自制 plan。

参与节点按[普通应用流程](/v2/configuration-reference/deployment)独立处理。若必要对端尚未应用相容配置，对应业务等待／拒绝；其他节点仍可完成自己的安装。用户配置应用与后续按需发布按各自 publication 跟踪进度。

NodeConfiguration 的候选、许可、本机 egress、诊断与需求结果字段见[节点配置](/v2/configuration-reference/node-configuration)。

## 路径与会话

- 每个候选完整路径都满足原业务 access、topology 和[逐段递归 first-match routing](/v2/configuration-reference/routing#递归路段)，不通过备用路径跳过必经点；路段起点不替代原业务源身份。
- 当前许可绑定网络、执行发布、认证源身份／地址、业务目的范围和有序 path。
- 新流可选择不同的获准路径；一经建立／预留的流和回复保留原完整逻辑路径，故障不能偷偷改路。
- 承载主备可在同一逻辑邻接内切换；源节点自改配置不能让正常接收端认可额外路径或反向新流。

## 规模与编译边界 {#编译边界}

当前 policy compilerVersion 为 `meshway-demand-4`；识别出的旧派生缓存失效与历史保留见[执行缓存版本](/v2/storage#执行缓存版本)。

Server 可复用同一源版本的编译事实，只对合法需求产生有界候选／切片；节点数增加不要求预计算所有简单路径，也不要求为每次实际探测重新保存管理员配置。编译按网络合并有效需求，出口／每出口路径分别有界；TLS 不相容只排除对应候选，材料缺失不降级。独立诊断许可不产生普通业务 permit。

一次需求可以影响少量参与节点；发布引用和相关依赖必须一致，不把无关节点的状态更新视为全网权限变更。持久顺序、各节点 digest、lease 和切换规则见[配置应用](/v2/configuration-reference/deployment#publication)。发布不表示 Node 已安装或业务连通，运行范围见 [Server](/v2/server) 与 [Node](/v2/node)。

## 限制

- 可信 DNS 地址关联及 TTL 参与域名与 IP 需求求值；当前 Server 尚未接入可信 DNS 输入，域名需求因此以缺失依赖结束，见 [Server 当前能力](/v2/server#当前能力)。
