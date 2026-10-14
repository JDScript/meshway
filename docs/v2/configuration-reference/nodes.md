# Nodes

以已注册 Node ID 为 key 的静态设置。hostname 是配置的权威值；运行观测见 [State](/v2/state-reference)。

## Structure

配置节选；Node 和引用的 Tag 须在当前网络有效：

```json
{
  "nodes": {
    "<node-id>": {
      "hostname": "backend-1",
      "remark": "开发后端",
      "identity": { "type": "tag", "tags": ["tag:backend"] },
      "enabled": true,
      "addresses": {
        "ipv4": "100.80.0.10",
        "ipv6": "fd42:7a65:8800::10"
      },
      "listeners": {},
      "exitEnabled": false,
      "region": "US"
    }
  }
}
```

## Fields

| 字段 | 类型 | 默认 | 说明 |
| --- | --- | --- | --- |
| `hostname` | `string` | 首次接入时生成 | 网络内唯一的规范 DNS 名称 |
| `remark` | `string` | `""` | 自由备注，不影响身份、DNS 或授权 |
| `identity` | `NodeIdentity` | 接入时写入 | User／Tag 有效授权身份的唯一配置源 |
| `enabled` | `boolean` | `true` | 是否启用本节点业务执行；停用可随配置回滚 |
| `addresses` | `NodeAddresses` | 接入时自动分配并保存 | Mesh IPv4／IPv6 的期望分配；可显式修改 |
| `listeners` | `Record<string, Listener>` | `{}` | [本节点入站配置](/v2/configuration-reference/listeners) |
| `dialers` | `DialerSet` | 省略则继承 | 显式 `{}` 禁用主动建连；非空集合替换默认 |
| `exitEnabled` | `boolean` | `false` | 管理员允许此节点承担出口；不是执行就绪状态 |
| `region` | `string` | 省略 | 管理员声明的地区，见 [region](#region) |

## hostname

使用单个 DNS label：小写字母、数字及中间连字符，1–63 个字符，首尾为字母或数字。手工改名重名时拒绝保存；首次接入规范化请求中的 hostname 并生成唯一后缀。

```text
hostname: backend-1
dns.domain: mesh.example
FQDN: backend-1.mesh.example
```

Node 查询投影此名称，没有第二个独立可写的显示名。Node 上报的 OS hostname 留在 State，不改写配置名称。

## addresses

`ipv4`／`ipv6` 分别为地址或 null，必须属于同版 addressPools，网络内唯一。已有节点省略时沿用，首次省略时自动分配；显式 null 清除该族地址。

改址不改变 Node ID 或密钥身份。普通保存对越界／重复地址报错；显式新地址与池修改可以同一配置事务完成。旧地址退出和复用见 [Addressing](/v2/configuration-reference/addressing)。

## identity

采用互斥判别对象：

<!-- schema: NodeIdentity -->
```json
{ "type": "user", "user": "alice@example.com" }
```

<!-- schema: NodeIdentity -->
```json
{ "type": "tag", "tags": ["tag:backend"] }
```

| 分支 | 字段 | 类型 | 约束 |
| --- | --- | --- | --- |
| user | `type` | `"user"` | 固定值 |
| user | `user` | `string` | 本网络有效用户的可读邮箱，版本绑定稳定 UserId／MembershipId |
| tag | `type` | `"tag"` | 固定值 |
| tag | `tags` | `string[]` | 非空、不重复，引用 tagsOwners |

user 分支不接受 tags，tag 分支不接受 user。Node 数据库记录不保存另一份 owner／subject／tags。接入批准与身份切换负责验证谁有权写入此字段；Tag→User 或改绑其他 User 仍须设备完成相应用户登录，整份配置保存／回滚也不能绕过。

## enabled

`false` 撤回该节点的业务执行许可，停止依赖它的路径；凭据仍可认证以同步配置、报告停用进度并接收后续启用。改回 `true` 时重新校验身份、授权与路径，不恢复旧流资格。

永久接入吊销／密钥吊销属于认证记录，不能由 enabled=true 或配置回滚清除。当前状态可投影为“配置已启用，但凭据已吊销”，具体来源保留，不能把两者合成一个可写状态。

## region

ISO 3166-1 alpha-2 代码（两个大写字母，如 `US`、`JP`），只作为[自动方案](/v2/configuration-reference/routing#auto) `auto` 的输入：源节点的有效方案为 `auto` 时，按此代码在 `routing.egress.auto.rules` 中选方案。省略表示未声明（`auto` 采用 fallback，basis `noTrustedRegion`），规范输出省略该字段；`null` 不是合法值，编辑操作 `setNodeSettings` 的 `fields.region: null` 表示清除。不合法的值以 `invalid_country_code` 拒绝（路径 `/nodes/<id>/region`，`params.value`）。

只有 Owner／Admin 能写：它是普通配置字段，经配置提交入口保存；接入的初始设置（`initialSettings`）与节点自助接口不能携带它，成员不能声明自己设备的地区。它不是观测：Server 不按节点的公网地址或 GeoIP 推断、也不校验它，节点不在本机判断地区。

## 生命周期

注册事务创建默认 `nodes[id]` 配置项、写入批准的 identity／地址并生成配置版本。JSON 中新增一个 key 不会创建节点凭据。

**删除是永久删除，不提供节点恢复。** 显式删除接口、deleteNode 操作或最终 JSONC 目标移除已有 nodes[id]，均在同一事务吊销凭据、移除配置项、记录旧地址退出并撤回相关执行许可。应用前对照列出永久删除项。

最小吊销和历史关联保留以拒绝旧凭据，并非回收站。重新接入使用新 Node ID／密钥。引用删除节点的规则明确不可执行，不能自动绕过必经点；管理员可再清理这些规则。

### Node 自助注销 {#node-self-leave}

`meshway logout` 由 Node 以自己当前的 session 调用 `leaveNode`（`POST /agent-api/v1/leave`，请求体 `requestId`、`nodeId`）。这是同一永久删除，经同一配置提交入口执行：

| 项 | 行为 |
| --- | --- |
| 授权 | 与提交同一事务检查 Node session、凭据未吊销与身份资格；`nodeId` 必须是该 session 的 Node，否则 `403 permission_denied`。没有可指定其他 Node 的参数 |
| 效果 | 与管理员删除相同：吊销凭据、移除配置项与库存、记录旧地址退出、撤回 netmap、重新编译受影响节点；配置版本 cause 为 `nodeDeleted` |
| 版本作者 | 版本表要求 user 作者：User 身份设备记为其绑定用户，Tag 设备记为接入它的用户（批准者或接入密钥签发者）。配置变更通知的 `actor` 为 `node`（该 Node ID） |
| 之后 | 该 Node 的 session 随凭据结束，再调用得 `401`；其控制密钥重新认证得 `403 node_identity_rejected`。重新加入需要新的接入 |
| 拒绝 | 配置无法删除该 Node（例如有规则引用它）时为 `422 configuration_invalid`，与管理员删除相同；`403 permission_denied` 同属明确拒绝 |

注销是持久意图，归 Control 的本机身份记录（`control.json` 的 `leave`，与 `daemon.json.desiredUp` 同样跨重启保留）：发送前先保存（带 requestId），之后只有下列结果结束它：

| Server 的回答 | 本机 |
| --- | --- |
| 成功响应（`network`／`node` 与本机绑定一致） | 清除身份记录、`netmap.json` 与 `peer-facts.json`（见[私有存储](/v2/node/control#私有存储与传输)），下线 |
| 对该身份的拒绝：旧 session 得 `401` 后，一次新的持钥认证得 `403 node_identity_rejected`；或直接得到该拒绝 | 视为注销已完成（先前请求已删除、响应丢失，或管理员已删除），同样清除 |
| 明确拒绝（`403 permission_denied`、`422`） | 撤回意图，身份与运行不变 |
| 没有回答（网络不可达、无可解码 body 的状态、`409`／`5xx`） | 保留意图，状态 `leaving`，按 1–15 秒退避重试，重启后继续；期间不同步、不取得 instance、数据面停止 |

意图存在期间 `up`、`up --reenroll`、`rotate-key` 返回 `ErrorCode::Leaving`；`logout` 继续同一意图（沿用已保存的 requestId）；`down` 只改变期望上线，不取消注销。只有带新接入密钥的 `up --enrollment-key` 覆盖意图：本机放弃旧身份（Server 未回答时其记录留待管理员删除）并以新密钥接入。

## 路由方案选择

节点使用哪个出口方案是使用状态，不是 `nodes[id]` 字段：管理员以 `routing.egress.assignments` 给出 allowed／default／locked，选择由节点（`meshway exit`）或用户（`setNodeRoutingSelection`）提交，Server 持有。库存的 `routing` 字段给出列表摘要，读取与切换见 [Routing · 选择的入口](/v2/configuration-reference/routing#选择的入口)。

Node 使用本地持久密钥身份认证。接入批准、公钥与实体绑定走[注册流程](/v2/api/authentication#node-的三层凭据)，私钥不进入这份静态配置。

Tag 身份不继承 User／Group 权限，原创建用户离网后仍保留；最后一个 Tag 不能直接清空，须经设备用户登录完成身份转换。详见[有效身份](/v2/configuration-reference/identity#有效授权身份)。默认初始化不写空 dialers，以采用网络默认出站集合。

配置历史仍可读取；恢复目标中包含已删除 Node 时失败，不能复活凭据。恢复的最终目标若删除当前其他 Node，同样按永久删除语义在应用对照中展示；用户成员关系和身份切换证明仍按当前条件检查。
