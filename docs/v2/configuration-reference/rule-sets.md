# Rule Sets

Controller 管理和更新的 IP／域名规则集草案。节点接收编译结果，不自行下载规则集。

## Structure

```json
{
  "office": {
    "type": "inline",
    "ipCIDR": ["10.20.0.0/16"],
    "domainSuffix": ["corp.example"]
  },
  "regional": {
    "type": "remote",
    "url": "https://rules.example.com/regional.json",
    "format": "sing-box-source",
    "updateInterval": "24h"
  }
}
```

## Fields

| 字段 | 适用类型 | 说明 |
| --- | --- | --- |
| `type` | 所有 | `inline` 或 `remote` |
| `ipCIDR` | inline | 网段列表，建议默认 `[]` |
| `domainSuffix` | inline | DNS 标签后缀，建议默认 `[]`；两项不能都为空 |
| `url` | remote | HTTPS 数据来源，不含明文认证信息 |
| `format` | remote | 提议支持 `sing-box-source`、`mihomo-domain`、`mihomo-ipcidr` 的可转换子集 |
| `updateInterval` | remote | 建议默认 `24h`，具体范围待定 |

inline 不接受 remote 字段，反之亦然。未知 key 引用、循环依赖和无法精确转换的条目必须报告，不能悄悄丢弃。

## 数据与状态

- Config 保存定义、来源和更新策略。
- 下载／编译产物保存为有摘要的不可变资源，由执行发布引用。
- State 报告最后成功更新时间、摘要、条目数和错误。
- 更新失败不得发布半份内容；建议保留最后一份已校验结果，并明确状态。首次没有可用结果时相关发布不可执行。
- 回滚使用历史发布绑定的资源版本，不能只保存 URL。历史内容不可用时明确失败并要求替代；不自动使用最新数据冒充复原。保留期限在接口／运维规则中给出。

本稿范围限 IP 段和域名后缀，不因为格式名相同就声明支持 sing-box／mihomo 的所有规则。
