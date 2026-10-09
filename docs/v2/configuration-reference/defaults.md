# Defaults

定义未命中规则时的行为，以及节点未覆盖时使用的出站承载集合。access 默认拒绝；新网络生成显式 Mesh 互访模板。

## Structure

<!-- schema: Defaults -->
```json
{
  "access": "deny",
  "topology": "allow",
  "dialers": {
    "native_udp": {},
    "native_tcp": {},
    "vless": {}
  }
}
```

## Fields

| 字段 | 类型 | 默认 | 说明 |
| --- | --- | --- | --- |
| `access` | `"deny"` | `"deny"` | 未获显式 access 的经 Mesh 业务拒绝；全允许用规则表达 |
| `topology` | `"deny" \| "allow"` | `"allow"` | 没有匹配组网规则时是否允许尝试建立承载 |
| `dialers` | `DialerSet` | 上例 | 以协议名为 key；节点未填写出站覆盖时采用此集合 |

## access

新网络预置以下可编辑规则（配置节选）；删除它后，只保留其他显式规则授予的权限：

```json
{
  "access": [
    { "remark": "默认允许 Mesh 成员互访", "src": ["*"], "dst": ["*"], "ip": ["*"] }
  ]
}
```

`*` 只指本网络节点，模板不开放公网／LAN 转发。access 取并集，仍有此宽规则时增加一个窄规则不会收紧其他访问；需要限制时删除／修改宽规则。空 access 拒绝经 Mesh 的新业务，本机 direct／unmanaged 范围见[访问权限](/v2/configuration-reference/access#scope)。

## topology

允许／阻断判断顺序固定为：匹配任意 block → 拒绝；否则匹配任意 allow → 允许；否则使用 defaults.topology。边级承载集合由 [Topology 首条匹配 allow](/v2/configuration-reference/topology#matching) 决定；默认允许没有边级承载限制。

`topology: []` 配合 allow 适合自动组网；配合 deny 则必须显式列出允许边。它不授予业务访问，也不会预先建立全网连接。

## dialers

节点省略 `dialers` 才继承默认集合；显式 `{}` 禁用主动建连，非空集合替换默认集合。省略 defaults.dialers 使用该配置版本固定的平台基线集合：`native_udp`、`native_tcp`、`vless`；显式 `{}` 表示网络默认不主动拨号。见[继承规则](/v2/configuration-reference/inheritance)。

dialers 只声明主动拨号能力与协议选项，承载偏好由 topology 定义；完整候选排序规则见 [Topology 承载交集](/v2/configuration-reference/topology#承载交集)。

listener 不会因 defaults 自动打开监听端口。出口兜底保留在 routing profile 的 `fallback` 中，避免与全网 defaults 出现两处竞争的路径决定。
