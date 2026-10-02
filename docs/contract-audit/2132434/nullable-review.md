# 全部 nullable 字段的语义与处置

冻结 HTTP 契约 `2132434`，57 项全覆盖，与导出字段集合精确核对。处置为审查建议，尚未更改契约。

| 字段 | required | null 的含义 | 建议 |
| --- | --- | --- | --- |
| `AccountInvitation.consumed_at` | yes | 未消费；二者须配对 | 保留输出；内部可合为 consumption |
| `AccountInvitation.consumed_by` | yes | 未消费；二者须配对 | 保留输出；内部可合为 consumption |
| `AccountInvitation.issuer_user_id` | yes | 实例凭据发行时没有用户 issuer | 保留，不编造用户 ID |
| `AccountInvitation.revoked_at` | yes | 没有撤销事件 | 保留，不填零时间 |
| `AccountPasswordReset.consumed_at` | yes | 尚未兑换 | 保留，区别于 revoked 和 credential_current |
| `AccountPasswordReset.revoked_at` | yes | 没有撤销事件 | 保留，不填零时间 |
| `ClaimOwnerRequest.email` | no | 分支不适用或字段缺失合为 None | 规范输入拆分支；兼容旧显式 null |
| `ClaimOwnerRequest.password` | no | 分支不适用或字段缺失合为 None | 规范输入拆分支；兼容旧显式 null |
| `ClaimOwnerRequest.session_token` | no | 分支不适用或字段缺失合为 None | 规范输入拆分支；兼容旧显式 null |
| `ConnectionCandidate.credential_ref` | yes | native 不需要代理 credential ref | 按 transport 限制；不是节点身份或明文密钥 |
| `ConnectionDiagnostic.item_id` | yes | 全局或非单项诊断 | 保留，不伪造规则 ID |
| `ConnectionListener.credential_ref` | yes | native 不需要代理 credential ref | 按 transport 限制；不是节点身份或明文密钥 |
| `ConnectionNeighborObservation.candidate_id` | yes | 入站侧可能没有精确 candidate 关联 | 保留，不能和 listener 一起非空化 |
| `ConnectionNeighborObservation.listener_id` | yes | 类型可空，但当前上报 validator 要求存在 | 收紧当前报告输入；后续不同 socket 来源另建明确分支 |
| `ConnectionNeighborObservation.probe_rtt_ms` | yes | 无可报告的 RTT | 保留缺测；不填 0 |
| `ConnectionNode.acknowledged_at` | yes | 尚无连接回执；revision 同时为 0 | 保留旧输出；内部可 Option&lt;Receipt&gt; |
| `ConnectionNode.acknowledged_digest` | yes | 尚无连接回执；revision 同时为 0 | 保留旧输出；内部可 Option&lt;Receipt&gt; |
| `ConnectionNode.public_key` | yes | 没有 connection identity 记录 | 保留；不是 SQL 公钥列为空 |
| `ConnectionRuntimeObservation.digest` | yes | 暂无报告或撤销后隐藏报告 | 保留；不得用当前配置补旧报告 |
| `ConnectionRuntimeObservation.reported_at` | yes | 暂无报告或撤销后隐藏报告 | 保留；不得用当前配置补旧报告 |
| `ConnectionRuntimeObservation.revision` | yes | 暂无报告或撤销后隐藏报告 | 保留；不得用当前配置补旧报告 |
| `FlowAuthorizationDecision.matched_direction` | yes | 默认、无匹配或 invalid 没有具体规则/方向 | 按结果解释，不强造 ID |
| `FlowAuthorizationDecision.rule_id` | yes | 默认、无匹配或 invalid 没有具体规则/方向 | 按结果解释，不强造 ID |
| `FlowQuery.destination_country` | yes | 可组合的目标与上下文信息 | 保留 node+IP、domain+IP 等合法组合；不是简单互斥三选一 |
| `FlowQuery.destination_ip` | yes | 可组合的目标与上下文信息 | 保留 node+IP、domain+IP 等合法组合；不是简单互斥三选一 |
| `FlowQuery.destination_node_id` | yes | 可组合的目标与上下文信息 | 保留 node+IP、domain+IP 等合法组合；不是简单互斥三选一 |
| `FlowQuery.domain` | yes | 可组合的目标与上下文信息 | 保留 node+IP、domain+IP 等合法组合；不是简单互斥三选一 |
| `LatencyChannelAssociation.candidate_id` | yes | 入站侧可能没有精确 candidate 关联 | 保留，不能和 listener 一起非空化 |
| `LatencyHistoryResponse.capacity_trimmed_before` | yes | 没有容量淘汰水位 | 保留，不代表历史完整连续 |
| `LatencyHistoryResponse.next` | yes | 没有下一页或初始游标 | 保留；仅固定窗口内直接沿用游标 |
| `LatencySample.rtt_us` | yes | timeout/failed 没有 RTT；success 必须有 | 用 outcome 分支收紧关系，不填 0 |
| `ManagedListenerSettings.vless` | yes | 非 VLESS 不适用 | 规范配置按 transport 分支 |
| `ManagedListenerSummary.certificate_sha256` | yes | native 不需要托管 VLESS 证书 | 保留，不是证书未知 |
| `ManagedVlessMaterial.private_key_pem` | yes | 拨号端无权取得 listener 私钥 | 保留保密投影，不能给所有节点填值 |
| `ManagedVlessSettings.websocket_host` | yes | TCP 不适用；WebSocket 必须有 | 按 stream 分支收紧关系 |
| `ManagedVlessSettings.websocket_path` | yes | TCP 不适用；WebSocket 必须有 | 按 stream 分支收紧关系 |
| `MeshAccessSnapshot.source_jsonc` | yes | 没有保存原始 JSONC | 保留 |
| `MeshNode.country` | yes | 未设置或清除管理员 country | 旧字段保留；不是 STUN/GeoIP |
| `NodeConnections.public_key` | yes | 没有 connection identity 记录 | 保留；不是 SQL 公钥列为空 |
| `NodeEndpointDiscovery.digest` | yes | 尚无发现报告；失效后可保留历史值 | 保留，null 不等于 expired |
| `NodeEndpointDiscovery.reported_at` | yes | 尚无发现报告；失效后可保留历史值 | 保留，null 不等于 expired |
| `NodeEndpointDiscovery.revision` | yes | 尚无发现报告；失效后可保留历史值 | 保留，null 不等于 expired |
| `NodeOwnership.membership_id` | yes | 尚未绑定账户 membership | 保留；新 token 节点也可能为空，不等于 legacy |
| `NodeSummary.country` | yes | 未设置或清除管理员 country | 旧字段保留；不是 STUN/GeoIP |
| `NodeSummary.revoked_at` | yes | 没有撤销事件 | 保留，不填零时间 |
| `OutgoingConnectionCandidate.credential_ref` | yes | native 不需要代理 credential ref | 按 transport 限制；不是节点身份或明文密钥 |
| `PolicyDiagnostic.rule_id` | yes | 全局或非单项诊断 | 保留，不伪造规则 ID |
| `PolicySnapshot.source_jsonc` | yes | 没有保存原始 JSONC | 保留 |
| `QueryLatencyHistoryRequest.after` | yes | 没有下一页或初始游标 | 保留；仅固定窗口内直接沿用游标 |
| `RouteAction.exit_node_id` | yes | Mesh 路径或不使用专门出口时不适用 | 按 action/scope 解释，保留 |
| `RouteDecision.exit_node_id` | yes | Mesh 路径或不使用专门出口时不适用 | 按 action/scope 解释，保留 |
| `RouteDecision.rule_id` | yes | 默认动作或 invalid 没有具体匹配规则 | 保留 |
| `SaveMeshAccessRequest.source_jsonc` | no | 缺失/null 都清除 source | 明确全量替换；不得误当 preserve |
| `SavePolicyRequest.source_jsonc` | no | 缺失/null 都清除 source | 明确全量替换；不得误当 preserve |
| `SetNodeCountryRequest.country` | yes | 未设置或清除管理员 country | 旧字段保留；不是 STUN/GeoIP |
| `StunEndpointObservation.mapped_endpoint` | yes | 失败无映射/RTT；mapped 必须有 | 按 outcome 收紧，不填零或虚构 endpoint |
| `StunEndpointObservation.rtt_ms` | yes | 失败无映射/RTT；mapped 必须有 | 按 outcome 收紧，不填零或虚构 endpoint |
