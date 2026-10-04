# 整网草稿与应用对照

每个用户编辑上下文、每张网络一份配置草稿，跨配置页面保留，最终一次提交。Server 接受完整文档或强类型操作，始终校验完整目标并执行严格 CAS。

## 草稿

| 字段 | 含义 |
| --- | --- |
| identity／networkId | 草稿所属用户会话上下文和网络，切换身份时隔离 |
| baseRevision | 编辑基准，不因 SSE 更新偷偷推进 |
| sourceJsonc | JSONC 原文，包括注释、无效文本和未保存编辑 |
| ops | 表单产生的强类型操作；仅在与当前文本一致时使用 |

这是一份用户自己的编辑工作区，不是所有管理员共同修改的服务器草稿。图形化和 JSONC 视图共享编辑结果；无法解析的文本仍保留，不能用旧 ops 重建并覆盖它。

公开摘要可进入普通查询缓存；含秘密的完整配置／JSONC 只在编辑上下文内存中保存，不放通用 query cache 或 LocalStorage。跨页面保留不等于跨刷新持久保存秘密；涉及退出／重载时保留草稿提示与身份隔离。

JSONC 提交整份文档；表单通过 ops 轻量编辑，秘密字段默认“保持”或写新值。日常表单不读取旧秘密，完整秘密读取仅经 configuration/source。

账户、成员、加入链接、接入密钥、诊断任务等有独立生命周期的动作不加入配置草稿。显式节点删除可独立提交，也可以由最终配置差异表达；它推进配置 revision，其他未保存草稿仍按严格 CAS 处理。

## 两种编辑输入

同一个 `POST /api/v1/networks/{networkId}/configuration` 接受明确的判别输入。以下为完整请求结构，示例 Node 必须已在目标网络注册，revision 必须来自该网络当前基准：

<!-- schema: ConfigurationRequest -->
```json
{
  "type": "operations",
  "expectedRevision": 42,
  "requestId": "00000000-0000-4000-8000-000000000001",
  "ops": [
    { "type": "setNodeSettings", "nodeId": "00000000-0000-4000-8000-000000000002", "fields": { "hostname": "web" } }
  ]
}
```

JSONC 输入使用 `type: "document"`、expectedRevision、requestId、sourceJsonc。恢复历史配置使用[历史恢复输入](/v2/api/history)，最终仍走普通 apply。

| 操作 | 类型化参数 |
| --- | --- |
| `replaceAccess` | AccessRule[] |
| `replaceTopology` | TopologyRule[] |
| `replaceRouting` | RoutingConfiguration |
| `replaceDns` | DnsConfiguration |
| `replaceDefaults` | DefaultsUpdate |
| `replaceGroups` | groups：邮箱数组的 map |
| `replaceTagsOwners` | tagsOwners：Tag owner 数组的 map |
| `replaceRuleSets` | ruleSets：RuleSet 的 map |
| `setNodeSettings` | NodeId＋允许字段的 NodeSettingsPatch |
| `setListener` | NodeId＋ListenerKey＋按协议判别的 ListenerUpdate 或 null |
| `setIdentityTags` | nodeId＋tags 字符串数组，仍检查 Tag owner／身份转换规则 |
| `setAddressPools` | AddressPoolsInput |
| `setAddresses` | NodeId＋各族地址覆盖，省略／null 按地址合同 |
| `deleteNode` | NodeId；提交时永久删除 |

没有任意 `replace(pointer, JSON)` 操作。NodeSettingsPatch 仅含 hostname、remark、enabled、exitEnabled、dialers 等明确定义字段；不借通用 fields 改身份、凭据或另一张网络。新增 Node 仍由接入流程创建。

ListenerUpdate 使用明确的协议字段与更新语义：已有秘密省略为保持，显式值为修改；首次创建按该协议规则补全／要求必要输入。服务端合成完整 Listener 再校验，不能要求表单提交它未读取的旧私钥，也不能把脱敏占位当值。

普通字段编辑不自动永久吊销旧材料。独立吊销记录中的材料不能通过回滚或复制到另一配置恢复有效。

表单中的 `DefaultsUpdate.dialers` 与 `NodeSettingsPatch.dialers` 使用 `DialerSetUpdate`：协议集合显式 `{}` 表示禁用；保留的 VLESS 项内，`websocket` 省略保持此前 WebSocket 参数，显式 `null` 移除；`websocket.headers` 省略保持旧值，显式 `{}` 清空，显式非空对象替换整个 header 集合。这些是表单更新语义；整份 JSONC 仍提交完整文档值。提供的 TLS 证书／私钥同样允许在原 listener 资源内省略保持；删除再创建不继承旧材料。

NodeSettingsPatch.dialers 省略保持当前覆盖，显式 null 移除节点覆盖并恢复网络默认集合。它与完整配置中的 dialers 省略继承、显式 `{}` 禁用是不同层的输入语义。

`replaceDefaults` 的 `DefaultsUpdate` 按明确字段修改：`access`、`topology`、`dialers` 均可省略，省略保持基准值，显式非 null 值覆盖。只改 topology 的操作不携带旧 dialers；用户选择新 revision 重检同一操作时，只重放明确修改的字段。服务端仍严格 CAS，不自动重基准提交，也不把新 Config 覆盖用户草稿。

每个操作独立检查权限，整批操作原子提交；Tag owner 的 setIdentityTags 不能与越权整网操作混合执行。服务端把操作应用到当前基准的 JSONC，保留未涉及部分的注释。替换整个区段时须明确展示对应文本变化，不能静默丢弃注释。

## 应用前必须看到什么

Console 在最终应用前展示**有效修改数量、前后对照和影响范围**。`POST /configuration/validate` 接受同样的草稿输入，返回非提交预览：

| 字段 | 含义 |
| --- | --- |
| `requestId` | 此次请求关联 |
| `baseRevision` | 实际比较的基准 |
| `preview` | 共享类型的规范化 ConfigurationView；脱敏、未提交，供基准视图与目标对照 |
| `changeCount` | 最终有效变化项数，不是点击次数或 ops 数量 |
| `changes` | 字段路径／变化类型的完整摘要 |
| `sourceChanged` | 是否还有原文／注释变化 |
| `deletedNodeIds` | 提交即永久删除的节点，单独醒目标明 |
| `addressChangedNodeIds` | 预计改址、会中断旧连接的节点 |
| `preparedRuleSets` | 按 key 返回此次校验暂定的规则集 digest／entries；不预留 ArtifactId |
| `unavailableDependencies` | 失效依赖；保留旧引用不等于恢复授权，也不表示运行时链路健康状态 |

`unavailableDependencies` 列出保留但已撤销／失效的成员与路由依赖。这类依赖不阻止撤权保存，执行侧必须拒绝受影响流量。稳定绑定本身变化也进入 `changes`（策略引用 `/bindings/users`、Node 身份解析 `/bindings/nodeUsers`、`/bindings/listeners`、`/bindings/ruleSets`）并计入 `changeCount`；原样文档／ops 保存和新设备接入不会把已失效的策略成员引用重新绑定到同名的新入网关系。两类用户绑定的具体边界见[配置版本存储](/v2/storage/database)。

计数对同一基准下的最终差异去重：改同一字段多次只计最终变化；改回原值不计；新增／删除整个对象时不再重复计算其所有子字段。first-match 数组重排作为该有序区段变化，完整对照显示顺序。changeCount 与返回的完整变化摘要一致；超出预算明确报限制，不隐瞒剩余变化。

表单的前后对照使用同一基准的非秘密 ConfigurationView 与 validate.preview；秘密字段只显示“保持／修改／将生成”。Node 的设置 GET 也是非秘密视图。完整秘密读取只经 configuration/source 的整份 JSONC 编辑入口。仅注释变化时单独显示，不把它宣传成节点业务配置变化。

预览不落盘、不分配永久材料、不预留地址、不创建待激活版本。材料准备要求不能作为成功预览的真实材料；应用时重新准备并检查当前权限、依赖和 CAS。提交后读取同 revision 的配置取得最终值，预览失败保留草稿。

规范化由 Rust 配置库完成，Console 不重复实现默认／继承／地址算法。缺少必要材料时返回 `configuration_invalid` 的 requirements，不能以伪造材料构造成功预览。preview 不写入当前配置／State 缓存，与当前 ConfigurationView 共用字段类型，但 envelope 明确本次预览基准及其非提交性质。

## 冲突

**保持严格 CAS。** expectedRevision 过旧返回 409 `revision_conflict`，不因为操作写入的 pointer 没变就自动提交。客户端重新读取基准并验证草稿；错误本身不包含完整差异。

同一 `requestId` 使用不同请求体返回 `request_id_conflict`；基准过旧返回 `revision_conflict`，两者含义独立。没有节点目标的普通 apply 返回 `targetCount: 0`、`publicationIds: []`；资源绑定仍可在内部保存对应的完整快照。

例如给 Node 换 Tag 时，别人的 access／groups 修改可能改变结果；仅检查 `/nodes/.../identity` 无法证明无冲突。defaults、地址池和路由顺序也存在读取依赖。

可提供辅助重放／合并草稿，但必须重新完整验证、显示新的修改数量与对照，由用户再次应用。原文本、注释、无效编辑和原基准不能被覆盖。SSE changedPointers 只作提示／缓存失效，不作为自动重放安全证明。

注册新节点、同步邮箱引用、删除节点等实际改变配置的系统操作同样产生新 revision，使用 cause 说明原因。它们不会让草稿丢失，只会让旧基准无法直接提交；Console 可带同一组 ops 在新基准重新 validate，由用户再次应用。纯 State／观测不产生管理员配置版本。

## 删除

删除 Node 是永久动作：最终目标移除已有 nodes[id]，或提交 deleteNode／删除接口，均执行凭据吊销＋配置项移除＋旧地址退出记录＋相关执行撤回的同一事务。应用前对照明确列出删除项。

没有“仅移除配置但继续保留可接入设备”的普通操作，也没有节点回收站／恢复。历史配置不能恢复已删除 Node；重新接入创建新 Node 和密钥。后端保留最小吊销／历史引用依据，不等于软删除产品流程。

管理员显式节点引用不会被自动改成直连或更宽规则；引用已删除节点的业务明确不可用。删除不能因为它曾是必经点而恢复凭据或绕过该点。
