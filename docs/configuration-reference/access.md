# Access · 访问权限

定义谁可以访问什么。规则取并集、顺序无关，只表达允许；未匹配行为见 [Defaults](/configuration-reference/defaults#access)。本页为下一版格式。

## Structure

```json
{
  "access": [
    {
      "remark": "开发组访问后端 Web 服务",
      "src": ["group:dev"],
      "dst": ["tag:backend"],
      "ip": ["tcp:443", "tcp:8443"],
      "bidirectional": false
    }
  ]
}
```

## Fields

| 字段 | 类型 | 默认 | 说明 |
| --- | --- | --- | --- |
| `remark` | `string` | `""` | 可重复的说明文本；不接受 null |
| `src` | `string[]` | 必填 | 非空源[选择器](/configuration-reference/identity#selectors)集合 |
| `dst` | `string[]` | 必填 | 非空目标集合 |
| `ip` | `string[]` | 必填 | 非空协议／目的端口集合 |
| `bidirectional` | `boolean` | `false` | 同时允许反向发起新会话；只适用于双方均为 Mesh 节点的规则 |

不使用规则 ID。错误定位到固定 revision 下的数组位置，例如 `/access/2/ip`；remark 变化不改变授权。现行 API 尚未接受本页的完整新格式。

### ip

| 示例 | 含义 |
| --- | --- |
| `*` | 本模型支持的全部业务协议／端口 |
| `tcp:443` | TCP 目的端口 443 |
| `tcp:8000-8100` | TCP 目的端口范围 |
| `udp:*` | UDP 任意目的端口 |
| `icmp:*` | 允许支持的 ICMP 流量；首片执行范围仍须明确 |

### 外部目的地

建议 `dst` 另外支持 IPv4／IPv6 CIDR 和 `domain:example.com`（域名后缀，按 DNS 标签边界匹配）。具体支持范围随执行能力校验，不能保存后忽略。

`autogroup:internet` 排除私网、ULA、CGNAT、链路本地等非公网范围。LAN 必须显式授权 CIDR。域名授权依赖可信 DNS／域名关联，不凭 IP 包猜测域名。

## Behavior

- 默认 `defaults.access: "deny"` 时，`access: []` 不授予业务访问；默认允许 Mesh 的含义由 defaults 明确表达。
- 单向授权允许已建立会话的状态型回复，不自动允许反向新连接。
- grant 允许访问，不代表已有路由、listener、认证通道或可用出口。
- 出口／路由方案必须处于 access 允许范围。路径与候选出口引用的联合校验见[流量路由](/configuration-reference/routing)。

## Enforcement

发送端的过滤可以提前拒绝无权业务，但不是唯一安全边界。**接收端依据自己从 Controller 获取的配置，验证已认证源节点、内层地址、完整路径及访问许可，然后才把明文交给目标系统。** 它不信任发送端声明的 User／Tag、权限或“本地配置已校验”。

只修改 A 的本地配置，不能让正常执行授权的 B 获得一条新的接收许可；伪造源 IP 也不能替代对应节点的密钥证明。中转场景验证业务端点身份与完整路径，而不是把最后一跳中继当成请求源。状态型回复须匹配已建立流，不能伪装成回复发起新流。

本边界以接收端执行可信配置为前提。控制目标主机的人可以修改本机服务或另建代理；已获授权的主体也可以转交其取得的数据，这不属于仅靠 Mesh 端点授权能够阻止的行为。撤销按目标收到更新或其旧 lease 到期生效，发送端不能自行延长目标的许可。
