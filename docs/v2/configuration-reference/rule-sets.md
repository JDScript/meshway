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
- 初版持久保存已验证、可执行的规范化内容及其格式；原始下载作为有界临时输入。每次更新仍报告无法精确转换的条目，不丢弃错误后声称转换完整。
- State 报告最后成功更新时间、摘要、条目数和错误。
- 更新失败不得发布半份内容；建议保留最后一份已校验结果，并明确状态。首次没有可用结果时相关发布不可执行。
- 回滚使用历史发布绑定的资源版本，不能只保存 URL。历史内容不可用时明确失败并要求替代；不自动使用最新数据冒充复原。保留期限在接口／运维规则中给出。

本稿范围限 IP 段和域名后缀，不因为格式名相同就声明支持 sing-box／mihomo 的所有规则。

## 新 Controller 候选用法

Owner／Admin 通过正常配置 API 保存未引用的 remote 声明，调用 refresh 取得经过完整转换的内容，再提交引用该集合的路由规则。尚未准备的引用返回 `ruleSetContent` 准备需求（422），声明本身不阻止普通 Mesh 配置保存。替换已使用的来源时，可先移除引用、修改声明并刷新，再应用新引用；不会把缺失集合当成空集合或跳到后续 direct 规则。

接口相对 `/api/v1/networks/{networkId}`：

| 方法与路径 | 返回 |
| --- | --- |
| `GET /rule-sets` | `RuleSetList`，当前网络定义与检查结果 |
| `GET /rule-sets/{ruleSetKey}` | `RuleSetList`，仅指定集合 |
| `POST /rule-sets/{ruleSetKey}/refresh` | `RuleSetView`，下载、完整转换、权限重验与执行发布后的结果 |

这些完整资源管理视图限本网络 Owner／Admin。普通成员与 Node 不取得整网来源／artifact 清单；Node 只消费已授权的执行计划。URL 不得含 userinfo、fragment；下载保留 HTTPS 证书／主机名校验，拒绝重定向及非公网解析结果，并固定已验证地址。

`RuleSetView` 包括 `configurationRevision`、`definition`、`artifact`（不可变 ID／摘要／创建时间／条目数）、`publication`（选中该 artifact 的确切发布）、`problem`、`checkedAt`、`lastSuccessAt`。检查时间与 artifact 创建时间不同；失败更新检查时间和错误，保留最后成功 artifact 与成功检查时间。没有对应检查记录的时间为 null，不伪造新鲜度。相同绑定随普通配置版本延续时保留原检查时间。

转换支持 sing-box source v1–v3 的单类别 `ip_cidr`／`domain_suffix` 规则，以及 mihomo 的 IP CIDR payload／`+.domain` 后缀 payload。未知字段、逻辑组合、精确域名或无法保留语义的混合条件会使整次转换失败。错误为 `download_failed`、`invalid_source`、`too_large`、`conversion_failed` 或 `validation_failed`；首次没有内容为 `not_ready`。

刷新在同一配置 revision 下创建 `cause=refresh` 的执行发布，合并所有有效需求，保留未变化 Node 的目标／lease／回执；不覆盖 JSONC、历史源绑定或 artifact。变更通过 `resources.changed` 通知有权管理该网络的 SSE 客户端，客户端重新读取资源 GET。初始同步或 stream reset 后也重新读取 GET。

当前候选提供显式 refresh；`updateInterval` 已保存，周期自动下载调度尚未接入。此闭环验证到真实 Controller 发布和 Node target，不代表 Agent／TUN 已实际访问出口。
