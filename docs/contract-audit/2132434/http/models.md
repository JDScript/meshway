# 模型与字段完整目录

生成快照：`2132434`；输入 SHA-256：`2564515d220241fed0123eb9ecdb675fad825c7d1d54fac8d5ea30d58a02cc55`。

required 表示在所属对象 / 分支中必须出现；nullable 表示 JSON 字面量 null 是否被 schema 接受。
不是字段使用建议或完整业务校验；分支内字段的 required 不表示所有分支都携带该字段。

161 schemas，477 个对象字段；5 个可省略，57 个可空。

## AcceptInvitationRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`acceptAccountInvitation`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` AcceptInvitationRequest.invitation_token ` | ` string ` | yes | no | ` {"minLength":68,"maxLength":68,"pattern":"^mwi_[0-9a-f]{64}$"} ` |  |

## AccessAction

类型：` string `；模型可空：no。

模型约束：` {"enum":["allow","reject"]} `。

枚举：` ["allow", "reject"] `。

request 使用：`saveMeshAccess`

response 使用：`getMeshAccess`, `saveMeshAccess`

## AccountInvitation

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`listAccountInvitations`, `createAccountInvitation`, `getAccountInvitation`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` AccountInvitation.consumed_at ` | ` integer \| null ` | yes | yes | ` {"format":"int64"} ` |  |
| ` AccountInvitation.consumed_by ` | ` string \| null ` | yes | yes | ` — ` |  |
| ` AccountInvitation.email ` | ` string ` | yes | no | ` — ` |  |
| ` AccountInvitation.expires_at ` | ` integer ` | yes | no | ` {"format":"int64"} ` |  |
| ` AccountInvitation.invitation_id ` | ` string ` | yes | no | ` — ` |  |
| ` AccountInvitation.issuer_user_id ` | ` string \| null ` | yes | yes | ` — ` |  |
| ` AccountInvitation.network_id ` | ` string ` | yes | no | ` — ` |  |
| ` AccountInvitation.revoked_at ` | ` integer \| null ` | yes | yes | ` {"format":"int64"} ` |  |
| ` AccountInvitation.role ` | ` AccountRole ` | yes | no | ` — ` |  |

## AccountInvitations

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`listAccountInvitations`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` AccountInvitations.invitations ` | ` array<AccountInvitation> ` | yes | no | ` — ` |  |

## AccountMember

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`listAccountMembers`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` AccountMember.membership ` | ` AccountMembership ` | yes | no | ` — ` |  |
| ` AccountMember.user ` | ` AccountUser ` | yes | no | ` — ` |  |

## AccountMembers

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`listAccountMembers`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` AccountMembers.members ` | ` array<AccountMember> ` | yes | no | ` — ` |  |

## AccountMembership

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`acceptAccountInvitation`, `registerInvitedAccount`, `getAccountSession`, `listAccountMembers`, `claimFirstOwner`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` AccountMembership.membership_id ` | ` string ` | yes | no | ` — ` |  |
| ` AccountMembership.network_id ` | ` string ` | yes | no | ` — ` |  |
| ` AccountMembership.role ` | ` AccountRole ` | yes | no | ` — ` |  |
| ` AccountMembership.user_id ` | ` string ` | yes | no | ` — ` |  |

## AccountMembershipChange

类型：` string `；模型可空：no。

模型约束：` {"enum":["owner","admin","member","remove"]} `。

枚举：` ["owner", "admin", "member", "remove"] `。

request 使用：`saveAccountMember`

response 使用：—

## AccountPasswordReset

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`listAccountPasswordResets`, `createAccountPasswordReset`, `getAccountPasswordReset`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` AccountPasswordReset.consumed_at ` | ` integer \| null ` | yes | yes | ` {"format":"int64"} ` |  |
| ` AccountPasswordReset.credential_current ` | ` boolean ` | yes | no | ` — ` |  |
| ` AccountPasswordReset.expires_at ` | ` integer ` | yes | no | ` {"format":"int64"} ` |  |
| ` AccountPasswordReset.reset_id ` | ` string ` | yes | no | ` — ` |  |
| ` AccountPasswordReset.revoked_at ` | ` integer \| null ` | yes | yes | ` {"format":"int64"} ` |  |
| ` AccountPasswordReset.user_id ` | ` string ` | yes | no | ` — ` |  |

## AccountPasswordResets

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`listAccountPasswordResets`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` AccountPasswordResets.resets ` | ` array<AccountPasswordReset> ` | yes | no | ` — ` |  |

## AccountReceipt

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`logoutAccount`, `redeemAccountPasswordReset`, `revokeAccountPasswordReset`, `revokeAccountSessions`, `changeAccountPassword`, `revokeAccountInvitation`, `leaveAccountNetwork`, `saveAccountMember`, `claimNodeOwnership`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` AccountReceipt.saved ` | ` boolean ` | yes | no | ` — ` |  |

## AccountRole

类型：` string `；模型可空：no。

模型约束：` {"enum":["owner","admin","member"]} `。

枚举：` ["owner", "admin", "member"] `。

request 使用：`createAccountInvitation`

response 使用：`acceptAccountInvitation`, `registerInvitedAccount`, `getAccountSession`, `listAccountInvitations`, `createAccountInvitation`, `getAccountInvitation`, `listAccountMembers`, `claimFirstOwner`

## AccountSession

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`getAccountSession`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` AccountSession.memberships ` | ` array<AccountMembership> ` | yes | no | ` — ` |  |
| ` AccountSession.user ` | ` AccountUser ` | yes | no | ` — ` |  |

## AccountUser

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`getAccountSession`, `listAccountMembers`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` AccountUser.email ` | ` string ` | yes | no | ` — ` |  |
| ` AccountUser.email_verified ` | ` boolean ` | yes | no | ` — ` |  |
| ` AccountUser.user_id ` | ` string ` | yes | no | ` — ` |  |

## AcknowledgeAgentConnectionsRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`acknowledgeAgentConnections`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` AcknowledgeAgentConnectionsRequest.digest ` | ` string ` | yes | no | ` {"minLength":64,"maxLength":64,"pattern":"^[0-9a-f]{64}$"} ` |  |
| ` AcknowledgeAgentConnectionsRequest.revision ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |

## AcknowledgeAgentConnectionsResponse

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`acknowledgeAgentConnections`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` AcknowledgeAgentConnectionsResponse.digest ` | ` string ` | yes | no | ` {"minLength":64,"maxLength":64,"pattern":"^[0-9a-f]{64}$"} ` |  |
| ` AcknowledgeAgentConnectionsResponse.revision ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |
| ` AcknowledgeAgentConnectionsResponse.status ` | ` ConnectionReceiptStatus ` | yes | no | ` — ` |  |

## AcknowledgeConfigurationRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`acknowledgeConfiguration`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` AcknowledgeConfigurationRequest.digest ` | ` string ` | yes | no | ` {"minLength":64,"maxLength":64,"pattern":"^[0-9a-f]{64}$"} ` |  |
| ` AcknowledgeConfigurationRequest.revision ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |

## AcknowledgeConfigurationResponse

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`acknowledgeConfiguration`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` AcknowledgeConfigurationResponse.digest ` | ` string ` | yes | no | ` {"minLength":64,"maxLength":64,"pattern":"^[0-9a-f]{64}$"} ` |  |
| ` AcknowledgeConfigurationResponse.revision ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |

## AgentConnectionsSnapshot

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`getAgentConnections`, `getAgentManagedConnections`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` AgentConnectionsSnapshot.digest ` | ` string ` | yes | no | ` {"minLength":64,"maxLength":64,"pattern":"^[0-9a-f]{64}$"} ` |  |
| ` AgentConnectionsSnapshot.payload ` | ` NodeConnections ` | yes | no | ` — ` |  |

## AgentDataplaneSnapshot

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`getAgentDataplane`, `getAgentManagedDataplane`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` AgentDataplaneSnapshot.digest ` | ` string ` | yes | no | ` {"minLength":64,"maxLength":64,"pattern":"^[0-9a-f]{64}$"} ` |  |
| ` AgentDataplaneSnapshot.payload ` | ` NodeDataplane ` | yes | no | ` — ` |  |

## AgentDnsSnapshot

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`getAgentDns`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` AgentDnsSnapshot.digest ` | ` string ` | yes | no | ` {"minLength":64,"maxLength":64,"pattern":"^[0-9a-f]{64}$"} ` |  |
| ` AgentDnsSnapshot.payload ` | ` NodeDnsConfiguration ` | yes | no | ` — ` |  |

## AgentManagedConnectionsSnapshot

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`getAgentManagedConnections`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` AgentManagedConnectionsSnapshot.connections ` | ` AgentConnectionsSnapshot ` | yes | no | ` — ` |  |
| ` AgentManagedConnectionsSnapshot.digest ` | ` string ` | yes | no | ` — ` |  |
| ` AgentManagedConnectionsSnapshot.materials ` | ` array<ManagedVlessMaterial> ` | yes | no | ` {"maxItems":128} ` |  |
| ` AgentManagedConnectionsSnapshot.network_id ` | ` string ` | yes | no | ` — ` |  |

## AgentManagedDataplaneSnapshot

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`getAgentManagedDataplane`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` AgentManagedDataplaneSnapshot.dataplane ` | ` AgentDataplaneSnapshot ` | yes | no | ` — ` |  |
| ` AgentManagedDataplaneSnapshot.digest ` | ` string ` | yes | no | ` — ` |  |
| ` AgentManagedDataplaneSnapshot.materials ` | ` array<ManagedVlessMaterial> ` | yes | no | ` {"maxItems":128} ` |  |
| ` AgentManagedDataplaneSnapshot.network_id ` | ` string ` | yes | no | ` — ` |  |

## AgentNodeResponse

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

Also lets a pending local enrollment recover its server-assigned node ID.

request 使用：—

response 使用：`enrollAgent`, `heartbeatAgent`, `exchangeDevice`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` AgentNodeResponse.node ` | ` NodeSummary ` | yes | no | ` — ` |  |

## AuthorizeDeviceRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`authorizeDevice`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` AuthorizeDeviceRequest.agent_version ` | ` string ` | yes | no | ` {"minLength":1,"maxLength":64,"pattern":"[^\\u0009-\\u000D\\u0020\\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000]"} ` |  |
| ` AuthorizeDeviceRequest.code_challenge ` | ` string ` | yes | no | ` {"minLength":43,"maxLength":43,"pattern":"^[A-Za-z0-9_-]{42}[AEIMQUYcgkosw048]$"} ` |  |
| ` AuthorizeDeviceRequest.name ` | ` string ` | yes | no | ` {"minLength":1,"maxLength":128,"pattern":"[^\\u0009-\\u000D\\u0020\\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000]"} ` |  |
| ` AuthorizeDeviceRequest.redirect_uri ` | ` string ` | yes | no | ` {"minLength":25,"maxLength":25,"pattern":"^meshway://enroll/callback$"} ` |  |

## AuthorizeDeviceResponse

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`authorizeDevice`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` AuthorizeDeviceResponse.code ` | ` string ` | yes | no | ` — ` |  |
| ` AuthorizeDeviceResponse.expires_at ` | ` string ` | yes | no | ` {"format":"date-time"} ` |  |

## BeginConnectionRuntimeRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`beginConnectionRuntime`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` BeginConnectionRuntimeRequest.expected_epoch ` | ` integer ` | yes | no | ` {"format":"int64","minimum":0,"maximum":4294967295} ` | Pin the initial epoch read across retries; never refresh after a conflict. |
| ` BeginConnectionRuntimeRequest.instance_id ` | ` string ` | yes | no | ` {"minLength":64,"maxLength":64,"pattern":"^[0-9a-f]{64}$"} ` |  |

## BeginConnectionRuntimeResponse

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`beginConnectionRuntime`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` BeginConnectionRuntimeResponse.epoch ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |

## ChangePasswordRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`changeAccountPassword`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` ChangePasswordRequest.current_password ` | ` string ` | yes | no | ` — ` |  |
| ` ChangePasswordRequest.new_password ` | ` string ` | yes | no | ` — ` |  |

## ClaimNodeOwnershipRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`claimNodeOwnership`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` ClaimNodeOwnershipRequest.membership_id ` | ` string ` | yes | no | ` — ` |  |
| ` ClaimNodeOwnershipRequest.node_id ` | ` string ` | yes | no | ` — ` |  |

## ClaimOwnerKind

类型：` string `；模型可空：no。

模型约束：` {"enum":["new_account","existing_account"]} `。

枚举：` ["new_account", "existing_account"] `。

request 使用：`claimFirstOwner`

response 使用：—

## ClaimOwnerRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

Bootstrap proof comes from adminBearer; existing accounts must prove their own session.

request 使用：`claimFirstOwner`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` ClaimOwnerRequest.email ` | ` string \| null ` | no | yes | ` — ` |  |
| ` ClaimOwnerRequest.kind ` | ` ClaimOwnerKind ` | yes | no | ` — ` |  |
| ` ClaimOwnerRequest.password ` | ` string \| null ` | no | yes | ` — ` |  |
| ` ClaimOwnerRequest.session_token ` | ` string \| null ` | no | yes | ` — ` |  |

## ConfigurationSnapshot

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`getAgentConfiguration`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` ConfigurationSnapshot.digest ` | ` string ` | yes | no | ` {"minLength":64,"maxLength":64,"pattern":"^[0-9a-f]{64}$"} ` |  |
| ` ConfigurationSnapshot.payload ` | ` NodeConfiguration ` | yes | no | ` — ` |  |

## ConnectionCandidate

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`saveConnections`

response 使用：`getConnections`, `saveConnections`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` ConnectionCandidate.credential_ref ` | ` string \| null ` | yes | yes | ` {"minLength":1,"maxLength":64,"pattern":"^[A-Za-z0-9._-]+$"} ` |  |
| ` ConnectionCandidate.dialer_node_id ` | ` string ` | yes | no | ` — ` |  |
| ` ConnectionCandidate.endpoint ` | ` ConnectionEndpoint ` | yes | no | ` — ` |  |
| ` ConnectionCandidate.id ` | ` string ` | yes | no | ` {"minLength":1,"maxLength":64,"pattern":"^[A-Za-z0-9._-]+$"} ` |  |
| ` ConnectionCandidate.listener_id ` | ` string ` | yes | no | ` — ` |  |
| ` ConnectionCandidate.priority ` | ` integer ` | yes | no | ` {"format":"int32","minimum":0,"maximum":65535} ` |  |

## ConnectionCapability

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`publishConnectionIdentity`

response 使用：`publishConnectionIdentity`, `getAgentConnections`, `getAgentDataplane`, `getAgentManagedConnections`, `getAgentManagedDataplane`, `getConnections`, `saveConnections`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` ConnectionCapability.can_accept ` | ` boolean ` | yes | no | ` — ` |  |
| ` ConnectionCapability.can_dial ` | ` boolean ` | yes | no | ` — ` |  |
| ` ConnectionCapability.semantics ` | ` ConnectionSemantics ` | yes | no | ` — ` |  |
| ` ConnectionCapability.transport ` | ` ConnectionTransport ` | yes | no | ` — ` |  |

## ConnectionDiagnostic

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`getAgentConnections`, `getAgentDataplane`, `getAgentManagedConnections`, `getAgentManagedDataplane`, `getConnections`, `saveConnections`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` ConnectionDiagnostic.field ` | ` string ` | yes | no | ` — ` |  |
| ` ConnectionDiagnostic.item_id ` | ` string \| null ` | yes | yes | ` — ` |  |
| ` ConnectionDiagnostic.message ` | ` string ` | yes | no | ` — ` |  |

## ConnectionEndpoint

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`saveConnections`, `createManagedEnrollmentToken`, `saveManagedListener`

response 使用：`getAgentConnections`, `getAgentDataplane`, `getAgentManagedConnections`, `getAgentManagedDataplane`, `getConnections`, `saveConnections`, `getManagedListeners`, `saveManagedListener`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` ConnectionEndpoint.host ` | ` string ` | yes | no | ` {"minLength":1,"maxLength":253} ` |  |
| ` ConnectionEndpoint.port ` | ` integer ` | yes | no | ` {"format":"int32","minimum":1,"maximum":65535} ` |  |

## ConnectionListener

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`saveConnections`

response 使用：`getAgentConnections`, `getAgentDataplane`, `getAgentManagedConnections`, `getAgentManagedDataplane`, `getConnections`, `saveConnections`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` ConnectionListener.advertised_endpoints ` | ` array<ConnectionEndpoint> ` | yes | no | ` {"maxItems":16} ` |  |
| ` ConnectionListener.bind ` | ` string ` | yes | no | ` — ` |  |
| ` ConnectionListener.credential_ref ` | ` string \| null ` | yes | yes | ` {"minLength":1,"maxLength":64,"pattern":"^[A-Za-z0-9._-]+$"} ` |  |
| ` ConnectionListener.id ` | ` string ` | yes | no | ` {"minLength":1,"maxLength":64,"pattern":"^[A-Za-z0-9._-]+$"} ` |  |
| ` ConnectionListener.node_id ` | ` string ` | yes | no | ` — ` |  |
| ` ConnectionListener.transport ` | ` ConnectionTransport ` | yes | no | ` — ` |  |

## ConnectionListenerObservation

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`reportConnectionRuntime`

response 使用：`getConnectionRuntimes`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` ConnectionListenerObservation.listener_id ` | ` string ` | yes | no | ` {"minLength":1,"maxLength":64,"pattern":"^[A-Za-z0-9._-]+$"} ` |  |
| ` ConnectionListenerObservation.reason ` | ` ConnectionRuntimeReason ` | yes | no | ` — ` |  |
| ` ConnectionListenerObservation.state ` | ` ConnectionListenerState ` | yes | no | ` — ` |  |

## ConnectionListenerState

类型：` string `；模型可空：no。

模型约束：` {"enum":["bound","error"]} `。

枚举：` ["bound", "error"] `。

request 使用：`reportConnectionRuntime`

response 使用：`getConnectionRuntimes`

## ConnectionNeighborObservation

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`reportConnectionRuntime`

response 使用：`getConnectionRuntimes`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` ConnectionNeighborObservation.candidate_id ` | ` string \| null ` | yes | yes | ` {"minLength":1,"maxLength":64,"pattern":"^[A-Za-z0-9._-]+$"} ` |  |
| ` ConnectionNeighborObservation.initiator_node_id ` | ` string ` | yes | no | ` {"minLength":1,"maxLength":64,"pattern":"^[A-Za-z0-9._-]+$"} ` |  |
| ` ConnectionNeighborObservation.listener_id ` | ` string \| null ` | yes | yes | ` {"minLength":1,"maxLength":64,"pattern":"^[A-Za-z0-9._-]+$"} ` |  |
| ` ConnectionNeighborObservation.peer_node_id ` | ` string ` | yes | no | ` {"minLength":1,"maxLength":64,"pattern":"^[A-Za-z0-9._-]+$"} ` |  |
| ` ConnectionNeighborObservation.probe_fresh ` | ` boolean ` | yes | no | ` — ` | A recent authenticated control-nonce probe, not merely a control heartbeat. |
| ` ConnectionNeighborObservation.probe_rtt_ms ` | ` integer \| null ` | yes | yes | ` {"format":"int64","minimum":0,"maximum":4294967295} ` |  |
| ` ConnectionNeighborObservation.reason ` | ` ConnectionRuntimeReason ` | yes | no | ` — ` | Sanitized peer-local outcome, independent of global reconcile state. |
| ` ConnectionNeighborObservation.state ` | ` ConnectionNeighborState ` | yes | no | ` — ` |  |
| ` ConnectionNeighborObservation.transport ` | ` ConnectionTransport ` | yes | no | ` — ` |  |

## ConnectionNeighborState

类型：` string `；模型可空：no。

模型约束：` {"enum":["authenticated","waiting","unreachable"]} `。

枚举：` ["authenticated", "waiting", "unreachable"] `。

request 使用：`reportConnectionRuntime`

response 使用：`getConnectionRuntimes`

## ConnectionNode

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`getConnections`, `saveConnections`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` ConnectionNode.acknowledged_at ` | ` string \| null ` | yes | yes | ` {"format":"date-time"} ` |  |
| ` ConnectionNode.acknowledged_digest ` | ` string \| null ` | yes | yes | ` {"minLength":64,"maxLength":64,"pattern":"^[0-9a-f]{64}$"} ` |  |
| ` ConnectionNode.acknowledged_revision ` | ` integer ` | yes | no | ` {"format":"int64","minimum":0,"maximum":4294967295} ` |  |
| ` ConnectionNode.capabilities ` | ` array<ConnectionCapability> ` | yes | no | ` — ` |  |
| ` ConnectionNode.node ` | ` NodeSummary ` | yes | no | ` — ` |  |
| ` ConnectionNode.public_key ` | ` string \| null ` | yes | yes | ` {"minLength":64,"maxLength":64,"pattern":"^[0-9a-f]{64}$"} ` |  |

## ConnectionPeer

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`getAgentConnections`, `getAgentDataplane`, `getAgentManagedConnections`, `getAgentManagedDataplane`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` ConnectionPeer.node_id ` | ` string ` | yes | no | ` — ` |  |
| ` ConnectionPeer.public_key ` | ` string ` | yes | no | ` {"minLength":64,"maxLength":64,"pattern":"^[0-9a-f]{64}$"} ` |  |

## ConnectionPlan

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`saveConnections`

response 使用：`getConnections`, `saveConnections`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` ConnectionPlan.candidates ` | ` array<ConnectionCandidate> ` | yes | no | ` {"maxItems":256} ` |  |
| ` ConnectionPlan.listeners ` | ` array<ConnectionListener> ` | yes | no | ` {"maxItems":128} ` |  |

## ConnectionReceiptStatus

类型：` string `；模型可空：no。

模型约束：` {"enum":["saved"]} `。

枚举：` ["saved"] `。

request 使用：—

response 使用：`acknowledgeAgentConnections`

## ConnectionRuntimeObservation

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`getConnectionRuntimes`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` ConnectionRuntimeObservation.current ` | ` boolean ` | yes | no | ` — ` | Matches the current node-scoped revision/digest and node is active. |
| ` ConnectionRuntimeObservation.digest ` | ` string \| null ` | yes | yes | ` {"minLength":64,"maxLength":64,"pattern":"^[0-9a-f]{64}$"} ` |  |
| ` ConnectionRuntimeObservation.epoch ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |
| ` ConnectionRuntimeObservation.listeners ` | ` array<ConnectionListenerObservation> ` | yes | no | ` {"maxItems":128} ` |  |
| ` ConnectionRuntimeObservation.node_id ` | ` string ` | yes | no | ` — ` |  |
| ` ConnectionRuntimeObservation.peers ` | ` array<ConnectionNeighborObservation> ` | yes | no | ` {"maxItems":256} ` |  |
| ` ConnectionRuntimeObservation.reason ` | ` ConnectionRuntimeReason ` | yes | no | ` — ` |  |
| ` ConnectionRuntimeObservation.reported_at ` | ` string \| null ` | yes | yes | ` {"format":"date-time"} ` |  |
| ` ConnectionRuntimeObservation.revision ` | ` integer \| null ` | yes | yes | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |
| ` ConnectionRuntimeObservation.sequence ` | ` integer ` | yes | no | ` {"format":"int64","minimum":0,"maximum":4294967295} ` |  |
| ` ConnectionRuntimeObservation.stale ` | ` boolean ` | yes | no | ` — ` | Not current, never reported, or receive age &gt;= stale_after_seconds. |
| ` ConnectionRuntimeObservation.stale_after_seconds ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |
| ` ConnectionRuntimeObservation.state ` | ` ConnectionRuntimeState ` | yes | no | ` — ` |  |

## ConnectionRuntimeReason

类型：` string `；模型可空：no。

模型约束：` {"enum":["none","disabled","invalid_configuration","profile_unavailable","bind_failed","resolve_failed","transport_failed","authentication_failed","probe_failed","lease_superseded","lease_expired","shutting_down","internal_error"]} `。

枚举：` ["none", "disabled", "invalid_configuration", "profile_unavailable", "bind_failed", "resolve_failed", "transport_failed", "authentication_failed", "probe_failed", "lease_superseded", "lease_expired", "shutting_down", "internal_error"] `。

request 使用：`reportConnectionRuntime`

response 使用：`getConnectionRuntimes`

## ConnectionRuntimeState

类型：` string `；模型可空：no。

模型约束：` {"enum":["awaiting_report","disabled","applying","applied","error","withdrawn"]} `。

枚举：` ["awaiting_report", "disabled", "applying", "applied", "error", "withdrawn"] `。

request 使用：`reportConnectionRuntime`

response 使用：`getConnectionRuntimes`

## ConnectionRuntimesSnapshot

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`getConnectionRuntimes`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` ConnectionRuntimesSnapshot.nodes ` | ` array<ConnectionRuntimeObservation> ` | yes | no | ` — ` |  |

## ConnectionSemantics

类型：` string `；模型可空：no。

模型约束：` {"enum":["datagram","stream"]} `。

枚举：` ["datagram", "stream"] `。

request 使用：`publishConnectionIdentity`

response 使用：`publishConnectionIdentity`, `getAgentConnections`, `getAgentDataplane`, `getAgentManagedConnections`, `getAgentManagedDataplane`, `getConnections`, `saveConnections`

## ConnectionTransport

类型：` string `；模型可空：no。

模型约束：` {"enum":["native_udp","native_tcp","vless"]} `。

枚举：` ["native_udp", "native_tcp", "vless"] `。

request 使用：`publishConnectionIdentity`, `reportConnectionRuntime`, `reportLatencySamples`, `saveConnections`, `createManagedEnrollmentToken`, `saveManagedListener`

response 使用：`publishConnectionIdentity`, `getAgentConnections`, `getAgentDataplane`, `getAgentManagedConnections`, `getAgentManagedDataplane`, `getConnections`, `saveConnections`, `getConnectionRuntimes`, `queryLatencyHistory`, `getManagedListeners`, `saveManagedListener`

## ConnectionsSnapshot

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`getConnections`, `saveConnections`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` ConnectionsSnapshot.configuration ` | ` ConnectionPlan ` | yes | no | ` — ` |  |
| ` ConnectionsSnapshot.diagnostics ` | ` array<ConnectionDiagnostic> ` | yes | no | ` — ` |  |
| ` ConnectionsSnapshot.nodes ` | ` array<ConnectionNode> ` | yes | no | ` — ` |  |
| ` ConnectionsSnapshot.revision ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |
| ` ConnectionsSnapshot.valid ` | ` boolean ` | yes | no | ` — ` |  |

## CreateEnrollmentTokenRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`createEnrollmentToken`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` CreateEnrollmentTokenRequest.expires_in_seconds ` | ` integer ` | yes | no | ` {"format":"int32","minimum":60,"maximum":86400} ` |  |

## CreateEnrollmentTokenResponse

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

The token is revealed only once, in this creation response.

request 使用：—

response 使用：`createEnrollmentToken`, `createManagedEnrollmentToken`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` CreateEnrollmentTokenResponse.expires_at ` | ` string ` | yes | no | ` {"format":"date-time"} ` |  |
| ` CreateEnrollmentTokenResponse.token ` | ` string ` | yes | no | ` — ` |  |
| ` CreateEnrollmentTokenResponse.token_id ` | ` string ` | yes | no | ` — ` |  |

## CreateInvitationRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`createAccountInvitation`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` CreateInvitationRequest.email ` | ` string ` | yes | no | ` — ` |  |
| ` CreateInvitationRequest.expires_in_seconds ` | ` integer ` | yes | no | ` {"format":"int32","minimum":0} ` |  |
| ` CreateInvitationRequest.role ` | ` AccountRole ` | yes | no | ` — ` |  |

## CreateInvitationResponse

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`createAccountInvitation`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` CreateInvitationResponse.invitation ` | ` AccountInvitation ` | yes | no | ` — ` |  |
| ` CreateInvitationResponse.invitation_token ` | ` string ` | yes | no | ` {"minLength":68,"maxLength":68,"pattern":"^mwi_[0-9a-f]{64}$"} ` |  |

## CreateManagedEnrollmentTokenRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`createManagedEnrollmentToken`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` CreateManagedEnrollmentTokenRequest.expires_in_seconds ` | ` integer ` | yes | no | ` {"format":"int32","minimum":60,"maximum":86400} ` |  |
| ` CreateManagedEnrollmentTokenRequest.initial_listener ` | ` ManagedListenerSettings ` | yes | no | ` — ` | Initial intent only: node_id MUST be empty and is bound by enrollment. |

## CreateNetworkRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`createNetwork`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` CreateNetworkRequest.name ` | ` string ` | yes | no | ` {"minLength":1,"maxLength":128,"pattern":"[^\\u0009-\\u000D\\u0020\\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000]"} ` |  |

## CreatePasswordResetRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`createAccountPasswordReset`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` CreatePasswordResetRequest.expires_in_seconds ` | ` integer ` | yes | no | ` {"format":"int32","minimum":0} ` |  |
| ` CreatePasswordResetRequest.user_id ` | ` string ` | yes | no | ` — ` |  |

## CreatePasswordResetResponse

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`createAccountPasswordReset`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` CreatePasswordResetResponse.reset ` | ` AccountPasswordReset ` | yes | no | ` — ` |  |
| ` CreatePasswordResetResponse.reset_token ` | ` string ` | yes | no | ` {"minLength":68,"maxLength":68,"pattern":"^mwr_[0-9a-f]{64}$"} ` |  |

## DefaultRouteAction

类型：` string `；模型可空：no。

模型约束：` {"enum":["direct","reject"]} `。

枚举：` ["direct", "reject"] `。

request 使用：`previewPolicy`, `savePolicy`

response 使用：`getAgentDataplane`, `getAgentManagedDataplane`, `getAgentConfiguration`, `getPolicy`, `savePolicy`

## DestinationScope

类型：` string `；模型可空：no。

模型约束：` {"enum":["any","mesh","internet"]} `。

枚举：` ["any", "mesh", "internet"] `。

request 使用：`previewPolicy`, `savePolicy`

response 使用：`getAgentDataplane`, `getAgentManagedDataplane`, `getAgentConfiguration`, `getPolicy`, `savePolicy`

## DestinationSelector

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`previewPolicy`, `savePolicy`

response 使用：`getAgentDataplane`, `getAgentManagedDataplane`, `getAgentConfiguration`, `getPolicy`, `savePolicy`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` DestinationSelector.kind ` | ` DestinationSelectorKind ` | yes | no | ` — ` |  |
| ` DestinationSelector.value ` | ` string ` | yes | no | ` — ` |  |

## DestinationSelectorKind

类型：` string `；模型可空：no。

模型约束：` {"enum":["node","domain_suffix","cidr","country"]} `。

枚举：` ["node", "domain_suffix", "cidr", "country"] `。

request 使用：`previewPolicy`, `savePolicy`

response 使用：`getAgentDataplane`, `getAgentManagedDataplane`, `getAgentConfiguration`, `getPolicy`, `savePolicy`

## EndpointDiscoveriesSnapshot

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`getEndpointDiscoveries`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` EndpointDiscoveriesSnapshot.network_id ` | ` string ` | yes | no | ` — ` |  |
| ` EndpointDiscoveriesSnapshot.nodes ` | ` array<NodeEndpointDiscovery> ` | yes | no | ` — ` |  |

## EndpointDiscoverySource

类型：` string `；模型可空：no。

模型约束：` {"enum":["managed_udp_socket_stun"]} `。

枚举：` ["managed_udp_socket_stun"] `。

request 使用：`reportEndpointDiscovery`

response 使用：`getEndpointDiscoveries`

## EndpointDiscoveryState

类型：` string `；模型可空：no。

模型约束：` {"enum":["awaiting_report","disabled","probing","observed","withdrawn","expired","error"]} `。

枚举：` ["awaiting_report", "disabled", "probing", "observed", "withdrawn", "expired", "error"] `。

request 使用：`reportEndpointDiscovery`

response 使用：`getEndpointDiscoveries`

## EnrollAgentRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

Persist the proposed random credential before sending this request. Retry<br>with the same enrollment token, credential, name and enrollment version.

request 使用：`enrollAgent`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` EnrollAgentRequest.agent_version ` | ` string ` | yes | no | ` {"minLength":1,"maxLength":64,"pattern":"[^\\u0009-\\u000D\\u0020\\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000]"} ` |  |
| ` EnrollAgentRequest.name ` | ` string ` | yes | no | ` {"minLength":1,"maxLength":128,"pattern":"[^\\u0009-\\u000D\\u0020\\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000]"} ` |  |
| ` EnrollAgentRequest.node_credential ` | ` string ` | yes | no | ` {"minLength":64,"maxLength":64,"pattern":"^[0-9a-f]{64}$"} ` |  |

## ErrorResponse

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`publishConnectionIdentity`, `getAgentConnections`, `acknowledgeAgentConnections`, `reportEndpointDiscovery`, `getConnectionRuntimeEpoch`, `reportConnectionRuntime`, `beginConnectionRuntime`, `getAgentDataplane`, `getAgentDns`, `reportLatencySamples`, `getAgentManagedConnections`, `getAgentManagedDataplane`, `publishManagedTransportCapabilities`, `getAgentConfiguration`, `acknowledgeConfiguration`, `enrollAgent`, `heartbeatAgent`, `acceptAccountInvitation`, `registerInvitedAccount`, `loginAccount`, `logoutAccount`, `listAccountPasswordResets`, `createAccountPasswordReset`, `redeemAccountPasswordReset`, `revokeAccountPasswordReset`, `getAccountPasswordReset`, `getAccountSession`, `revokeAccountSessions`, `exchangeDevice`, `changeAccountPassword`, `listNetworks`, `createNetwork`, `getMeshAccess`, `saveMeshAccess`, `getConnections`, `saveConnections`, `getEndpointDiscoveries`, `getConnectionRuntimes`, `authorizeDevice`, `createEnrollmentToken`, `previewFlow`, `listAccountInvitations`, `createAccountInvitation`, `revokeAccountInvitation`, `getAccountInvitation`, `queryLatencyHistory`, `createManagedEnrollmentToken`, `getManagedListeners`, `saveManagedListener`, `listAccountMembers`, `leaveAccountNetwork`, `saveAccountMember`, `getNetwork`, `saveNetwork`, `listNodeOwnership`, `claimNodeOwnership`, `listNodes`, `setNodeCountry`, `revokeNode`, `setNodeTags`, `getOverview`, `claimFirstOwner`, `getPolicy`, `previewPolicy`, `savePolicy`, `getTags`, `saveTags`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` ErrorResponse.message ` | ` string ` | yes | no | ` — ` |  |

## ExchangeDeviceRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`exchangeDevice`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` ExchangeDeviceRequest.agent_version ` | ` string ` | yes | no | ` {"minLength":1,"maxLength":64,"pattern":"[^\\u0009-\\u000D\\u0020\\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000]"} ` |  |
| ` ExchangeDeviceRequest.code_verifier ` | ` string ` | yes | no | ` {"minLength":43,"maxLength":128,"pattern":"^[A-Za-z0-9._~-]{43,128}$"} ` |  |
| ` ExchangeDeviceRequest.name ` | ` string ` | yes | no | ` {"minLength":1,"maxLength":128,"pattern":"[^\\u0009-\\u000D\\u0020\\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000]"} ` |  |
| ` ExchangeDeviceRequest.node_credential ` | ` string ` | yes | no | ` {"minLength":64,"maxLength":64,"pattern":"^[0-9a-f]{64}$"} ` |  |
| ` ExchangeDeviceRequest.redirect_uri ` | ` string ` | yes | no | ` {"minLength":25,"maxLength":25,"pattern":"^meshway://enroll/callback$"} ` |  |

## FlowAuthorizationAction

类型：` string `；模型可空：no。

模型约束：` {"enum":["allow","reject","invalid"]} `。

枚举：` ["allow", "reject", "invalid"] `。

request 使用：—

response 使用：`previewFlow`

## FlowAuthorizationDecision

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`previewFlow`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` FlowAuthorizationDecision.action ` | ` FlowAuthorizationAction ` | yes | no | ` — ` |  |
| ` FlowAuthorizationDecision.matched_direction ` | ` string \| null ` | yes | yes | ` {"pattern":"^(forward\|reverse)$"} ` |  |
| ` FlowAuthorizationDecision.reason ` | ` string ` | yes | no | ` — ` |  |
| ` FlowAuthorizationDecision.rule_id ` | ` string \| null ` | yes | yes | ` — ` |  |

## FlowQuery

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

Internet country is a preview hint, not automatic geolocation. Mesh country<br>and source country always come from the active-node inventory.

request 使用：`previewFlow`, `previewPolicy`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` FlowQuery.destination_country ` | ` string \| null ` | yes | yes | ` — ` |  |
| ` FlowQuery.destination_ip ` | ` string \| null ` | yes | yes | ` — ` |  |
| ` FlowQuery.destination_node_id ` | ` string \| null ` | yes | yes | ` — ` |  |
| ` FlowQuery.domain ` | ` string \| null ` | yes | yes | ` — ` |  |
| ` FlowQuery.source_node_id ` | ` string ` | yes | no | ` — ` |  |

## GetConnectionRuntimeEpochResponse

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`getConnectionRuntimeEpoch`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` GetConnectionRuntimeEpochResponse.epoch ` | ` integer ` | yes | no | ` {"format":"int64","minimum":0,"maximum":4294967295} ` | Zero means this node has never acquired a runtime lease. |

## HeartbeatRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

Node identity is derived exclusively from the node bearer credential.

request 使用：`heartbeatAgent`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` HeartbeatRequest.agent_version ` | ` string ` | yes | no | ` {"minLength":1,"maxLength":64,"pattern":"[^\\u0009-\\u000D\\u0020\\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000]"} ` |  |

## IncomingConnectionApproval

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`getAgentConnections`, `getAgentDataplane`, `getAgentManagedConnections`, `getAgentManagedDataplane`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` IncomingConnectionApproval.candidate_id ` | ` string ` | yes | no | ` — ` |  |
| ` IncomingConnectionApproval.listener_id ` | ` string ` | yes | no | ` — ` |  |
| ` IncomingConnectionApproval.peer_node_id ` | ` string ` | yes | no | ` — ` |  |
| ` IncomingConnectionApproval.transport ` | ` ConnectionTransport ` | yes | no | ` — ` |  |

## LatencyChannelAssociation

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`reportLatencySamples`

response 使用：`queryLatencyHistory`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` LatencyChannelAssociation.candidate_id ` | ` string \| null ` | yes | yes | ` — ` |  |
| ` LatencyChannelAssociation.initiator_node_id ` | ` string ` | yes | no | ` — ` |  |
| ` LatencyChannelAssociation.listener_id ` | ` string ` | yes | no | ` — ` |  |
| ` LatencyChannelAssociation.transport ` | ` ConnectionTransport ` | yes | no | ` — ` |  |

## LatencyDirection

类型：` string `；模型可空：no。

模型约束：` {"enum":["source_to_peer"]} `。

枚举：` ["source_to_peer"] `。

request 使用：`reportLatencySamples`

response 使用：`queryLatencyHistory`

## LatencyFailureReason

类型：` string `；模型可空：no。

模型约束：` {"enum":["channel_unavailable","authentication_failed","local_send_failed"]} `。

枚举：` ["channel_unavailable", "authentication_failed", "local_send_failed"] `。

request 使用：`reportLatencySamples`

response 使用：`queryLatencyHistory`

## LatencyHistoryRecord

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`queryLatencyHistory`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` LatencyHistoryRecord.cursor ` | ` string ` | yes | no | ` — ` |  |
| ` LatencyHistoryRecord.historical ` | ` boolean ` | yes | no | ` — ` | Always true: a retained measurement never asserts present connectivity. |
| ` LatencyHistoryRecord.network_id ` | ` string ` | yes | no | ` — ` |  |
| ` LatencyHistoryRecord.received_at ` | ` string ` | yes | no | ` {"format":"date-time"} ` |  |
| ` LatencyHistoryRecord.sample ` | ` LatencySample ` | yes | no | ` — ` |  |
| ` LatencyHistoryRecord.source_node_id ` | ` string ` | yes | no | ` — ` |  |

## LatencyHistoryResponse

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`queryLatencyHistory`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` LatencyHistoryResponse.capacity_trimmed ` | ` boolean ` | yes | no | ` — ` | The requested window intersects the conservative capacity-trimmed range. |
| ` LatencyHistoryResponse.capacity_trimmed_before ` | ` string \| null ` | yes | yes | ` {"format":"date-time"} ` | Conservative source-wide capacity watermark (all peers). Some earlier<br>events were evicted; it does not describe offline time or continuous coverage. |
| ` LatencyHistoryResponse.expected_interval_seconds ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |
| ` LatencyHistoryResponse.next ` | ` string \| null ` | yes | yes | ` — ` |  |
| ` LatencyHistoryResponse.records ` | ` array<LatencyHistoryRecord> ` | yes | no | ` — ` |  |
| ` LatencyHistoryResponse.retention_from ` | ` string ` | yes | no | ` {"format":"date-time"} ` | Current TTL floor. This is a maximum age, not a promise of continuous data. |
| ` LatencyHistoryResponse.retention_seconds ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |

## LatencyOutcome

类型：` string `；模型可空：no。

模型约束：` {"enum":["success","timeout","failed"]} `。

枚举：` ["success", "timeout", "failed"] `。

request 使用：`reportLatencySamples`

response 使用：`queryLatencyHistory`

## LatencySample

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`reportLatencySamples`

response 使用：`queryLatencyHistory`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` LatencySample.channel_associations ` | ` array<LatencyChannelAssociation> ` | yes | no | ` {"maxItems":1} ` | Runtime returns exact channel metadata only with successful proof. A<br>timeout leaves this empty rather than racing a separate registry read.<br>Exactly one association for success, zero for timeout. |
| ` LatencySample.completed_at ` | ` string ` | yes | no | ` {"format":"date-time"} ` |  |
| ` LatencySample.digest ` | ` string ` | yes | no | ` {"minLength":64,"maxLength":64,"pattern":"^[0-9a-f]{64}$"} ` |  |
| ` LatencySample.direction ` | ` LatencyDirection ` | yes | no | ` — ` |  |
| ` LatencySample.failure_reasons ` | ` array<LatencyFailureReason> ` | yes | no | ` {"maxItems":1} ` | One sanitized attempt failure for Failed, otherwise empty. This is not<br>a network loss classification; local send failures are kept separate. |
| ` LatencySample.measured_at ` | ` string ` | yes | no | ` {"format":"date-time"} ` | Actual probe invocation start; retained unchanged through queue/retry. |
| ` LatencySample.measurement_id ` | ` string ` | yes | no | ` {"minLength":64,"maxLength":64,"pattern":"^[0-9a-f]{64}$"} ` |  |
| ` LatencySample.outcome ` | ` LatencyOutcome ` | yes | no | ` — ` |  |
| ` LatencySample.peer_node_id ` | ` string ` | yes | no | ` — ` |  |
| ` LatencySample.revision ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |
| ` LatencySample.rtt_us ` | ` integer \| null ` | yes | yes | ` {"format":"int64","minimum":1,"maximum":60000000} ` | Integer microseconds, measured using a monotonic clock, never HTTP RTT. |
| ` LatencySample.runtime_epoch ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |
| ` LatencySample.sampler_generation ` | ` string ` | yes | no | ` {"minLength":64,"maxLength":64,"pattern":"^[0-9a-f]{64}$"} ` | Local sampler generation, not an assertion of physical channel identity. |

## LoginRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`loginAccount`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` LoginRequest.email ` | ` string ` | yes | no | ` {"minLength":3,"maxLength":254} ` |  |
| ` LoginRequest.password ` | ` string ` | yes | no | ` {"maxLength":1024} ` | Controller enforces 12–1024 UTF-8 bytes. JSON Schema string lengths count<br>Unicode code points, so a byte minimum must not be emitted as minLength. |

## LoginResponse

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

Returned only after durable session commit; the Web server keeps it private.

request 使用：—

response 使用：`loginAccount`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` LoginResponse.expires_at ` | ` integer ` | yes | no | ` {"format":"int64"} ` |  |
| ` LoginResponse.session_token ` | ` string ` | yes | no | ` {"minLength":68,"maxLength":68,"pattern":"^mws_[0-9a-f]{64}$"} ` |  |

## ManagedListenerSettings

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`createManagedEnrollmentToken`, `saveManagedListener`

response 使用：`getManagedListeners`, `saveManagedListener`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` ManagedListenerSettings.advertised_endpoints ` | ` array<ConnectionEndpoint> ` | yes | no | ` {"maxItems":16} ` | Explicit announcement, not proof of reachability. Empty means no dial endpoint. |
| ` ManagedListenerSettings.bind_address ` | ` string ` | yes | no | ` — ` |  |
| ` ManagedListenerSettings.bind_port ` | ` integer ` | yes | no | ` {"format":"int32","minimum":1,"maximum":65535} ` |  |
| ` ManagedListenerSettings.enabled ` | ` boolean ` | yes | no | ` — ` |  |
| ` ManagedListenerSettings.id ` | ` string ` | yes | no | ` {"minLength":1,"maxLength":48,"pattern":"^[A-Za-z0-9._-]+$"} ` |  |
| ` ManagedListenerSettings.node_id ` | ` string ` | yes | no | ` — ` |  |
| ` ManagedListenerSettings.transport ` | ` ConnectionTransport ` | yes | no | ` — ` |  |
| ` ManagedListenerSettings.vless ` | ` oneOf(ManagedVlessSettings, null) ` | yes | yes | ` — ` |  |

## ManagedListenerSummary

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`getManagedListeners`, `saveManagedListener`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` ManagedListenerSummary.certificate_sha256 ` | ` string \| null ` | yes | yes | ` — ` |  |
| ` ManagedListenerSummary.compiled_capability_available ` | ` boolean ` | yes | no | ` — ` | False is retained intent awaiting a compatible identity/capability report. |
| ` ManagedListenerSummary.material_version ` | ` integer ` | yes | no | ` {"format":"int32","minimum":1,"maximum":4294967295} ` |  |
| ` ManagedListenerSummary.settings ` | ` ManagedListenerSettings ` | yes | no | ` — ` |  |

## ManagedListenersSnapshot

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`getManagedListeners`, `saveManagedListener`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` ManagedListenersSnapshot.listeners ` | ` array<ManagedListenerSummary> ` | yes | no | ` — ` |  |
| ` ManagedListenersSnapshot.revision ` | ` integer ` | yes | no | ` {"format":"int32","minimum":1,"maximum":4294967295} ` |  |

## ManagedVlessMaterial

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

No Debug: serialization is permitted only on the node-authenticated endpoint.

request 使用：—

response 使用：`getAgentManagedConnections`, `getAgentManagedDataplane`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` ManagedVlessMaterial.certificate_pem ` | ` string ` | yes | no | ` {"minLength":1,"maxLength":32768} ` |  |
| ` ManagedVlessMaterial.certificate_sha256 ` | ` string ` | yes | no | ` — ` |  |
| ` ManagedVlessMaterial.credential_ref ` | ` string ` | yes | no | ` — ` |  |
| ` ManagedVlessMaterial.listener_id ` | ` string ` | yes | no | ` — ` |  |
| ` ManagedVlessMaterial.private_key_pem ` | ` string \| null ` | yes | yes | ` {"format":"password","minLength":1,"maxLength":16384} ` | Only listener owner receives this; dialers receive null. |
| ` ManagedVlessMaterial.settings ` | ` ManagedVlessSettings ` | yes | no | ` — ` |  |
| ` ManagedVlessMaterial.uuid ` | ` string ` | yes | no | ` {"format":"password"} ` |  |
| ` ManagedVlessMaterial.version ` | ` integer ` | yes | no | ` {"format":"int32","minimum":1,"maximum":4294967295} ` |  |

## ManagedVlessSettings

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`createManagedEnrollmentToken`, `saveManagedListener`

response 使用：`getAgentManagedConnections`, `getAgentManagedDataplane`, `getManagedListeners`, `saveManagedListener`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` ManagedVlessSettings.server_name ` | ` string ` | yes | no | ` {"minLength":1,"maxLength":253} ` | Certificate name and client verification name, independent of HTTP Host. |
| ` ManagedVlessSettings.stream ` | ` VlessStreamTransport ` | yes | no | ` — ` |  |
| ` ManagedVlessSettings.websocket_host ` | ` string \| null ` | yes | yes | ` {"minLength":1,"maxLength":253} ` |  |
| ` ManagedVlessSettings.websocket_path ` | ` string \| null ` | yes | yes | ` {"minLength":1,"maxLength":1024} ` |  |

## MeshAccessGrant

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`getAgentDataplane`, `getAgentManagedDataplane`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` MeshAccessGrant.id ` | ` string ` | yes | no | ` {"minLength":1,"maxLength":64,"pattern":"^[A-Za-z0-9._-]+$"} ` |  |
| ` MeshAccessGrant.path ` | ` array<string> ` | yes | no | ` {"minItems":2,"maxItems":8} ` | Exact ordered full path, including source and destination. No repeats. |

## MeshAccessPolicy

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`saveMeshAccess`

response 使用：`getMeshAccess`, `saveMeshAccess`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` MeshAccessPolicy.default_action ` | ` AccessAction ` | yes | no | ` — ` |  |
| ` MeshAccessPolicy.rules ` | ` array<MeshAccessRule> ` | yes | no | ` {"maxItems":128} ` |  |

## MeshAccessRule

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`saveMeshAccess`

response 使用：`getMeshAccess`, `saveMeshAccess`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` MeshAccessRule.action ` | ` AccessAction ` | yes | no | ` — ` |  |
| ` MeshAccessRule.bidirectional ` | ` boolean ` | yes | no | ` — ` |  |
| ` MeshAccessRule.destination ` | ` array<string> ` | yes | no | ` {"minItems":1,"maxItems":32} ` |  |
| ` MeshAccessRule.id ` | ` string ` | yes | no | ` {"minLength":1,"maxLength":64,"pattern":"^[A-Za-z0-9._-]+$"} ` |  |
| ` MeshAccessRule.source ` | ` array<string> ` | yes | no | ` {"minItems":1,"maxItems":32} ` |  |

## MeshAccessSnapshot

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`getMeshAccess`, `saveMeshAccess`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` MeshAccessSnapshot.diagnostics ` | ` array<PolicyDiagnostic> ` | yes | no | ` — ` |  |
| ` MeshAccessSnapshot.policy ` | ` MeshAccessPolicy ` | yes | no | ` — ` | Saved source intent, retained even when references become invalid. |
| ` MeshAccessSnapshot.revision ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |
| ` MeshAccessSnapshot.source_jsonc ` | ` string \| null ` | yes | yes | ` — ` |  |
| ` MeshAccessSnapshot.valid ` | ` boolean ` | yes | no | ` — ` |  |

## MeshDnsConfiguration

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`saveNetwork`

response 使用：`getAgentDns`, `listNetworks`, `createNetwork`, `getNetwork`, `saveNetwork`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` MeshDnsConfiguration.enabled ` | ` boolean ` | yes | no | ` — ` |  |
| ` MeshDnsConfiguration.ttl_seconds ` | ` integer ` | yes | no | ` {"format":"int32","minimum":1,"maximum":300} ` |  |
| ` MeshDnsConfiguration.zone ` | ` string ` | yes | no | ` {"minLength":3,"maxLength":128} ` |  |

## MeshNetwork

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`getAgentDataplane`, `getAgentManagedDataplane`, `getAgentConfiguration`, `listNetworks`, `createNetwork`, `getNetwork`, `saveNetwork`, `getPolicy`, `savePolicy`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` MeshNetwork.ipv4_prefix ` | ` string ` | yes | no | ` — ` |  |
| ` MeshNetwork.ipv6_prefix ` | ` string ` | yes | no | ` — ` |  |

## MeshNode

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

Public active-node inventory; country is administrator-managed metadata.

request 使用：—

response 使用：`getAgentDataplane`, `getAgentManagedDataplane`, `getAgentConfiguration`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` MeshNode.country ` | ` string \| null ` | yes | yes | ` — ` |  |
| ` MeshNode.id ` | ` string ` | yes | no | ` — ` |  |
| ` MeshNode.mesh_ipv4 ` | ` string ` | yes | no | ` — ` |  |
| ` MeshNode.mesh_ipv6 ` | ` string ` | yes | no | ` — ` |  |
| ` MeshNode.name ` | ` string ` | yes | no | ` — ` |  |

## MeshPathAuthorization

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

Internal compiled path authorization. Its JSON remains `{ "grants": \[...\] }`<br>so schema-version-1 NodeDataplane remains byte-contract compatible.

request 使用：—

response 使用：`getAgentDataplane`, `getAgentManagedDataplane`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` MeshPathAuthorization.grants ` | ` array<MeshAccessGrant> ` | yes | no | ` {"maxItems":128} ` |  |

## NetworkList

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`listNetworks`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` NetworkList.networks ` | ` array<NetworkSnapshot> ` | yes | no | ` — ` |  |

## NetworkSnapshot

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

Saved intent at the network configuration revision. Address pools are immutable.

request 使用：—

response 使用：`listNetworks`, `createNetwork`, `getNetwork`, `saveNetwork`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` NetworkSnapshot.dns ` | ` MeshDnsConfiguration ` | yes | no | ` — ` |  |
| ` NetworkSnapshot.name ` | ` string ` | yes | no | ` {"minLength":1,"maxLength":128,"pattern":"[^\\u0009-\\u000D\\u0020\\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000]"} ` |  |
| ` NetworkSnapshot.network ` | ` MeshNetwork ` | yes | no | ` — ` |  |
| ` NetworkSnapshot.network_id ` | ` string ` | yes | no | ` {"minLength":32,"maxLength":32,"pattern":"^[0-9a-f]{32}$"} ` |  |
| ` NetworkSnapshot.revision ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |

## NodeConfiguration

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`getAgentDataplane`, `getAgentManagedDataplane`, `getAgentConfiguration`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` NodeConfiguration.diagnostics ` | ` array<PolicyDiagnostic> ` | yes | no | ` — ` |  |
| ` NodeConfiguration.network ` | ` MeshNetwork ` | yes | no | ` — ` |  |
| ` NodeConfiguration.node ` | ` MeshNode ` | yes | no | ` — ` |  |
| ` NodeConfiguration.peers ` | ` array<MeshNode> ` | yes | no | ` — ` |  |
| ` NodeConfiguration.policy_valid ` | ` boolean ` | yes | no | ` — ` |  |
| ` NodeConfiguration.revision ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |
| ` NodeConfiguration.routing_policy ` | ` RoutingPolicy ` | yes | no | ` — ` |  |
| ` NodeConfiguration.schema_version ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |

## NodeConnections

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`getAgentConnections`, `getAgentDataplane`, `getAgentManagedConnections`, `getAgentManagedDataplane`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` NodeConnections.capabilities ` | ` array<ConnectionCapability> ` | yes | no | ` — ` |  |
| ` NodeConnections.diagnostics ` | ` array<ConnectionDiagnostic> ` | yes | no | ` — ` |  |
| ` NodeConnections.incoming ` | ` array<IncomingConnectionApproval> ` | yes | no | ` — ` |  |
| ` NodeConnections.listeners ` | ` array<ConnectionListener> ` | yes | no | ` — ` |  |
| ` NodeConnections.node_id ` | ` string ` | yes | no | ` — ` |  |
| ` NodeConnections.outgoing ` | ` array<OutgoingConnectionCandidate> ` | yes | no | ` — ` |  |
| ` NodeConnections.peers ` | ` array<ConnectionPeer> ` | yes | no | ` — ` |  |
| ` NodeConnections.public_key ` | ` string \| null ` | yes | yes | ` {"minLength":64,"maxLength":64,"pattern":"^[0-9a-f]{64}$"} ` |  |
| ` NodeConnections.revision ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |
| ` NodeConnections.schema_version ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |
| ` NodeConnections.valid ` | ` boolean ` | yes | no | ` — ` |  |

## NodeDataplane

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

One coherent global revision, projected for the authenticated node.

request 使用：—

response 使用：`getAgentDataplane`, `getAgentManagedDataplane`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` NodeDataplane.access_policy ` | ` MeshPathAuthorization ` | yes | no | ` — ` |  |
| ` NodeDataplane.access_valid ` | ` boolean ` | yes | no | ` — ` |  |
| ` NodeDataplane.configuration ` | ` NodeConfiguration ` | yes | no | ` — ` |  |
| ` NodeDataplane.connections ` | ` NodeConnections ` | yes | no | ` — ` |  |
| ` NodeDataplane.identities ` | ` array<ConnectionPeer> ` | yes | no | ` {"maxItems":64} ` |  |
| ` NodeDataplane.node_id ` | ` string ` | yes | no | ` — ` |  |
| ` NodeDataplane.revision ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |
| ` NodeDataplane.schema_version ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":1} ` |  |

## NodeDnsConfiguration

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`getAgentDns`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` NodeDnsConfiguration.diagnostics ` | ` array<PolicyDiagnostic> ` | yes | no | ` {"maxItems":64} ` |  |
| ` NodeDnsConfiguration.dns ` | ` MeshDnsConfiguration ` | yes | no | ` — ` |  |
| ` NodeDnsConfiguration.network_id ` | ` string ` | yes | no | ` {"minLength":32,"maxLength":32,"pattern":"^[0-9a-f]{32}$"} ` |  |
| ` NodeDnsConfiguration.node_id ` | ` string ` | yes | no | ` {"minLength":64,"maxLength":64,"pattern":"^[0-9a-f]{64}$"} ` |  |
| ` NodeDnsConfiguration.records ` | ` array<NodeDnsRecord> ` | yes | no | ` {"maxItems":1024} ` |  |
| ` NodeDnsConfiguration.revision ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |
| ` NodeDnsConfiguration.schema_version ` | ` integer ` | yes | no | ` {"format":"int32","minimum":1,"maximum":1} ` |  |

## NodeDnsRecord

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`getAgentDns`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` NodeDnsRecord.aliases ` | ` array<string> ` | yes | no | ` {"maxItems":1} ` |  |
| ` NodeDnsRecord.canonical_name ` | ` string ` | yes | no | ` {"minLength":1,"maxLength":253,"pattern":"^id\\.[0-9a-f]{32}\\.[0-9a-f]{32}\\.[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?(?:\\.[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?)+$"} ` |  |
| ` NodeDnsRecord.mesh_ipv4 ` | ` string ` | yes | no | ` — ` |  |
| ` NodeDnsRecord.mesh_ipv6 ` | ` string ` | yes | no | ` — ` |  |
| ` NodeDnsRecord.node_id ` | ` string ` | yes | no | ` {"minLength":64,"maxLength":64,"pattern":"^[0-9a-f]{64}$"} ` |  |

## NodeEndpointDiscovery

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`getEndpointDiscoveries`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` NodeEndpointDiscovery.current ` | ` boolean ` | yes | no | ` — ` |  |
| ` NodeEndpointDiscovery.digest ` | ` string \| null ` | yes | yes | ` {"minLength":64,"maxLength":64,"pattern":"^[0-9a-f]{64}$"} ` |  |
| ` NodeEndpointDiscovery.epoch ` | ` integer ` | yes | no | ` {"format":"int64","minimum":0,"maximum":4294967295} ` |  |
| ` NodeEndpointDiscovery.node_id ` | ` string ` | yes | no | ` — ` |  |
| ` NodeEndpointDiscovery.reported_at ` | ` string \| null ` | yes | yes | ` {"format":"date-time"} ` |  |
| ` NodeEndpointDiscovery.revision ` | ` integer \| null ` | yes | yes | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |
| ` NodeEndpointDiscovery.sequence ` | ` integer ` | yes | no | ` {"format":"int64","minimum":0,"maximum":4294967295} ` |  |
| ` NodeEndpointDiscovery.sockets ` | ` array<UdpSocketDiscovery> ` | yes | no | ` {"maxItems":128} ` |  |
| ` NodeEndpointDiscovery.stale ` | ` boolean ` | yes | no | ` — ` |  |
| ` NodeEndpointDiscovery.state ` | ` EndpointDiscoveryState ` | yes | no | ` — ` |  |

## NodeList

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`listNodes`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` NodeList.nodes ` | ` array<NodeSummary> ` | yes | no | ` — ` |  |

## NodeOwnership

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`listNodeOwnership`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` NodeOwnership.membership_id ` | ` string \| null ` | yes | yes | ` — ` | Null means legacy-unassigned; tags remain ordinary node attributes. |
| ` NodeOwnership.network_id ` | ` string ` | yes | no | ` — ` |  |
| ` NodeOwnership.node_id ` | ` string ` | yes | no | ` — ` |  |

## NodeOwnershipList

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`listNodeOwnership`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` NodeOwnershipList.nodes ` | ` array<NodeOwnership> ` | yes | no | ` — ` |  |

## NodeSummary

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

Durable control-plane identity. Last contact is not tunnel health.

request 使用：—

response 使用：`enrollAgent`, `heartbeatAgent`, `exchangeDevice`, `getConnections`, `saveConnections`, `listNodes`, `setNodeCountry`, `revokeNode`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` NodeSummary.acknowledged_revision ` | ` integer ` | yes | no | ` {"format":"int64","minimum":0,"maximum":4294967295} ` |  |
| ` NodeSummary.agent_version ` | ` string ` | yes | no | ` — ` |  |
| ` NodeSummary.configuration_revision ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |
| ` NodeSummary.country ` | ` string \| null ` | yes | yes | ` — ` |  |
| ` NodeSummary.id ` | ` string ` | yes | no | ` — ` |  |
| ` NodeSummary.last_seen ` | ` string ` | yes | no | ` {"format":"date-time"} ` |  |
| ` NodeSummary.mesh_ipv4 ` | ` string ` | yes | no | ` — ` |  |
| ` NodeSummary.mesh_ipv6 ` | ` string ` | yes | no | ` — ` |  |
| ` NodeSummary.name ` | ` string ` | yes | no | ` — ` |  |
| ` NodeSummary.registered_at ` | ` string ` | yes | no | ` {"format":"date-time"} ` |  |
| ` NodeSummary.revoked_at ` | ` string \| null ` | yes | yes | ` {"format":"date-time"} ` |  |

## NodeTagAssignment

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`setNodeTags`, `getTags`, `saveTags`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` NodeTagAssignment.node_id ` | ` string ` | yes | no | ` — ` |  |
| ` NodeTagAssignment.tags ` | ` array<string> ` | yes | no | ` {"maxItems":32} ` |  |

## OutgoingConnectionCandidate

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`getAgentConnections`, `getAgentDataplane`, `getAgentManagedConnections`, `getAgentManagedDataplane`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` OutgoingConnectionCandidate.credential_ref ` | ` string \| null ` | yes | yes | ` — ` |  |
| ` OutgoingConnectionCandidate.endpoint ` | ` ConnectionEndpoint ` | yes | no | ` — ` |  |
| ` OutgoingConnectionCandidate.id ` | ` string ` | yes | no | ` — ` |  |
| ` OutgoingConnectionCandidate.listener_id ` | ` string ` | yes | no | ` — ` |  |
| ` OutgoingConnectionCandidate.peer_node_id ` | ` string ` | yes | no | ` — ` |  |
| ` OutgoingConnectionCandidate.priority ` | ` integer ` | yes | no | ` {"format":"int32","minimum":0,"maximum":65535} ` |  |
| ` OutgoingConnectionCandidate.transport ` | ` ConnectionTransport ` | yes | no | ` — ` |  |

## Overview

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`getOverview`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` Overview.configuration_revision ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |
| ` Overview.name ` | ` string ` | yes | no | ` — ` |  |
| ` Overview.node_count ` | ` integer ` | yes | no | ` {"format":"int64","minimum":0,"maximum":4294967295} ` |  |
| ` Overview.version ` | ` string ` | yes | no | ` — ` |  |

## PolicyDiagnostic

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`getAgentDataplane`, `getAgentDns`, `getAgentManagedDataplane`, `getAgentConfiguration`, `getMeshAccess`, `saveMeshAccess`, `previewFlow`, `getPolicy`, `previewPolicy`, `savePolicy`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` PolicyDiagnostic.field ` | ` string ` | yes | no | ` — ` |  |
| ` PolicyDiagnostic.message ` | ` string ` | yes | no | ` — ` |  |
| ` PolicyDiagnostic.rule_id ` | ` string \| null ` | yes | yes | ` — ` |  |

## PolicySnapshot

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`getPolicy`, `savePolicy`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` PolicySnapshot.diagnostics ` | ` array<PolicyDiagnostic> ` | yes | no | ` — ` |  |
| ` PolicySnapshot.network ` | ` MeshNetwork ` | yes | no | ` — ` |  |
| ` PolicySnapshot.policy ` | ` RoutingPolicy ` | yes | no | ` — ` |  |
| ` PolicySnapshot.revision ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |
| ` PolicySnapshot.source_jsonc ` | ` string \| null ` | yes | yes | ` — ` |  |
| ` PolicySnapshot.valid ` | ` boolean ` | yes | no | ` — ` |  |

## PreviewFlowRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`previewFlow`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` PreviewFlowRequest.flow ` | ` FlowQuery ` | yes | no | ` — ` |  |

## PreviewFlowResponse

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`previewFlow`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` PreviewFlowResponse.authorization ` | ` FlowAuthorizationDecision ` | yes | no | ` — ` |  |
| ` PreviewFlowResponse.diagnostics ` | ` array<PolicyDiagnostic> ` | yes | no | ` — ` |  |
| ` PreviewFlowResponse.execution_reason ` | ` string ` | yes | no | ` — ` |  |
| ` PreviewFlowResponse.execution_supported ` | ` boolean ` | yes | no | ` — ` |  |
| ` PreviewFlowResponse.revision ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |
| ` PreviewFlowResponse.routing ` | ` RouteDecision ` | yes | no | ` — ` |  |
| ` PreviewFlowResponse.valid ` | ` boolean ` | yes | no | ` — ` |  |

## PreviewPolicyRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`previewPolicy`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` PreviewPolicyRequest.flow ` | ` FlowQuery ` | yes | no | ` — ` |  |
| ` PreviewPolicyRequest.policy ` | ` RoutingPolicy ` | yes | no | ` — ` |  |

## PreviewPolicyResponse

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`previewPolicy`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` PreviewPolicyResponse.decision ` | ` RouteDecision ` | yes | no | ` — ` |  |
| ` PreviewPolicyResponse.diagnostics ` | ` array<PolicyDiagnostic> ` | yes | no | ` — ` |  |
| ` PreviewPolicyResponse.revision ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |
| ` PreviewPolicyResponse.valid ` | ` boolean ` | yes | no | ` — ` |  |

## PublishConnectionIdentityRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`publishConnectionIdentity`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` PublishConnectionIdentityRequest.capabilities ` | ` array<ConnectionCapability> ` | yes | no | ` {"maxItems":3} ` |  |
| ` PublishConnectionIdentityRequest.public_key ` | ` string ` | yes | no | ` {"minLength":64,"maxLength":64,"pattern":"^[0-9a-f]{64}$"} ` |  |

## PublishConnectionIdentityResponse

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`publishConnectionIdentity`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` PublishConnectionIdentityResponse.capabilities ` | ` array<ConnectionCapability> ` | yes | no | ` — ` |  |
| ` PublishConnectionIdentityResponse.node_id ` | ` string ` | yes | no | ` — ` |  |
| ` PublishConnectionIdentityResponse.public_key ` | ` string ` | yes | no | ` {"minLength":64,"maxLength":64,"pattern":"^[0-9a-f]{64}$"} ` |  |
| ` PublishConnectionIdentityResponse.revision ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |

## PublishManagedTransportCapabilitiesRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`publishManagedTransportCapabilities`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` PublishManagedTransportCapabilitiesRequest.vless_streams ` | ` array<VlessStreamTransport> ` | yes | no | ` {"maxItems":2} ` |  |

## PublishManagedTransportCapabilitiesResponse

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`publishManagedTransportCapabilities`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` PublishManagedTransportCapabilitiesResponse.revision ` | ` integer ` | yes | no | ` {"format":"int32","minimum":1,"maximum":4294967295} ` |  |

## QueryLatencyHistoryRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

Raw events for one directed pair. Exclusive row cursor, not an offset.

request 使用：`queryLatencyHistory`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` QueryLatencyHistoryRequest.after ` | ` string \| null ` | yes | yes | ` {"minLength":1,"maxLength":20,"pattern":"^[0-9]+$"} ` |  |
| ` QueryLatencyHistoryRequest.from ` | ` string ` | yes | no | ` {"format":"date-time"} ` |  |
| ` QueryLatencyHistoryRequest.limit ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":1000} ` |  |
| ` QueryLatencyHistoryRequest.peer_node_id ` | ` string ` | yes | no | ` — ` |  |
| ` QueryLatencyHistoryRequest.source_node_id ` | ` string ` | yes | no | ` — ` |  |
| ` QueryLatencyHistoryRequest.until ` | ` string ` | yes | no | ` {"format":"date-time"} ` |  |

## RedeemPasswordResetRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`redeemAccountPasswordReset`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` RedeemPasswordResetRequest.new_password ` | ` string ` | yes | no | ` — ` |  |
| ` RedeemPasswordResetRequest.reset_token ` | ` string ` | yes | no | ` {"minLength":68,"maxLength":68,"pattern":"^mwr_[0-9a-f]{64}$"} ` |  |

## RegisterInvitationRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`registerInvitedAccount`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` RegisterInvitationRequest.email ` | ` string ` | yes | no | ` — ` |  |
| ` RegisterInvitationRequest.invitation_token ` | ` string ` | yes | no | ` {"minLength":68,"maxLength":68,"pattern":"^mwi_[0-9a-f]{64}$"} ` |  |
| ` RegisterInvitationRequest.password ` | ` string ` | yes | no | ` — ` |  |

## ReportConnectionRuntimeRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`reportConnectionRuntime`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` ReportConnectionRuntimeRequest.digest ` | ` string ` | yes | no | ` {"minLength":64,"maxLength":64,"pattern":"^[0-9a-f]{64}$"} ` |  |
| ` ReportConnectionRuntimeRequest.epoch ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |
| ` ReportConnectionRuntimeRequest.listeners ` | ` array<ConnectionListenerObservation> ` | yes | no | ` {"maxItems":128} ` |  |
| ` ReportConnectionRuntimeRequest.peers ` | ` array<ConnectionNeighborObservation> ` | yes | no | ` {"maxItems":256} ` |  |
| ` ReportConnectionRuntimeRequest.reason ` | ` ConnectionRuntimeReason ` | yes | no | ` — ` |  |
| ` ReportConnectionRuntimeRequest.revision ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |
| ` ReportConnectionRuntimeRequest.sequence ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |
| ` ReportConnectionRuntimeRequest.state ` | ` ConnectionRuntimeState ` | yes | no | ` — ` |  |

## ReportConnectionRuntimeResponse

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`reportConnectionRuntime`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` ReportConnectionRuntimeResponse.epoch ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |
| ` ReportConnectionRuntimeResponse.reported_at ` | ` string ` | yes | no | ` {"format":"date-time"} ` |  |
| ` ReportConnectionRuntimeResponse.sequence ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |

## ReportEndpointDiscoveryRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`reportEndpointDiscovery`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` ReportEndpointDiscoveryRequest.digest ` | ` string ` | yes | no | ` {"minLength":64,"maxLength":64,"pattern":"^[0-9a-f]{64}$"} ` |  |
| ` ReportEndpointDiscoveryRequest.epoch ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` | Reuses the authenticated managed runtime's acquired epoch, not a new identity. |
| ` ReportEndpointDiscoveryRequest.revision ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |
| ` ReportEndpointDiscoveryRequest.sequence ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` | Independent monotonic discovery report sequence within that epoch. |
| ` ReportEndpointDiscoveryRequest.sockets ` | ` array<UdpSocketDiscovery> ` | yes | no | ` {"maxItems":128} ` |  |
| ` ReportEndpointDiscoveryRequest.state ` | ` EndpointDiscoveryState ` | yes | no | ` — ` |  |

## ReportEndpointDiscoveryResponse

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`reportEndpointDiscovery`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` ReportEndpointDiscoveryResponse.epoch ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |
| ` ReportEndpointDiscoveryResponse.reported_at ` | ` string ` | yes | no | ` {"format":"date-time"} ` |  |
| ` ReportEndpointDiscoveryResponse.sequence ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |

## ReportLatencySamplesRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`reportLatencySamples`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` ReportLatencySamplesRequest.samples ` | ` array<LatencySample> ` | yes | no | ` {"minItems":1,"maxItems":64} ` |  |

## ReportLatencySamplesResponse

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`reportLatencySamples`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` ReportLatencySamplesResponse.duplicates ` | ` integer ` | yes | no | ` {"format":"int64","minimum":0,"maximum":64} ` |  |
| ` ReportLatencySamplesResponse.inserted ` | ` integer ` | yes | no | ` {"format":"int64","minimum":0,"maximum":64} ` |  |

## RevokeAccountSessionsRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`revokeAccountSessions`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` RevokeAccountSessionsRequest.user_id ` | ` string ` | yes | no | ` — ` |  |

## RevokeInvitationRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`revokeAccountInvitation`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` RevokeInvitationRequest.invitation_id ` | ` string ` | yes | no | ` — ` |  |

## RevokeNodeRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`revokeNode`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` RevokeNodeRequest.node_id ` | ` string ` | yes | no | ` {"minLength":1,"maxLength":128} ` |  |

## RevokeNodeResponse

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`setNodeCountry`, `revokeNode`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` RevokeNodeResponse.node ` | ` NodeSummary ` | yes | no | ` — ` |  |

## RevokePasswordResetRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`revokeAccountPasswordReset`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` RevokePasswordResetRequest.reset_id ` | ` string ` | yes | no | ` — ` |  |

## RouteAction

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`previewPolicy`, `savePolicy`

response 使用：`getAgentDataplane`, `getAgentManagedDataplane`, `getAgentConfiguration`, `getPolicy`, `savePolicy`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` RouteAction.exit_node_id ` | ` string \| null ` | yes | yes | ` — ` |  |
| ` RouteAction.kind ` | ` RouteActionKind ` | yes | no | ` — ` |  |
| ` RouteAction.via ` | ` array<string> ` | yes | no | ` — ` |  |

## RouteActionKind

类型：` string `；模型可空：no。

模型约束：` {"enum":["route","reject"]} `。

枚举：` ["route", "reject"] `。

request 使用：`previewPolicy`, `savePolicy`

response 使用：`getAgentDataplane`, `getAgentManagedDataplane`, `getAgentConfiguration`, `getPolicy`, `savePolicy`

## RouteDecision

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`previewFlow`, `previewPolicy`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` RouteDecision.action ` | ` RouteDecisionAction ` | yes | no | ` — ` |  |
| ` RouteDecision.exit_node_id ` | ` string \| null ` | yes | yes | ` — ` |  |
| ` RouteDecision.path ` | ` array<string> ` | yes | no | ` — ` |  |
| ` RouteDecision.reason ` | ` string ` | yes | no | ` — ` |  |
| ` RouteDecision.rule_id ` | ` string \| null ` | yes | yes | ` — ` |  |

## RouteDecisionAction

类型：` string `；模型可空：no。

模型约束：` {"enum":["route","reject","invalid"]} `。

枚举：` ["route", "reject", "invalid"] `。

request 使用：—

response 使用：`previewFlow`, `previewPolicy`

## RouteRule

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`previewPolicy`, `savePolicy`

response 使用：`getAgentDataplane`, `getAgentManagedDataplane`, `getAgentConfiguration`, `getPolicy`, `savePolicy`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` RouteRule.action ` | ` RouteAction ` | yes | no | ` — ` |  |
| ` RouteRule.destination_any ` | ` array<DestinationSelector> ` | yes | no | ` — ` |  |
| ` RouteRule.destination_scope ` | ` DestinationScope ` | yes | no | ` — ` |  |
| ` RouteRule.id ` | ` string ` | yes | no | ` — ` |  |
| ` RouteRule.source ` | ` SourceSelector ` | yes | no | ` — ` |  |

## RoutingPolicy

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`previewPolicy`, `savePolicy`

response 使用：`getAgentDataplane`, `getAgentManagedDataplane`, `getAgentConfiguration`, `getPolicy`, `savePolicy`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` RoutingPolicy.default_action ` | ` DefaultRouteAction ` | yes | no | ` — ` |  |
| ` RoutingPolicy.rules ` | ` array<RouteRule> ` | yes | no | ` — ` |  |

## SaveConnectionsRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`saveConnections`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` SaveConnectionsRequest.configuration ` | ` ConnectionPlan ` | yes | no | ` — ` |  |
| ` SaveConnectionsRequest.expected_revision ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |

## SaveManagedListenerRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`saveManagedListener`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` SaveManagedListenerRequest.expected_revision ` | ` integer ` | yes | no | ` {"format":"int32","minimum":1,"maximum":4294967295} ` |  |
| ` SaveManagedListenerRequest.settings ` | ` ManagedListenerSettings ` | yes | no | ` — ` |  |

## SaveMembershipRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`saveAccountMember`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` SaveMembershipRequest.role ` | ` AccountMembershipChange ` | yes | no | ` — ` |  |
| ` SaveMembershipRequest.user_id ` | ` string ` | yes | no | ` — ` |  |

## SaveMeshAccessRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`saveMeshAccess`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` SaveMeshAccessRequest.expected_revision ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |
| ` SaveMeshAccessRequest.policy ` | ` MeshAccessPolicy ` | yes | no | ` — ` |  |
| ` SaveMeshAccessRequest.source_jsonc ` | ` string \| null ` | no | yes | ` — ` |  |

## SaveNetworkRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`saveNetwork`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` SaveNetworkRequest.dns ` | ` MeshDnsConfiguration ` | yes | no | ` — ` |  |
| ` SaveNetworkRequest.expected_revision ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |
| ` SaveNetworkRequest.name ` | ` string ` | yes | no | ` {"minLength":1,"maxLength":128,"pattern":"[^\\u0009-\\u000D\\u0020\\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000]"} ` |  |

## SavePolicyRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`savePolicy`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` SavePolicyRequest.expected_revision ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |
| ` SavePolicyRequest.policy ` | ` RoutingPolicy ` | yes | no | ` — ` |  |
| ` SavePolicyRequest.source_jsonc ` | ` string \| null ` | no | yes | ` — ` |  |

## SaveTagsRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`saveTags`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` SaveTagsRequest.expected_revision ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |
| ` SaveTagsRequest.tags ` | ` array<TagDefinition> ` | yes | no | ` {"maxItems":64} ` |  |

## SetNodeCountryRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`setNodeCountry`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` SetNodeCountryRequest.country ` | ` string \| null ` | yes | yes | ` — ` |  |
| ` SetNodeCountryRequest.expected_revision ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |
| ` SetNodeCountryRequest.node_id ` | ` string ` | yes | no | ` — ` |  |

## SetNodeTagsRequest

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`setNodeTags`

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` SetNodeTagsRequest.expected_revision ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |
| ` SetNodeTagsRequest.node_id ` | ` string ` | yes | no | ` — ` |  |
| ` SetNodeTagsRequest.tags ` | ` array<string> ` | yes | no | ` {"maxItems":32} ` |  |

## SourceSelector

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

Empty dimensions are wildcards; dimensions combine with AND, members with OR.

request 使用：`previewPolicy`, `savePolicy`

response 使用：`getAgentDataplane`, `getAgentManagedDataplane`, `getAgentConfiguration`, `getPolicy`, `savePolicy`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` SourceSelector.countries ` | ` array<string> ` | yes | no | ` — ` |  |
| ` SourceSelector.node_ids ` | ` array<string> ` | yes | no | ` — ` |  |

## StunDiscoveryOutcome

类型：` string `；模型可空：no。

模型约束：` {"enum":["mapped","timeout","invalid_response","io_error","stun_error"]} `。

枚举：` ["mapped", "timeout", "invalid_response", "io_error", "stun_error"] `。

request 使用：`reportEndpointDiscovery`

response 使用：`getEndpointDiscoveries`

## StunEndpointObservation

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`reportEndpointDiscovery`

response 使用：`getEndpointDiscoveries`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` StunEndpointObservation.expires_at ` | ` string ` | yes | no | ` {"format":"date-time"} ` |  |
| ` StunEndpointObservation.mapped_endpoint ` | ` string \| null ` | yes | yes | ` {"minLength":3,"maxLength":64} ` |  |
| ` StunEndpointObservation.observed_at ` | ` string ` | yes | no | ` {"format":"date-time"} ` | Repeated reports preserve these times; retransmission is not renewal. |
| ` StunEndpointObservation.observer ` | ` string ` | yes | no | ` {"minLength":3,"maxLength":64} ` | Explicit numeric STUN observer endpoint, not a Meshway peer identity. |
| ` StunEndpointObservation.outcome ` | ` StunDiscoveryOutcome ` | yes | no | ` — ` |  |
| ` StunEndpointObservation.rtt_ms ` | ` integer \| null ` | yes | yes | ` {"format":"int64","minimum":0,"maximum":4294967295} ` |  |

## TagDefinition

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`saveTags`

response 使用：`setNodeTags`, `getTags`, `saveTags`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` TagDefinition.description ` | ` string ` | yes | no | ` {"maxLength":256} ` |  |
| ` TagDefinition.tag ` | ` string ` | yes | no | ` {"pattern":"^tag:[a-z0-9][a-z0-9_-]{0,47}$"} ` |  |

## TagSnapshot

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：`setNodeTags`, `getTags`, `saveTags`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` TagSnapshot.assignments ` | ` array<NodeTagAssignment> ` | yes | no | ` — ` |  |
| ` TagSnapshot.revision ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |
| ` TagSnapshot.tags ` | ` array<TagDefinition> ` | yes | no | ` {"maxItems":64} ` |  |

## UdpDiscoveryResourceKind

类型：` string `；模型可空：no。

模型约束：` {"enum":["listener","outgoing_candidate"]} `。

枚举：` ["listener", "outgoing_candidate"] `。

request 使用：`reportEndpointDiscovery`

response 使用：`getEndpointDiscoveries`

## UdpSocketDiscovery

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：`reportEndpointDiscovery`

response 使用：`getEndpointDiscoveries`

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` UdpSocketDiscovery.bound_address ` | ` string ` | yes | no | ` {"minLength":3,"maxLength":64} ` |  |
| ` UdpSocketDiscovery.generation ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` | Public process-local runtime generation and socket ordinal. |
| ` UdpSocketDiscovery.observations ` | ` array<StunEndpointObservation> ` | yes | no | ` {"maxItems":4} ` |  |
| ` UdpSocketDiscovery.resource_id ` | ` string ` | yes | no | ` {"minLength":1,"maxLength":64,"pattern":"^[A-Za-z0-9._-]+$"} ` | Controller listener/candidate ID, never a private runtime token. |
| ` UdpSocketDiscovery.resource_kind ` | ` UdpDiscoveryResourceKind ` | yes | no | ` — ` |  |
| ` UdpSocketDiscovery.socket_id ` | ` integer ` | yes | no | ` {"format":"int64","minimum":1,"maximum":4294967295} ` |  |
| ` UdpSocketDiscovery.source ` | ` EndpointDiscoverySource ` | yes | no | ` — ` |  |

## VlessStreamTransport

类型：` string `；模型可空：no。

模型约束：` {"enum":["tcp","websocket"]} `。

枚举：` ["tcp", "websocket"] `。

request 使用：`publishManagedTransportCapabilities`, `createManagedEnrollmentToken`, `saveManagedListener`

response 使用：`getAgentManagedConnections`, `getAgentManagedDataplane`, `getManagedListeners`, `saveManagedListener`
