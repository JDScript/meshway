# GeoIP 与 ASN

Server 使用可见 endpoint 中的 IP，在本地数据集查询 GeoIP／ASN，作为 State 派生信息返回。Node 报告只包含实际地址和来源，不自行声明可信国家、位置或 ASN。

## State 位置

```text
StateView.nodes[nodeId]
  ├─ report                            # full NodeState 或裁剪后的 PeerState
  └─ derived
       ├─ ipInfo[ip]                     # 此节点可见 endpoint IP 的 GeoIP／ASN
       └─ location                       # 从合格本机观测选择的节点位置及来源
```

`ipInfo` 按规范 IP 去重，多个 socket／端口共享同一次查询结果；来源和有效期仍取自各自 endpoint。Server 先裁剪调用者可见的 endpoint，再返回对应 ipInfo，不通过元数据泄露隐藏接口。

节点详情、网络 State snapshot／SSE 和相关 peer query 使用同一派生流程。需要轻量视图时可以不展开详细地理字段，展开与否有明确视图类型。

## IpInfo

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `geoip` | GeoIpResult | 地理数据查询结果 |
| `asn` | AsnResult | 自治系统查询结果，与 GeoIP 可独立成功／缺失 |

map key 是被查询的 IP。两个结果采用相同的状态形状，data 分别使用下面的确定类型：

| status | 字段 | 语义 |
| --- | --- | --- |
| `available` | `datasetId`、`lookedUpAt`、`data` | 该版本数据集存在结果 |
| `notFound` | `datasetId`、`lookedUpAt` | 查询成功但没有条目 |
| `unavailable` | `reason` | datasetUnavailable／lookupFailed；不冒充没有条目 |
| `notApplicable` | `reason` | nonPublicAddress／meshAddress；不作公网元数据推导 |

`datasetId: ArtifactId` 固定数据版本，`lookedUpAt: Timestamp` 是实际执行查询的时间。reason 使用封闭枚举；未取得 ASN 不填 AS0，未取得位置不填 `(0,0)`。

### GeoIpData

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `country` | string? | 数据集给出的 ISO 3166-1 alpha-2 国家／地区码 |
| `region` | string? | 一级地区名称／标识，随提供方格式明确 |
| `city` | string? | 城市名称 |
| `position` | GeoPosition? | 数据集实际提供的近似位置；没有则 null |

### GeoPosition

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `latitude` | number | -90 至 90 |
| `longitude` | number | -180 至 180，与 latitude 成对出现 |
| `accuracyRadiusKm` | number? | 提供方给出的估计半径；未提供为 null |

GeoIP 不是 GPS，也不是节点机房的独立认证。地图标明近似来源；没有 position 时可以显示明确标注的地区聚合或“待定位”，不能自行生成一个精确节点坐标。

### AsnData

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `number` | u32 | 实际 ASN，展示时可格式化为 AS12345；0 不是未知值 |
| `organization` | string? | 数据集记录的网络组织名称，不等于 Node 所有人 |
| `prefix` | IpPrefix? | 数据集返回的匹配网段，若提供 |

ASN 描述 IP 的网络归属，不是当前 BGP 路径／可达性测量，也不能据此断言运营商、出口或中继职责。

## 哪些地址可以查询

- 字面 IPv4／IPv6 公网地址可查询。私网、ULA、CGNAT、链路本地、loopback、多播、未指定、文档保留地址及本网络 Mesh 地址明确 notApplicable。
- configured endpoint 的字面公网 IP 可以展示它自己的 GeoIP／ASN，但仅是“声明入口的 IP 信息”。它可能指向 NAT、CDN、代理或远端转发入口。
- 域名型 endpoint 不产生 IP 查询项。不能用 Server 所在位置解析出的 CDN 地址冒充节点实际使用的地址。
- 查询数据库不代表 endpoint 已验证可达；原 endpoint.origin、socket 代际与过期规则继续保留。EndpointCandidate 没有 validation 字段。

## 节点位置与自动方案

`derived.location` 只从有效的本机 underlay 公网 IP 观测选择，例如当前 socket 的 STUN 结果或合格公网 interface 候选。单纯 configured 声明不成为设备实测地区；Server HTTP 来源 IP、Mesh 地址、所选 Meshway 出口地址和管理员 country 也不能替代。

位置返回所选 IP、socket／代际、endpoint source、原 observedAt／expiresAt，以及对应的 GeoIP 数据版本和结果。它引用同一 ipInfo 查询结果，Console 地图与自动路由方案共用这份依据，不各自计算国家。

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `location` | GeoObservation? | 有有效合格观测且地理结果可用时返回 |
| `locationAbsent` | LocationAbsentReason? | location 为 null 时给出原因 |

LocationAbsentReason 为 noObservation、observationExpired、datasetUnavailable、notInDataset、lookupFailed、ambiguousObservation。合格观测无法形成一致位置时，保留每个 IP 的元数据并返回 ambiguousObservation，不随 map 顺序随机选择地区。

不能确定地区时，自动方案使用已定义 fallback。VPN／代理出口 IP 的 GeoIP 反映该 IP 的位置，不保证设备物理位置。

## 缓存与时间

以 `(IP, datasetId)` 缓存查询，GeoIP 和 ASN 可使用不同数据版本。复用缓存保留原 lookedUpAt；查询 State 不制造新的 IP 观测。

endpoint 到期后，其 GeoIP／ASN 仍可能是正确的数据库映射，但不能继续作为当前节点位置／自动地区方案的有效依据。数据集更新重新查询仍有效的候选并推进派生 State，既不改管理员 Config revision，也不重写 Node 原报告。

数据集属于 Server 实例资源，网络 Owner 不因管理网络获得实例数据集写权限。部署范围见 [Server](/v2/server)。无数据集或有效观测时返回对应缺失原因，不伪造位置；数值遵守 Schema 的有限值与范围约束。

## 数据集部署 {#datasets}

数据集由 Server 主机的运维通过本机维护命令导入，没有管理 API，也不由 Server 自行下载：

```sh
meshway-server 停止后
ip-datasets --data-dir <data> import --kind geoip --file GeoLite2-City.mmdb
ip-datasets --data-dir <data> import --kind asn   --file GeoLite2-ASN.mmdb
ip-datasets --data-dir <data> status
ip-datasets --data-dir <data> lookup 93.184.216.34
```

- 文件是 MaxMind DB 格式（GeoLite2／GeoIP2）。`geoip` 接受 `database_type` 含 City 或 Country 的库，`asn` 接受含 ASN 的库；类型不符、无法解析或超过 128 MiB 的文件拒绝导入，不替换现有数据集。GeoLite2 需要 MaxMind 账号和许可才能下载，由运维取得文件，Server 不持有 MaxMind 凭据。
- 导入在同一事务写入不可变 artifact 并更新该类数据集的持久 head；每类各有一个当前版本，再次导入即替换，此后的查询和重启恢复均引用该 head，不用时间戳或随机 ID 推断提交顺序。无法构造合法有限坐标的记录返回 `lookupFailed`，不发布越界位置。
- 命令与 Server 共用同一数据目录要求（绝对路径、私有权限）、锁和密钥检查：必须先停止 Server，否则报告存储被占用；其他打开失败按 Server 启动的同一原因报告。Server 下次启动时在接受任何报告之前加载当前数据集。
- `status` 显示两类数据集的 artifact ID；`lookup` 对任意地址做一次诊断查询，输出与 State 派生相同的结果（含 `not applicable` 的非公网地址），不产生观测。

开发环境可将已取得的数据保存在独立部署资产目录，并在显式重置业务库时从校验快照重新导入。文件清单、摘要与重置顺序统一见[独立 GeoIP／ASN 部署资产](/v2/development#独立-geoip-asn-部署资产)。
