# 地址类别与位置依据

Server 从调用者可见的 endpoint 派生两项事实放在 State：每个 IP 的地址类别（`addressScope`），以及节点位置所依据的那个本机公网观测（`location`）。State 不含 GeoIP／ASN 查询结果；地点由客户端自己按 `public` 地址查询。Node 报告只包含实际地址和来源，不自行声明国家、位置或 ASN。

## State 位置

```text
StateView.nodes[nodeId]
  ├─ report                            # full NodeState 或裁剪后的 PeerState
  └─ derived
       ├─ addressScope[ip]             # 此节点可见 endpoint IP 的类别
       ├─ location                     # 位置依据的本机公网观测，或 null
       └─ locationAbsent               # location 为 null 的原因
```

`addressScope` 按规范 IP 去重，多个 socket／端口共享一项。Server 先裁剪调用者可见的 endpoint，再派生这两项，不通过派生值泄露隐藏接口。网络 State（GET 与 SSE）、单节点 State 和 Node 的 peer 查询使用同一派生流程。

## addressScope

| 值 | 含义 |
| --- | --- |
| `public` | 全局可路由的单播地址 |
| `nonPublic` | 私网、ULA、CGNAT、链路本地、loopback、多播、未指定、文档保留及其他保留地址 |
| `mesh` | 本网络的 Mesh 地址 |

- 只含调用者可见 endpoint 中的字面 IP；域名型 endpoint 不产生条目，不用 Server 所在位置解析出的地址代替。
- configured endpoint 的字面公网 IP 也是 `public`，但只是“声明入口的地址”，可能指向 NAT、CDN、代理或远端转发入口，不作为设备位置。
- 类别不代表 endpoint 已验证可达；原 endpoint.origin、socket 代际与期限继续保留。

## location {#location}

`location` 只从有效的本机 underlay 公网观测中选择：当前 socket 的 STUN 结果或公网 interface 候选。单纯 configured 声明、Server HTTP 来源 IP、Mesh 地址、所选 Meshway 出口地址和管理员标签都不能替代。不需要任何数据集。

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `address` | string | 所选的公网 IP |
| `endpointSource` | `stun` \| `interface` | 观测来源 |
| `observedAt`／`expiresAt` | Timestamp | 原观测时间与期限；到期后不再是当前依据 |
| `sourceSocketId` | string | 提供该观测的实际 socket |
| `sourceGeneration` | integer | 该 socket 的资源代际 |

### 选择规则

1. 合格观测按等级分组：STUN 先于 interface，同来源内 IPv4 先于 IPv6。
2. 取存在当前（未到期）观测的最高等级。该等级只有一个不同地址时，它就是 `location`；有多个不同地址时返回 `locationAbsent: ambiguousObservation`，不随 map 顺序挑选。
3. 同一地址有多条观测（多个 socket）时，取 `expiresAt` 最晚的一条，相同再取 `sourceSocketId` 最小的一条。

### locationAbsent

| 值 | 含义 |
| --- | --- |
| `noObservation` | 没有合格的本机公网观测 |
| `observationExpired` | 合格观测都已到期，或报告本身已不是当前报告 |
| `ambiguousObservation` | 最高等级有多个不同的合格地址 |

`location` 存在时 `locationAbsent` 为 null。没有坐标，也不填零；VPN／代理出口的地址反映该地址的位置，不保证设备物理位置。

## 时间

派生值不制造新的 IP 观测：读取 State 不刷新原 observedAt／expiresAt。endpoint 到期后，它的地址仍可能出现在 `addressScope`（只要 endpoint 仍在可见报告里），但不能继续作为 `location`。

## 数据集部署 {#datasets}

GeoIP／ASN 数据集只供自动路由方案 `routing.egress.auto` 使用（自动方案尚未实现，见 [Server](/v2/server)），State 不返回其查询结果。数据集由 Server 主机的运维通过本机维护命令导入，没有管理 API，也不由 Server 自行下载：

```sh
meshway-server 停止后
ip-datasets --data-dir <data> import --kind geoip --file GeoLite2-City.mmdb
ip-datasets --data-dir <data> import --kind asn   --file GeoLite2-ASN.mmdb
ip-datasets --data-dir <data> status
ip-datasets --data-dir <data> lookup 93.184.216.34
```

- 文件是 MaxMind DB 格式（GeoLite2／GeoIP2）。`geoip` 接受 `database_type` 含 City 或 Country 的库，`asn` 接受含 ASN 的库；类型不符、无法解析或超过 128 MiB 的文件拒绝导入，不替换现有数据集。GeoLite2 需要 MaxMind 账号和许可才能下载，由运维取得文件，Server 不持有 MaxMind 凭据。
- 导入在同一事务写入不可变 artifact 并更新该类数据集的持久 head；每类各有一个当前版本，再次导入即替换，此后的查询和重启恢复均引用该 head，不用时间戳或随机 ID 推断提交顺序。无法构造合法有限坐标的记录查询为失败，不发布越界位置。
- 命令与 Server 共用同一数据目录要求（绝对路径、私有权限）、锁和密钥检查：必须先停止 Server，否则报告存储被占用；其他打开失败按 Server 启动的同一原因报告。Server 下次启动时加载当前数据集。
- `status` 显示两类数据集的 artifact ID；`lookup` 对任意地址做一次诊断查询（非公网地址显示 `not applicable`），不产生观测，也不影响 State。

开发环境可将已取得的数据保存在独立部署资产目录，并在显式重置业务库时从校验快照重新导入。文件清单、摘要与重置顺序统一见[独立 GeoIP／ASN 部署资产](/v2/development#独立-geoip-asn-部署资产)。
