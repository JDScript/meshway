# Capabilities

capability 描述当前 Node 实例的实现能力，不能代替配置许可或资源状态。key 使用 `transport:`、`feature:` 等命名空间，value 是封闭的 Capability 类型。

## Structure

下面是单个 VLESS capability 值：

<!-- schema: Capability -->
```json
{
  "kind": "transport",
  "roles": ["dial", "listen"],
  "streams": ["tcp", "websocket"]
}
```

在 capabilities map 中以 `transport:vless` 为 key。Native UDP／TCP 分别使用 `transport:native_udp`／`transport:native_tcp`。

| kind | 必填字段 | 含义 |
| --- | --- | --- |
| transport | roles、streams | 实际支持的方向与封装；roles 使用 dial／listen，streams 使用 tcp／websocket／udp |
| feature | version | 该功能的协议版本，例如 feature:path-observation version 1 |

roles 和 streams 明确填写；没有“省略 roles 自动双向”的运行合同。一个方向的构建只声明实际支持方向，未声明的 key 不作为已支持能力。

## 能力、配置与状态

| 位置 | 含义 |
| --- | --- |
| capabilities | 实现支持哪些功能 |
| Config listeners／topology／routing | 允许及期望使用哪些功能 |
| State listeners／sockets／links | 实际资源和运行结果 |

端口占用、权限不足属于资源错误，不伪称安装成功；没有配置 listener 不等于没有 listen 实现。标准包必带 UDP/TCP 不允许绕过网络的 VLESS-only 限制。

能力通过持钥认证后的实例 acquire 声明，普通 State 上报不能覆盖已编译执行能力。实例替换后重新协商，不继承旧证明。Node 插件装载与版本关系见[插件](/v2/node/plugins)；未知插件协议仍须平台 schema／编译支持。
