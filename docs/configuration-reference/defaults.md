# Defaults

定义未命中规则时的行为，以及节点未覆盖时使用的出站承载集合。以下是下一版格式的默认值提案。

## Structure

```json
{
  "access": "deny",
  "topology": "allow",
  "dialers": {
    "native_udp": { "priority": 100 },
    "native_tcp": { "priority": 200 },
    "vless": { "priority": 300 }
  }
}
```

## Fields

| 字段 | 类型 | 默认 | 说明 |
| --- | --- | --- | --- |
| `access` | `"deny" \| "allow"` | `"deny"` | **Mesh 节点互访**的兜底；外部目的地仍须显式授权 |
| `topology` | `"deny" \| "allow"` | `"allow"` | 没有匹配组网规则时是否允许尝试建立承载 |
| `dialers` | `DialerSet` | 上例 | 以协议名为 key；节点 `dialers: {}` 时采用此集合 |

`access: "allow"` 的范围限 Mesh，是本轮建议的边界，尚待确认；不将“默认允许”隐式解释成开放所有公网／LAN 目的地。当前发布契约尚没有本节 defaults。

## access

- `deny`：只有显式 access 匹配的业务被允许；空 access 拒绝 Mesh 新会话。
- `allow`：等价于额外的一条 Mesh `src: ["*"]`、`dst: ["*"]`、`ip: ["*"]` 允许规则。
- access 只包含允许规则。因此 allow 下增加更窄的 access 规则不能收紧权限；需要收紧时改为 deny 并显式列出允许范围。
- 公网、LAN、域名目的授权仍由显式 access 表达；出口资格和可执行路径也仍须成立。

## topology

判断顺序固定为：匹配任意 block → 拒绝；否则匹配任意 allow → 允许；否则使用 defaults.topology。规则顺序不参与判断。

`topology: []` 配合 allow 适合自动组网；配合 deny 则必须显式列出允许边。它不授予业务访问，也不会预先建立全网连接。

## dialers

节点非空 `dialers` 替换这份默认集合；省略 defaults.dialers 使用平台基线，显式 `{}` 表示默认没有出站拨号器。具体例子和字段默认值顺序统一见[继承规则](/configuration-reference/inheritance)。

listener 不会因 defaults 自动打开监听端口。出口兜底保留在 routing profile 的 `fallback` 中，避免与全网 defaults 出现两处竞争的路径决定。
