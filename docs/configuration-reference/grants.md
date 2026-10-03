# Grants

业务访问授权草案。规则取并集、顺序无关，只表达允许。

## Structure

```json
{
  "grants": [
    {
      "id": "dev-web",
      "src": ["group:dev"],
      "dst": ["tag:backend"],
      "ip": ["tcp:443", "tcp:8443"],
      "bidirectional": false
    }
  ]
}
```

## Fields

| 字段 | 类型 | 建议默认 | 说明 |
| --- | --- | --- | --- |
| `id` | `string` | 必填 | 唯一规则 key，用于诊断、引用和增量 |
| `src` | `string[]` | 必填 | 非空源[选择器](/configuration-reference/identity#selectors)集合 |
| `dst` | `string[]` | 必填 | 非空目标集合 |
| `ip` | `string[]` | 必填 | 非空协议／目的端口集合 |
| `bidirectional` | `boolean` | `false` | 同时允许反向发起新会话；只适用于双方均为 Mesh 节点的规则 |

`id`、`ip` 的确切格式属于本稿提案；现行 grants 不支持本页全部字段组合。

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

- `grants: []` 不授予业务访问；新网络的“成员全部允许”通过显式模板规则实现。
- 单向授权允许已建立会话的状态型回复，不自动允许反向新连接。
- grant 允许访问，不代表已有路由、listener、认证通道或可用出口。
- 出口／路由方案必须处于 grants 允许范围。路径与候选出口引用的联合校验见[路由](/configuration-reference/routing)。
