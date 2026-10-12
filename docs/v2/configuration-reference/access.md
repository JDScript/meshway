# Access · 访问权限

定义谁可以访问什么。规则取并集、顺序无关，只表达允许；未匹配行为见 [Defaults](/v2/configuration-reference/defaults#access)。

## Structure

配置节选，引用的用户组与标签须在当前网络有效：

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
| `src` | `string[]` | 必填 | 非空源[选择器](/v2/configuration-reference/identity#selectors)集合 |
| `dst` | `string[]` | 必填 | 非空目标集合 |
| `ip` | `string[]` | 必填 | 非空协议／目的端口集合 |
| `bidirectional` | `boolean` | `false` | 同时允许反向发起新会话；只适用于双方均为 Mesh 节点的规则 |

错误定位到固定 revision 下的数组位置，例如 `/access/2/ip`；remark 变化不改变授权。

### ip

| 示例 | 含义 |
| --- | --- |
| `*` | 本模型支持的全部业务协议／端口 |
| `tcp:443` | TCP 目的端口 443 |
| `tcp:8000-8100` | TCP 目的端口范围 |
| `udp:*` | UDP 任意目的端口 |
| `icmp:*` | ICMP 授权；执行需求使用明确的 ICMP echo 类型 |

### 外部目的地

`dst` 也支持 IPv4／IPv6 CIDR 和 `domain:example.com`（域名后缀，按 DNS 标签边界匹配）。执行仍校验目标范围与节点能力。

`autogroup:internet` 排除私网、ULA、CGNAT、链路本地等非公网范围。LAN 必须显式授权 CIDR。域名授权依赖可信 DNS／域名关联，不凭 IP 包猜测域名；因为域名按解析地址集合授权，持有 `autogroup:internet` 的源也可以把域名目的地（路由规则或规则集中的名称）交给出口，执行时仍逐个校验解析地址在公网范围内。

## Behavior

- `defaults.access: "deny"`；新网络的 Mesh 互访许可由显式模板规则表达。`access: []` 不授予经 Mesh 的新业务访问。
- 单向授权允许已建立会话的状态型回复，不自动允许反向新连接。
- grant 允许访问，不代表已有路由、listener、认证通道或可用出口。
- 出口／路由方案必须处于 access 允许范围。路径与候选出口引用的联合校验见[流量路由](/v2/configuration-reference/routing)。

## 保护范围 {#scope}

access 保护 Mesh 目标和 Mesh 出口转发。源端 direct／unmanaged 使用自身本地网络，不额外套用 Mesh access；本规则不声称能阻止设备所有者自行出网。

access 约束业务目的地，不再重复定义一套“允许出口列表”。允许的方案和 routing 决定可用出口，与 exitEnabled、topology 联合生成接收端执行许可。访问出口节点上的服务，不等于允许通过它转发。

## 域名授权 {#domains}

域名按**可信 DNS 解析所得地址集合**授权。域名许可不是严格的目标服务身份：两个域名共享 IP 时，不能保证只访问其中一个域名。同理，`autogroup:internet` 或覆盖解析地址的 CIDR 授权已经包含该域名的流量；配置校验据此接受 Internet 授权下的域名出口规则，而 CIDR 授权在声明期无法判定解析结果，仍要求 `domain:` 授权。

域名仍可为正常客户端分流提供上下文，以遵守有序规则。需要强制覆盖某实际地址上所有流量的路径约束，应按 IP／CIDR 定义；不能靠源端声明一个域名来证明同 IP 上实际访问的是哪个应用。

执行关联由 Server 校验出口 resolver 的受控解析结果后发布，绑定网络、源与出口的 netmap、源的出口 selection、名称／CNAME、公网地址和 TTL；`domain:` 授权按这些关联的名称执行，见 [Routing · 域名规则的执行](/v2/configuration-reference/routing#域名规则的执行)。普通 State 不能注入新的 DNS 授权关联或增加出口接受权限。

## Enforcement

发送端的过滤可以提前拒绝无权业务，但不是唯一安全边界。**接收端必须依据自己从 Server 获取的配置，验证已认证源节点、内层地址、完整路径及访问许可，然后才把明文交给目标系统。** 它不信任发送端声明的 User／Tag、权限或“本地配置已校验”。

只修改 A 的本地配置，不能让正常执行授权的 B 获得一条新的接收许可；伪造源 IP 也不能替代对应节点的密钥证明。中转场景验证业务端点身份与完整路径，而不是把最后一跳中继当成请求源。状态型回复须匹配已建立流，不能伪装成回复发起新流。

本边界以接收端执行可信配置为前提。控制目标主机的人可以修改本机服务或另建代理；已获授权的主体也可以转交其取得的数据，这不属于仅靠 Mesh 端点授权能够阻止的行为。撤销按目标收到更新或其旧 lease 到期生效，发送端不能自行延长目标的许可。
