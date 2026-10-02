# SPA 操作分类与轮询建议

冻结HTTP源 `2132434`；路径和认证类别来自导出，使用/频率为审查建议，不是当前服务器承诺。

67 个操作全部覆盖；18 个节点/App操作保留其独立生命周期和幂等合同。

配置和观测缓存分开；每次请求捕获当前标签页的身份上下文，晚到响应不能污染新会话。

| operationId | 方法与路径 | 凭据 | SPA用途 / 类别 | 建议节奏 | 重试边界 |
| --- | --- | --- | --- | --- | --- |
| publishConnectionIdentity | POST /api/v1/agent/connection-identity | nodeBearer | 节点 / App，非 SPA 数据源 / node protocol | 由 Agent / App 生命周期决定 | 遵循节点合同；接入/兑换保留精确绑定幂等恢复 |
| getAgentConnections | GET /api/v1/agent/connections | nodeBearer | 节点 / App，非 SPA 数据源 / node protocol | 由 Agent / App 生命周期决定 | 遵循节点合同；接入/兑换保留精确绑定幂等恢复 |
| acknowledgeAgentConnections | POST /api/v1/agent/connections/ack | nodeBearer | 节点 / App，非 SPA 数据源 / node protocol | 由 Agent / App 生命周期决定 | 遵循节点合同；接入/兑换保留精确绑定幂等恢复 |
| reportEndpointDiscovery | POST /api/v1/agent/connections/discovery | nodeBearer | 节点 / App，非 SPA 数据源 / node protocol | 由 Agent / App 生命周期决定 | 遵循节点合同；接入/兑换保留精确绑定幂等恢复 |
| getConnectionRuntimeEpoch | GET /api/v1/agent/connections/runtime | nodeBearer | 节点 / App，非 SPA 数据源 / node protocol | 由 Agent / App 生命周期决定 | 遵循节点合同；接入/兑换保留精确绑定幂等恢复 |
| reportConnectionRuntime | POST /api/v1/agent/connections/runtime | nodeBearer | 节点 / App，非 SPA 数据源 / node protocol | 由 Agent / App 生命周期决定 | 遵循节点合同；接入/兑换保留精确绑定幂等恢复 |
| beginConnectionRuntime | POST /api/v1/agent/connections/runtime/begin | nodeBearer | 节点 / App，非 SPA 数据源 / node protocol | 由 Agent / App 生命周期决定 | 遵循节点合同；接入/兑换保留精确绑定幂等恢复 |
| getAgentDataplane | GET /api/v1/agent/dataplane | nodeBearer | 节点 / App，非 SPA 数据源 / node protocol | 由 Agent / App 生命周期决定 | 遵循节点合同；接入/兑换保留精确绑定幂等恢复 |
| getAgentDns | GET /api/v1/agent/dns | nodeBearer | 节点 / App，非 SPA 数据源 / node protocol | 由 Agent / App 生命周期决定 | 遵循节点合同；接入/兑换保留精确绑定幂等恢复 |
| reportLatencySamples | POST /api/v1/agent/latency/samples | nodeBearer | 节点 / App，非 SPA 数据源 / node protocol | 由 Agent / App 生命周期决定 | 遵循节点合同；接入/兑换保留精确绑定幂等恢复 |
| getAgentManagedConnections | GET /api/v1/agent/managed-connections | nodeBearer | 节点 / App，非 SPA 数据源 / node protocol | 由 Agent / App 生命周期决定 | 遵循节点合同；接入/兑换保留精确绑定幂等恢复 |
| getAgentManagedDataplane | GET /api/v1/agent/managed-dataplane | nodeBearer | 节点 / App，非 SPA 数据源 / node protocol | 由 Agent / App 生命周期决定 | 遵循节点合同；接入/兑换保留精确绑定幂等恢复 |
| publishManagedTransportCapabilities | POST /api/v1/agent/managed-transport-capabilities | nodeBearer | 节点 / App，非 SPA 数据源 / node protocol | 由 Agent / App 生命周期决定 | 遵循节点合同；接入/兑换保留精确绑定幂等恢复 |
| getAgentConfiguration | GET /api/v1/agents/configuration | nodeBearer | 节点 / App，非 SPA 数据源 / node protocol | 由 Agent / App 生命周期决定 | 遵循节点合同；接入/兑换保留精确绑定幂等恢复 |
| acknowledgeConfiguration | POST /api/v1/agents/configuration/ack | nodeBearer | 节点 / App，非 SPA 数据源 / node protocol | 由 Agent / App 生命周期决定 | 遵循节点合同；接入/兑换保留精确绑定幂等恢复 |
| enrollAgent | POST /api/v1/agents/enroll | enrollmentBearer | 节点 / App，非 SPA 数据源 / node protocol | 由 Agent / App 生命周期决定 | 遵循节点合同；接入/兑换保留精确绑定幂等恢复 |
| heartbeatAgent | POST /api/v1/agents/heartbeat | nodeBearer | 节点 / App，非 SPA 数据源 / node protocol | 由 Agent / App 生命周期决定 | 遵循节点合同；接入/兑换保留精确绑定幂等恢复 |
| acceptAccountInvitation | POST /api/v1/auth/invitations/accept | userSession | 人类界面 / 登录或一次性秘密操作 | 不轮询 | 不盲目重试；区分提交失败与响应丢失 |
| registerInvitedAccount | POST /api/v1/auth/invitations/register | anonymous | 人类界面 / 登录或一次性秘密操作 | 不轮询 | 不盲目重试；区分提交失败与响应丢失 |
| loginAccount | POST /api/v1/auth/login | anonymous | 人类界面 / 登录或一次性秘密操作 | 不轮询 | 不盲目重试；区分提交失败与响应丢失 |
| logoutAccount | POST /api/v1/auth/logout | userSession | 人类界面 / mutation | 不轮询 | 不自动重试；失败后核对结果 |
| listAccountPasswordResets | GET /api/v1/auth/password-resets | adminBearer | 实例初始化 / 恢复 / 按需读取 | 导航 / 聚焦 / 相关变更后 | 同一身份作用域内有界退避重试 |
| createAccountPasswordReset | POST /api/v1/auth/password-resets | adminBearer | 实例初始化 / 恢复 / 登录或一次性秘密操作 | 不轮询 | 不盲目重试；区分提交失败与响应丢失 |
| redeemAccountPasswordReset | POST /api/v1/auth/password-resets/redeem | anonymous | 人类界面 / 登录或一次性秘密操作 | 不轮询 | 不盲目重试；区分提交失败与响应丢失 |
| revokeAccountPasswordReset | POST /api/v1/auth/password-resets/revoke | adminBearer | 实例初始化 / 恢复 / mutation | 不轮询 | 相同身份与目标可有界重试，仍检查当前权限 |
| getAccountPasswordReset | GET /api/v1/auth/password-resets/{reset_id} | adminBearer | 实例初始化 / 恢复 / 按需读取 | 导航 / 聚焦 / 相关变更后 | 同一身份作用域内有界退避重试 |
| getAccountSession | GET /api/v1/auth/session | userSession | 人类界面 / 按需读取 | 导航 / 聚焦 / 相关变更后 | 同一身份作用域内有界退避重试 |
| revokeAccountSessions | POST /api/v1/auth/sessions/revoke | managementBearer | 人类界面 / mutation | 不轮询 | 不自动重试；失败后核对结果 |
| exchangeDevice | POST /api/v1/devices/exchange | enrollmentBearer | 节点 / App，非 SPA 数据源 / node protocol | 由 Agent / App 生命周期决定 | 遵循节点合同；接入/兑换保留精确绑定幂等恢复 |
| changeAccountPassword | POST /api/v1/me/password | userSession | 人类界面 / mutation | 不轮询 | 不自动重试；失败后核对结果 |
| listNetworks | GET /api/v1/networks | managementBearer | 人类界面 / 按需读取 | 导航 / 聚焦 / 相关变更后 | 同一身份作用域内有界退避重试 |
| createNetwork | POST /api/v1/networks | adminBearer | 实例初始化 / 恢复 / mutation | 不轮询 | 不自动重试；失败后核对结果 |
| getMeshAccess | GET /api/v1/networks/{network_id}/access | managementBearer | 人类界面 / 按需读取 | 导航 / 聚焦 / 相关变更后 | 同一身份作用域内有界退避重试 |
| saveMeshAccess | POST /api/v1/networks/{network_id}/access | managementBearer | 人类界面 / mutation | 不轮询 | 不自动重试；失败后核对结果 |
| getConnections | GET /api/v1/networks/{network_id}/connections | managementBearer | 人类界面 / 配置 / 状态混合 | 编辑基准按需；观测副本可见时约15–30s | 同一身份作用域内有界退避重试 |
| saveConnections | POST /api/v1/networks/{network_id}/connections | managementBearer | 人类界面 / mutation | 不轮询 | 不自动重试；失败后核对结果 |
| getEndpointDiscoveries | GET /api/v1/networks/{network_id}/connections/discovery | managementBearer | 人类界面 / 动态读取 | 发现面板打开时约10–15s | 同一身份作用域内有界退避重试 |
| getConnectionRuntimes | GET /api/v1/networks/{network_id}/connections/runtime | managementBearer | 人类界面 / 动态读取 | 相关视图可见时约10–15s | 同一身份作用域内有界退避重试 |
| authorizeDevice | POST /api/v1/networks/{network_id}/device-authorizations | userSession | 人类界面 / 登录或一次性秘密操作 | 不轮询 | 不盲目重试；区分提交失败与响应丢失 |
| createEnrollmentToken | POST /api/v1/networks/{network_id}/enrollment-tokens | managementBearer | 人类界面 / 登录或一次性秘密操作 | 不轮询 | 不盲目重试；区分提交失败与响应丢失 |
| previewFlow | POST /api/v1/networks/{network_id}/flows/preview | managementBearer | 人类界面 / POST 只读预览 | 手动或编辑防抖，不定时轮询 | 同一身份/输入下有界重试，过时结果丢弃 |
| listAccountInvitations | GET /api/v1/networks/{network_id}/invitations | managementBearer | 人类界面 / 按需读取 | 导航 / 聚焦 / 相关变更后 | 同一身份作用域内有界退避重试 |
| createAccountInvitation | POST /api/v1/networks/{network_id}/invitations | managementBearer | 人类界面 / 登录或一次性秘密操作 | 不轮询 | 不盲目重试；区分提交失败与响应丢失 |
| revokeAccountInvitation | POST /api/v1/networks/{network_id}/invitations/revoke | managementBearer | 人类界面 / mutation | 不轮询 | 相同身份与目标可有界重试，仍检查当前权限 |
| getAccountInvitation | GET /api/v1/networks/{network_id}/invitations/{invitation_id} | managementBearer | 人类界面 / 按需读取 | 导航 / 聚焦 / 相关变更后 | 同一身份作用域内有界退避重试 |
| queryLatencyHistory | POST /api/v1/networks/{network_id}/latency/history/query | managementBearer | 人类界面 / POST 只读历史查询（内部有清理写入） | 按需；曲线可见时约30–60s | 固定窗口分页；跨窗口不能直接复用after |
| createManagedEnrollmentToken | POST /api/v1/networks/{network_id}/managed-enrollment-tokens | managementBearer | 人类界面 / 登录或一次性秘密操作 | 不轮询 | 不盲目重试；区分提交失败与响应丢失 |
| getManagedListeners | GET /api/v1/networks/{network_id}/managed-listeners | managementBearer | 人类界面 / 配置 / 状态混合 | 配置按需；节点详情可选约30s | 同一身份作用域内有界退避重试 |
| saveManagedListener | POST /api/v1/networks/{network_id}/managed-listeners | managementBearer | 人类界面 / mutation | 不轮询 | 不自动重试；失败后核对结果 |
| listAccountMembers | GET /api/v1/networks/{network_id}/members | managementBearer | 人类界面 / 按需读取 | 导航 / 聚焦 / 相关变更后 | 同一身份作用域内有界退避重试 |
| leaveAccountNetwork | POST /api/v1/networks/{network_id}/members/leave | userSession | 人类界面 / mutation | 不轮询 | 不自动重试；失败后核对结果 |
| saveAccountMember | POST /api/v1/networks/{network_id}/members/save | managementBearer | 人类界面 / mutation | 不轮询 | 不自动重试；失败后核对结果 |
| getNetwork | GET /api/v1/networks/{network_id}/network | managementBearer | 人类界面 / 按需读取 | 导航 / 聚焦 / 相关变更后 | 同一身份作用域内有界退避重试 |
| saveNetwork | POST /api/v1/networks/{network_id}/network | managementBearer | 人类界面 / mutation | 不轮询 | 不自动重试；失败后核对结果 |
| listNodeOwnership | GET /api/v1/networks/{network_id}/node-ownership | managementBearer | 人类界面 / 按需读取 | 导航 / 聚焦 / 相关变更后 | 同一身份作用域内有界退避重试 |
| claimNodeOwnership | POST /api/v1/networks/{network_id}/node-ownership/claim | userSession | 人类界面 / mutation | 不轮询 | 不自动重试；失败后核对结果 |
| listNodes | GET /api/v1/networks/{network_id}/nodes | managementBearer | 人类界面 / 配置 / 状态混合 | 节点视图可见时约15–30s | 同一身份作用域内有界退避重试 |
| setNodeCountry | POST /api/v1/networks/{network_id}/nodes/country | managementBearer | 兼容保留，非日常地区入口 / legacy mutation | 不轮询 | 不自动重试；失败后核对结果 |
| revokeNode | POST /api/v1/networks/{network_id}/nodes/revoke | managementBearer | 人类界面 / mutation | 不轮询 | 相同身份与目标可有界重试，仍检查当前权限 |
| setNodeTags | POST /api/v1/networks/{network_id}/nodes/tags | managementBearer | 人类界面 / mutation | 不轮询 | 不自动重试；失败后核对结果 |
| getOverview | GET /api/v1/networks/{network_id}/overview | managementBearer | 人类界面 / 动态读取 | 概览可见时约30s | 同一身份作用域内有界退避重试 |
| claimFirstOwner | POST /api/v1/networks/{network_id}/owner-claim | adminBearer | 实例初始化 / 恢复 / mutation | 不轮询 | 不自动重试；失败后核对结果 |
| getPolicy | GET /api/v1/networks/{network_id}/policy | managementBearer | 人类界面 / 按需读取 | 导航 / 聚焦 / 相关变更后 | 同一身份作用域内有界退避重试 |
| previewPolicy | POST /api/v1/networks/{network_id}/policy/preview | managementBearer | 人类界面 / POST 只读预览 | 手动或编辑防抖，不定时轮询 | 同一身份/输入下有界重试，过时结果丢弃 |
| savePolicy | POST /api/v1/networks/{network_id}/policy/save | managementBearer | 人类界面 / mutation | 不轮询 | 不自动重试；失败后核对结果 |
| getTags | GET /api/v1/networks/{network_id}/tags | managementBearer | 人类界面 / 按需读取 | 导航 / 聚焦 / 相关变更后 | 同一身份作用域内有界退避重试 |
| saveTags | POST /api/v1/networks/{network_id}/tags | managementBearer | 人类界面 / mutation | 不轮询 | 不自动重试；失败后核对结果 |
