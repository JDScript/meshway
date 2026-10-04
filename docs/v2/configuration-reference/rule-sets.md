# Rule Sets

Server 管理和更新的 IP／域名规则集。节点接收编译结果，不自行下载规则集。

## Structure

以下为 ruleSets 字段值。remote URL 是示例，提交时必须能取得符合格式的完整数据：

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
| `ipCIDR` | inline | 网段列表，默认 `[]` |
| `domainSuffix` | inline | DNS 标签后缀，默认 `[]`；两项不能都为空 |
| `url` | remote | HTTPS 数据来源，不含明文认证信息 |
| `format` | remote | 支持 `sing-box-source`、`mihomo-domain`、`mihomo-ipcidr` 的可转换子集，范围见下文 |
| `updateInterval` | remote | 默认 `24h`，保存正 duration 原值；实际调度下界、预算与失败退避见下文 |

inline 不接受 remote 字段，反之亦然。未知 key 引用、循环依赖和无法精确转换的条目必须报告，不能悄悄丢弃。

## 数据与状态

- Config 保存定义、来源和更新策略。
- 下载／编译产物保存为有摘要的不可变资源，由执行发布引用。
- 持久保存已验证、可执行的规范化内容及其格式；原始下载作为有界临时输入。每次更新报告无法精确转换的条目，不丢弃错误后声称转换完整。
- State 报告最后成功更新时间、摘要、条目数和错误。
- 更新失败保留最后一份已校验结果并明确错误，不发布半份内容。首次没有可用结果时相关发布不可执行。
- 回滚使用历史发布绑定的资源版本，不能只保存 URL。历史内容不可用时明确失败并要求替代；不自动使用最新数据冒充复原。保留期限在接口／运维规则中给出。

转换范围限 IP 段和域名后缀，不支持 sing-box／mihomo 的所有规则。

## 配置与刷新

Owner／Admin 可在一次配置 apply 中新增 remote 声明及引用它的路由规则，也可直接替换正在引用的来源。Server 先验证用户／网络与输入，在 SQLite 事务外进行有界下载和完整转换，最后同事务重验身份、源 revision、原 artifact 及完整配置权限，固定 artifact／源版本初始绑定并发布。失败保留原配置与已验证内容，不把缺失集合当成空集合或跳到后续 direct 规则。

validate 走相同下载／转换与授权校验，但不保存 artifact、不预留配置版本。`ConfigurationPreviewResponse.preparedRuleSets` 按 key 返回暂定规范化内容的 `digest`／`entries`，不使用假 ArtifactId；缺失 listener 材料等仍返回类型化准备需求。最终 apply 独立重新准备／重验，不能将一次 preview 当作免 CAS 的承诺。单次配置最多准备 32 项远端集合、规范化内容合计不超过 8 MiB，下载准备总期限 30 秒；进程内最多 4 项配置准备，包括等待最终事务的输入。每张网络实际绑定的规则集执行内容合计最多 8 MiB／100000 条目，按 key 计入，不能通过多次提交绕过执行内存预算。

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

## 周期更新与生命周期

Server 启动周期调度器，来源为当前网络配置、持久检查记录与确切 artifact，使用内部任务资格，不持有管理员会话。网络按轮转顺序检查，网内优先处理最久未检查的到期集合。调度下界为 1 秒；每轮检查间隔至少 100 ms，每个实例同时执行一项周期下载，所有配置／手动／周期调用共用进程内 4 项实际下载预算。同一 URL／格式的并发下载合并，最多同时接纳 8 项刷新工作；负载、轮转和下载耗时会推迟实际检查，不保证逐毫秒执行。

正常期限从 `checkedAt` 加 `max(updateInterval, 1s)` 计算。失败按连续次数从 5 秒指数退避、上限 5 分钟，次数和时间持久保存，重启不清除退避。只有调整间隔且来源／格式相同时，沿用可信内容与检查记录。历史恢复已有可信 artifact 但新版本没有检查记录时，以该配置提交时间作为首个周期起点，不伪造 State 检查时间；没有 artifact 则立即准备。时间运算饱和处理超大 duration。

规范化内容不变时只更新检查时间／结果，不新建 artifact 或执行 publication；内容变化才进入同 revision 的刷新发布。最终事务还比较原检查截点，旧下载结果不能覆盖较新的刷新或配置提交。App shutdown／drop 取消网络工作，正常 shutdown join 调度器后释放存储；已接纳 SQLite 事务仍完成提交与事件发布。检查更新不会延长 Node lease 或制造连接／探测历史样本。

资源发布不证明 Node 已应用配置或实际访问出口；使用实际回执与业务观测判断运行结果。
