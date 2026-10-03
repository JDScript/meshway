# Capabilities

节点实际构建支持的能力集合。key 使用命名空间，便于 transport 之外的能力继续扩展；以下是下一版 State 格式。

## Structure

```json
{
  "transport:native_udp": {},
  "transport:vless": {
    "streams": ["tcp", "websocket"]
  },
  "feature:ipv4-egress": {}
}
```

key 存在表示节点声明支持这项能力；省略表示未声明支持。每项 value 由该能力的明确 Schema 定义，不是任意 JSON 的执行入口。

## Transport roles

通常一份 transport 实现同时提供 dial 和 listen。建议在这种情况下省略重复的布尔值，`roles` 默认 `["dial", "listen"]`。

如果某个构建只实现客户端，必须明确报告：

```json
{
  "transport:vless": {
    "roles": ["dial"],
    "streams": ["tcp"]
  }
}
```

| 字段 | 类型 | 建议默认 | 含义 |
| --- | --- | --- | --- |
| `roles` | `("dial" \| "listen")[]` | `["dial", "listen"]` | 实现提供的方向；非空且不重复 |
| `streams` | 协议定义的 enum 数组 | 协议描述决定 | 例如 VLESS 实际支持的 TCP／WS 子能力 |

完整实现才能使用“省略 roles 即双向”的简写，不能把当前 client-only 实现直接写成 `{}`。该默认值是待确认的表达方式，不改变运行时校验。

## 三个不同问题

| 位置 | 回答的问题 |
| --- | --- |
| `capabilities['transport:vless']` | 这个构建是否实现 VLESS，支持哪些角色／封装？ |
| Config listeners／topology | 管理员是否配置入口、允许这项建连？ |
| State listeners／links | 本机是否成功 bind、是否已有认证通道？ |

没有配置 listener 不等于没有 listen 能力；端口被占用或系统权限不足应报告资源失败，不把编译能力伪装成已执行成功。

`feature:ipv4-egress` 也只表示实现能力，不代替 Config 的出口开关、访问授权、NAT 准备与执行状态。不用能力名称重新引入“服务器／终端”节点分类。

## 扩展规则

- transport 使用 `transport:<协议>`，其他能力可以按明确的命名空间登记，例如 `feature:ipv4-egress`。
- 每项定义自己的参数、取值和协商规则；只使用已识别且相容的能力参与执行选择。
- 能力绑定当前 Node 实例；实例更替／报告过期时不能沿用为当前可用证明。
- 前缀只用于 capability key。配置中的 `type: 'vless'`、dialers 协议 key 和 LinkState.transport 不随之加前缀。
