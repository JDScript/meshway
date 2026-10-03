# 访问授权（Access）

Access 是唯一 `NetworkPolicy` 中的 `grants` 层，决定谁可以访问什么；连接与路由决定如何承载与经过哪些节点。路由不能授予访问，也不能跳过必经节点或降低 E2E。

- 只允许、未匹配拒绝；grants 取并集，与顺序和具体度无关。
- 新网络默认包含成员 `* → *` grant；显式空 grants 为拒绝全部，不自动恢复模板。
- 当前 selector 为本网 `*`、`node:<完整身份>`、管理员分配的 `tag:<名称>`。
- `bidirectional` 显式允许反向发起业务。TCP、UDP 与 ICMP echo 的状态型回复只需原方向 grant，沿原会话完整路径反向返回；反向新会话仍须独立授权并匹配其路由规则。
- 未知或已撤销明确引用会使源失效；保留源与诊断，完整节点执行失败关闭。缺能力或公钥时不能删除必经节点再选路。

源通过 GET `/api/v1/networks/{network_id}/policy` 读取，POST `.../policy/save` 保存。请求使用当前 `expected_revision` 和完整四层模型；`source_jsonc` 可省略/null，提供文本时必须与结构化 policy 相同。支持注释、尾逗号，拒绝重复对象键。只改注释也推进 revision；无变化保存不推进。角色权限与 CAS 在根事务内复核，失败保留草稿。

标签由管理 API 维护，Agent 自报不授予标签。仍被节点或 policy 引用的标签不能删除。图形化与 JSONC 编辑器应共享一份逐网络草稿和 CAS 基准，切换不能覆盖无效文本、注释或未保存修改；当前 console UI 由独立工作流接入这些合同。

公开模型见[四层策略](/policy)，实际节点投影、预算与当前 IPv4 Mesh 限制见[执行合同](/mesh-execution)。HTTP 不再提供独立 Access 源或旧 Flow 预览。编译诊断和逻辑路径均不等于实际 trace 或成功访问。
