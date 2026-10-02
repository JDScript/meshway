# SPA 的会话、查询与配置消费

`apps/console` 是用户另一工作流维护的 pure SPA；本文保留与后端对接的合同，不描述其活跃实现完成状态。当前精确 HTTP 操作/字段来自[生成契约](/protocol)，按需[完整导出](/model-api-audit)。

## 已确认的登录与部署方式

- 使用不透明 `mws_` 用户会话，LocalStorage 持久化，标签页捕获到内存后以 Authorization 调生成 SDK。storage 变化时锁定/重载，旧身份的请求、缓存和草稿隔离。
- 实例 bootstrap 令牌只放内存，日常用户/网络角色不继承实例权限；App/CLI 不取得密码、浏览器会话或管理员凭据。
- 无 BFF，Controller 从运行时配置目录同源提供 dist；Rust 构建/测试不依赖前端构建。静态托管属于独立 console 接线目标，不据此宣称后端已部署。
- SPA fallback 不吞 `/api/v1/**` 的路径/方法错误，也不把缺失 JS/CSS 返回为 index.html；保留 App API、`/authorize-device` 和邀请/reset 页面路由。API no-store、HTML 与 hash assets 缓存分别处理。

## 身份、网络与草稿/CAS

请求使用渲染/草稿所属登录凭据快照，发送或重试不重新借用 LocalStorage 中新登录。cache key 区分登录上下文、network ID、operation/参数，token 不进入展示 key/日志；旧请求返回后再次核对上下文，不能清除新登录或覆盖新 cache。

Controller 每请求/事务检查真实身份、过期、credential version 与网络权限；客户端选网不授予权限。编辑源和动态观测副本分开，轮询不更新未保存草稿/CAS base。图形化与 JSONC 共用同份草稿和基准，无效文本、注释与未保存编辑不能因切换视图丢失；409 保留草稿并显式解决冲突。

## 状态与轮询

配置 revision 不代表新鲜度；runtime epoch/sequence 与 reported/received/expires 时间分别处理。没有新 sequence 时状态也会 stale/expired。多个独立 GET 不是同事务快照，混合 revision/epoch 不可画成已连通证据。心跳不是隧道在线，ACK 不是 applied，规划路径不是 trace。

| 数据 | 消费方式 |
| --- | --- |
| session、网络、配置源 | 导航/聚焦/相关 mutation 后读取；保存期限真实老化 |
| 节点与配置混合列表 | 按需加载；可见状态约 15–30s，编辑 base 不跟刷新推进 |
| runtime / discovery | 相关面板可见时约 10–15s；unknown/stale/expired 不补健康或零 |
| latency history | 可见曲线约 30–60s，固定窗口分页；重复读取不制造测量 |
| 编译 preview | 用户触发或防抖；不定时轮询，不用于真实包流证明 |
| mutation / 一次性秘密 | 不轮询；重试按操作合同 |

频率是初始建议而非现行服务端承诺；单 query 单次在途、后台暂停、有界退避，避免刷新整份源/材料。节点执行下载带私有材料且可能发布能力，不能给 SPA 当状态 GET。

## 历史分页

现有 history 以 `measured_ms ∈ [from, until)`、`row_id > after` 并按 row ID 排序；接收顺序不等于测量时间顺序。**cursor 仅在固定窗口内连续分页**，不能带进扩大后的窗口，否则可能永久漏掉先前被窗口排除的迟到/未来测量。

同一窗口全部读完才推进读取锚点；下一窗口不沿用旧 cursor，按 measurement identity 去重。若有可信服务端时间，可从上次完整成功锚点前重叠读取（当前允许迟到120s/超前5s，建议留130s），失败、截断、容量淘汰或长期离线明确展示缺口。客户端时钟和 SQL 毫秒精度不能冒充更精确服务端时间。[邻接历史](/latency-history)说明保留范围与测量来源。

## 错误、重试与待改进项

现有 SDK 保留失败 status，丢弃错误 body。401/403 及时处理，未知或解码失败仍保持失败；不回显任意服务器文本。GET/只读查询可在同一身份/输入/窗口内有界重试；创建网络、邀请/reset/接入秘密不盲重试，响应丢失与未提交分开。节点 enroll/exchange 的精确身份幂等恢复仍有效，撤销不能复活。claim 的409不能当作成功。

以下为尚需按当前源核对/演进的接口事项，不是已实现的新版合同：明确 session/server-time 元数据、稳定 error code/机器问题、SDK AbortSignal、创建恢复 request ID、membership 并发版本与轻量同事务状态视图。当前 strict 成功 DTO 对新增字段敏感，须协调升级或新操作；不为形式统一改 App/Agent URL，也不通过忽略未知字段规避兼容。

历史专项证据入口见[导出说明](/model-api-audit)。后端仍须保护 CAS、权限事务与 lease 时序；前端界面由独立工作流实现并手工验证。
