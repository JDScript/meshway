# HTTP、事件与生成客户端

下载 [OpenAPI 契约（JSON）](/v2/contracts/openapi.json) 查看完整操作、参数和类型。该文件由 Rust 类型与实际路由生成，本站在构建／预览时原样发布；Rust／TypeScript SDK 使用同一来源。本文定义共同语义，可用范围见 [Server](/v2/server)。

## 两类调用者

| 根路径 | 调用者与凭据 | 职责 |
| --- | --- | --- |
| `/api/v1` | Console／用户 API 客户端，不透明用户会话 | 账户、网络管理、配置、获准查询与诊断 |
| `/agent-api/v1` | 接入中的设备或已认证 Node | 持钥接入、自身实例、执行目标、需求与报告 |

两个根路径分别承载用户与 Node 合同，没有首用户认领入口。

## 功能参考

| 主题 | 合同 |
| --- | --- |
| 注册、会话、加入网络、设备批准、换钥 | [认证与接入](/v2/api/authentication) |
| JSONC、typed ops、只读预览、严格 CAS | [编辑与应用对照](/v2/configuration-reference/editing) |
| 执行目标、full／delta、实例、lease、回执 | [配置应用](/v2/configuration-reference/deployment) |
| 配置版本、恢复、活动、分页 | [版本与操作历史](/v2/api/history) |
| State report／query、可见范围、新鲜度 | [State Reference](/v2/state-reference) |
| 首次同步、实体增量、cursor／reset | [SSE](/v2/api/events) |
| 诊断任务、续租、结果与历史 | [Realtime Observation](/v2/state-reference/realtime) |
| schema、承载目录、数据集 | [Transport](/v2/configuration-reference/transports) · [Rule Sets](/v2/configuration-reference/rule-sets) |
| 操作提示与错误 | [Actions 与 Problem](/v2/api/problems) |

## 共同规则

- 用户管理 mutation 逐请求及最终事务检查身份和网络权限；Node 凭据只代表其绑定节点，不替代用户管理凭据。
- 创建操作用 requestId 幂等；相同 ID 不同输入是 `request_id_conflict`。回滚、材料及身份生命周期仍须按当前条件重验。
- 配置写入使用 expectedRevision；成员版本等使用各自并发基准。pointer 没变不代表允许自动 rebase 提交。
- 对象拒绝未知／重复字段和数组表示。字段用 camelCase；协议 key 与错误 code 保留其明确定义的词汇。
- 可省略、明确 null、未知、过期和不适用是不同语义。时间为 UTC RFC 3339，整数范围须可被生成客户端无损表示。
- 空状态保持真实。逻辑计划、saved、applied、认证 link 与实际业务成功分别表达。

## 查询、秘密与缓存

普通 Config／Node settings 使用非秘密视图；整份 `configuration/source` 是显式秘密读取，不进通用缓存或 LocalStorage。表单秘密使用保持／覆盖语义，JSONC 编辑整份文档。

单节点 settings 返回 networkId、nodeId、revision 与共享 settings 片段。库存中的 desiredExecution 是期望引用；实际回执和 State 独立。普通成员的本人读取不扩展为整网配置历史或原创建 Tag 设备的管理权限。

列表采用有界 limit／不透明 cursor，绑定身份、网络、过滤和读取上界；每页重新鉴权。total 按相同范围计算，不能用已加载数量或隐藏网络的全局数量代替。State 的新鲜度依原报告期限计算，不因重复读取刷新。

## SDK

Rust 操作直接复用 `meshway-protocol` 的公开 DTO。TypeScript 对成功和错误响应执行生成的运行时校验；不能以类型断言代替解码。未知／畸形响应不回显原始秘密内容。

JSON 与显式声明的 SSE 由同一路由导出。SSE 使用 `x-meshway-sse` 指向具体事件根类型；接收事件不自动推进调用者的已应用 cursor。用法和具体函数签名见 `packages/control-client/README.md`。

生成／检查入口是 `mise run generate:protocol` 与 `mise run check:protocol`，实现位于 `scripts/protocol/`。修改合同同时重生成，不直接编辑产物；Server Rust 构建不依赖 Console 构建。
