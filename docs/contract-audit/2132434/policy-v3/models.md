# 模型与字段完整目录

生成快照：`2132434-policy-source-not-http`；输入 SHA-256：`db83f47418e200520c1a1207abbadf34c7cae8bb3bbe4cef4b30dfe69ab294b4`。

required 表示在所属对象 / 分支中必须出现；nullable 表示 JSON 字面量 null 是否被 schema 接受。
不是字段使用建议或完整业务校验；分支内字段的 required 不表示所有分支都携带该字段。

12 schemas，45 个对象字段；2 个可省略，0 个可空。

## AutomaticRouting

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` AutomaticRouting.fallback ` | ` string ` | yes | no | ` — ` |  |
| ` AutomaticRouting.regions ` | ` map<string, string> ` | yes | no | ` {"additionalProperties":{"type":"string"},"propertyNames":{"type":"string"}} ` |  |

## ConnectionAction

类型：` string `；模型可空：no。

模型约束：` {"enum":["allow","block"]} `。

枚举：` ["allow", "block"] `。

request 使用：—

response 使用：—

## ConnectionRule

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` ConnectionRule.a ` | ` string ` | yes | no | ` — ` |  |
| ` ConnectionRule.action ` | ` ConnectionAction ` | yes | no | ` — ` |  |
| ` ConnectionRule.b ` | ` string ` | yes | no | ` — ` |  |
| ` ConnectionRule.id ` | ` string ` | yes | no | ` — ` |  |

## ExitMatch

类型：` oneOf(object, object, object) `；模型可空：no。

request 使用：—

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` ExitMatch/oneOf/0.kind ` | ` string ` | yes | no | ` {"enum":["domain_suffix"]} ` |  |
| ` ExitMatch/oneOf/0.value ` | ` string ` | yes | no | ` — ` |  |
| ` ExitMatch/oneOf/1.kind ` | ` string ` | yes | no | ` {"enum":["cidr"]} ` |  |
| ` ExitMatch/oneOf/1.value ` | ` string ` | yes | no | ` — ` |  |
| ` ExitMatch/oneOf/2.kind ` | ` string ` | yes | no | ` {"enum":["rule_set"]} ` |  |
| ` ExitMatch/oneOf/2.value ` | ` string ` | yes | no | ` — ` |  |

## ExitRule

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` ExitRule.candidates ` | ` array<string> ` | yes | no | ` — ` |  |
| ` ExitRule.destinations ` | ` array<ExitMatch> ` | yes | no | ` — ` |  |
| ` ExitRule.id ` | ` string ` | yes | no | ` — ` |  |
| ` ExitRule.src ` | ` string ` | no | no | ` — ` |  |
| ` ExitRule.target ` | ` ExitTarget ` | yes | no | ` — ` |  |
| ` ExitRule.via ` | ` array<string> ` | yes | no | ` — ` |  |

## ExitTarget

类型：` string `；模型可空：no。

模型约束：` {"enum":["nodes","direct"]} `。

枚举：` ["nodes", "direct"] `。

request 使用：—

response 使用：—

## Grant

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` Grant.bidirectional ` | ` boolean ` | yes | no | ` — ` |  |
| ` Grant.dst ` | ` array<string> ` | yes | no | ` — ` |  |
| ` Grant.id ` | ` string ` | yes | no | ` — ` |  |
| ` Grant.src ` | ` array<string> ` | yes | no | ` — ` |  |

## MeshRule

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` MeshRule.dst ` | ` string ` | yes | no | ` — ` |  |
| ` MeshRule.id ` | ` string ` | yes | no | ` — ` |  |
| ` MeshRule.src ` | ` string ` | yes | no | ` — ` |  |
| ` MeshRule.via ` | ` array<string> ` | yes | no | ` — ` |  |

## NetworkPolicy

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` NetworkPolicy.assignments ` | ` array<ProfileAssignment> ` | yes | no | ` — ` |  |
| ` NetworkPolicy.automatic ` | ` AutomaticRouting ` | no | no | ` — ` |  |
| ` NetworkPolicy.connections ` | ` array<ConnectionRule> ` | yes | no | ` — ` |  |
| ` NetworkPolicy.grants ` | ` array<Grant> ` | yes | no | ` — ` |  |
| ` NetworkPolicy.mesh ` | ` array<MeshRule> ` | yes | no | ` — ` |  |
| ` NetworkPolicy.profiles ` | ` array<RoutingProfile> ` | yes | no | ` — ` |  |
| ` NetworkPolicy.public_rules ` | ` array<ExitRule> ` | yes | no | ` — ` |  |

## ProfileAssignment

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` ProfileAssignment.allowed ` | ` array<string> ` | yes | no | ` — ` |  |
| ` ProfileAssignment.default_profile ` | ` string ` | yes | no | ` — ` |  |
| ` ProfileAssignment.locked ` | ` boolean ` | yes | no | ` — ` |  |
| ` ProfileAssignment.src ` | ` string ` | yes | no | ` — ` |  |

## ProfileFallback

类型：` oneOf(object, object, object) `；模型可空：no。

request 使用：—

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` ProfileFallback/oneOf/0.kind ` | ` string ` | yes | no | ` {"enum":["none"]} ` |  |
| ` ProfileFallback/oneOf/1.kind ` | ` string ` | yes | no | ` {"enum":["direct"]} ` |  |
| ` ProfileFallback/oneOf/2.candidates ` | ` array<string> ` | yes | no | ` — ` |  |
| ` ProfileFallback/oneOf/2.kind ` | ` string ` | yes | no | ` {"enum":["nodes"]} ` |  |
| ` ProfileFallback/oneOf/2.via ` | ` array<string> ` | yes | no | ` — ` |  |

## RoutingProfile

类型：` object `；模型可空：no。

模型约束：` {"additionalProperties":false} `。

request 使用：—

response 使用：—

| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ` RoutingProfile.fallback ` | ` ProfileFallback ` | yes | no | ` — ` |  |
| ` RoutingProfile.id ` | ` string ` | yes | no | ` — ` |  |
| ` RoutingProfile.rules ` | ` array<ExitRule> ` | yes | no | ` — ` |  |
