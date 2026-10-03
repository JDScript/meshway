# Grants

业务访问授权。规则取并集、顺序无关，只表达允许；未匹配行为见 [Defaults](/configuration-reference/defaults#grants)。本页为下一版格式。

## Structure

```json
{
  "grants": [
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

不使用规则 ID。错误定位到固定 revision 下的数组位置，例如 `/grants/2/ip`；remark 变化不改变授权。现行 API 尚未接受本页的完整新格式。

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

- 默认 `defaults.grants: "deny"` 时，`grants: []` 不授予业务访问；默认允许 Mesh 的含义由 defaults 明确表达。
- 单向授权允许已建立会话的状态型回复，不自动允许反向新连接。
- grant 允许访问，不代表已有路由、listener、认证通道或可用出口。
- 出口／路由方案必须处于 grants 允许范围。路径与候选出口引用的联合校验见[路由](/configuration-reference/routing)。
