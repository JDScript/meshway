# Pure SPA 的 HTTP 接口审查

代码基线 `2132434`，覆盖全部 67 个操作；未读取独立 `apps/console` 的活跃实现。
本文结合专项静态审查、补充纠正和协调者核对；没有进行浏览器测试。
完整定义见[模型 / schema 导出](/model-api-audit)，逐操作分类见[67 项使用与轮询表](/contract-audit/2132434/spa-usage)。

## 已确认的迁移基线

已核对 2026-10-02 console 线程的用户确认记录：

- Vite / TanStack Router / Query 的 pure SPA，不保留 BFF。
- 使用现有不透明 `mws_` 用户会话，不改 JWT；LocalStorage 持久化，标签页捕获到内存后使用 Authorization。
- 实例管理员令牌仅放内存，继续与普通用户 / 网络角色分开。
- storage 变化时锁定 / 重载；缓存和草稿按登录上下文隔离，接口仍由 Controller 检查实际权限。
- 同源部署，由 Controller 从运行时目录提供静态 dist；Rust 构建 / 测试不依赖前端构建。

这些是目标决策，静态资源托管尚不是冻结 Controller 的已实现功能。Cookie 会话只是初审比较过的备选，
本方案无需为了替代 Next 而先引入 Cookie / CSRF 会话体系。Bearer-only 本身没有 Cookie 型的环境凭据 CSRF；
这也不免除 XSS 预防、凭据保密、真实服务端权限和客户端登录上下文隔离。

## 主要结论与具体缺口

### 1. 会话承接：保留已确认的 Bearer 方式，补齐操作上下文

Controller 的 `bearer()`（`apps/controller/src/lib.rs:334–353`）只读取 Authorization，
当前不解析浏览器 Cookie。旧 Next 的 sign-in marker 和分次 Cookie 清理不自动适用于 SPA。
这是一项迁移责任，不能据此宣称 Bearer-only 已存在可利用的 Cookie 漏洞。

SPA 每个请求使用渲染 / 草稿所属登录的凭据快照；不能在发送或重试时重新读 LocalStorage 借用新登录。
查询 cache key 至少区分登录上下文、network ID、operation 和参数；令牌本身不放进可展示的 cache key / 日志。
旧请求返回后还要核对上下文，不能用其成功、401 或 logout 结果清除新登录或覆盖新 cache。
持久化登录的跨 tab 清理也须定义并发协议；一次“读取后比较再删除”不是跨 tab 的原子操作。

`AccountSession`（`crates/protocol/src/accounts.rs:52–55`）目前只有 user / memberships，
缺少当前会话期限和明确的服务端时间；期限只在 `LoginResponse.expires_at` 返回。
可在有版本的会话元数据操作中补齐这些信息；公开 session reference 可以辅助客户端关联，
但不替代真正的 Bearer 认证，也不是当前方案必须重新发明的另一类权限。
改密码 / reset / 撤销全部会话会使旧会话失效，`{saved}` 的返回未显式描述此效果，文档和客户端必须正确处理。

bootstrap 本来就是实例级权限，用它操作网络不等于网络 Owner 越权。
现有 `/setup` 与兼容入口继续保留，实例凭据不进入日常持久会话；CLI-only 初始化或 setup grant 属于未来提案。

### 2. 状态读取需有时间边界，配置版本不能代表新鲜度

运行报告与发现响应按服务端 `now` 派生 current / stale / expired：
`store/connection_runtime.rs`、`store/endpoint_discovery.rs:244–299`。
没有新 sequence 或配置 revision 时，报告也会变旧，单笔 STUN 观测也会过期或被过滤。

建议动态读取提供明确的服务端时间、当前源 revision，以及逐部分观测时间 / 有效期。
`last_seen` 只表示最后控制面联系，不能改名为隧道在线；ACK、应用自报告与真实探测仍分开。
字段或 metadata header 的演进须考虑下述严格解码兼容边界。

现有 NodeSummary / ConnectionsSnapshot 把身份、源配置和动态联系 / ACK 放在一起。
SPA 应将配置编辑基准与刷新中的观测副本分开，后续可提供轻量 status read model，
减少为刷新一个状态而下载整份源策略、JSONC 或材料摘要。

### 3. 多次独立读取不构成同一快照，轮询成本也需控制

旧节点 / 拓扑每约 12 秒触发读取，内部并发 listNodes、getConnections、getConnectionRuntimes、
getManagedListeners；它们各自有事务，原始结果可能来自不同 revision / epoch。
已有 UI 的交叉校验值得保留，这不等于已证明它把混合结果当成真实连通。

Controller 仍通过全局 `Mutex<Store>`，管理读入口也执行 `BEGIN IMMEDIATE`
（`lib.rs:125–143`、`store/account_authorization.rs:72–95`）。发现读取还会计算连接 / runtime 投影。
建议先约束可见视图轮询、合并同一用途读取和减少重复 payload；需要时增加同事务的状态摘要操作。
不机械改成 DEFERRED：它不会解除全局 mutex 串行，历史读取目前还执行 DELETE 清理。
数据库并发优化须保留鉴权时钟、事务快照和撤回边界，再用实际负载判断。

### 4. 历史 cursor 只能直接用于固定测量窗口分页

`store/latency_history/mod.rs:249` 同时使用：

```text
measured_ms ∈ [from, until)
row_id > after
ORDER BY row_id
```

row ID 按接收顺序，测量时间允许迟到 120 秒或超前 5 秒。若上次窗口排除了一条“未来测量”，
却返回了 row ID 更大的其他记录，把该 cursor 带进下次扩大的窗口会永久跳过它。
协调者用相同 SQL 过滤关系的合成两行探针确认了这一结果；这是谓词证明，不是现网测量或 HTTP 漏洞复现。
现有 UI 每次重新读取完整窗口不使用这种跨窗口 cursor，代价是重复传输。

**现 API 可采用的读取策略**：

1. 固定一次请求窗口，分页直到完整；同一窗口内才沿用 `after`。
2. 有了可信服务端时间后，以最后一轮完整成功的锚点 `S_prev` 为基准，下轮重读
   `[S_prev - 130s, S_now)`，不带上一窗口 cursor，按事件 cursor / identity 去重。
3. 本轮固定窗口有多页时全部读完才推进锚点；失败、分页截断、容量淘汰要明确保留缺口。
4. 长时间离线、超出 24h 单窗口或保留范围时，重新建立需要显示的窗口，不能盲推进旧锚点。

浏览器时间快超过 5 秒可能让首次查询直接 422；该失败响应没有 retention 元数据，不能靠它反推时钟。
可使用经过确认的可信 HTTP Date 估计，或新增服务端时间 / 相对窗口请求合同。
后续若增加 tail / received-cursor 模式，应定义高水位和完整性，不能只把字段改名。

### 5. 错误与重试需要机器语义

当前 `ErrorResponse` 只有 message；409 复用配置冲突 / 绑定冲突文案，账户操作也可能落入这一分类。
部分账户路由声明的错误状态少于实际 JSON rejection、权限和存储错误。
建议补充稳定 error code、有界字段问题、适用时的 current revision / Retry-After，客户端按 status 先判定，
细节缺失或解码失败仍保持原失败状态，不回显未经验证的任意 body。

TS SDK 当前丢弃错误 body。Rust 生成客户端在 `client.rs:514–523` 已提前返回非 2xx，
后面残留的类型化 ErrorResponse 解析模板不会在这条路径执行；不能仅凭模板存在就认为 Agent 正依赖它。
应保持节点端 status-first，不能为了丰富错误而延迟 401/403 等撤回信号。
SDK 同时缺少外部 AbortSignal，SPA 的 query cancellation 不能真正中断当前 fetch，可在传输合同中补齐。

重试按具体语义决定：

- GET 与只读 POST 查询可在同一身份 / 输入 / 窗口内有界重试。
- 人类侧创建一次性秘密、注册邀请、reset、创建网络等不盲目重试；区分提交失败与响应丢失。
- 节点 enroll / exchange 保留精确原绑定的幂等恢复，不应用上述人类侧禁重试规则。
- 资源撤销的幂等性不取消当前鉴权；CAS 冲突需重读核对内容。
- `claimNodeOwnership` 的 409 可能是已经归属另一个 membership，不能直接当作成功。

### 6. 创建操作和成员更新的恢复 / 并发合同仍有缺口

接入令牌没有列表 / 撤销操作，响应丢失后难以定位与撤销已创建的有效令牌。
邀请 / reset 已有元数据列表和撤销，但缺少将丢失响应与一次创建关联的 request ID。
`createNetwork` 不以名称去重，也没有请求幂等键，自动重试可能创建合法但非预期的重复网络。
可增加限于主体 / 操作 / 网络范围的 client request ID、请求摘要与公有资源 ID；不要为恢复重发已撤销身份或任意重发秘密。

`SaveMembershipRequest` 以 user_id 定位目标（`accounts.rs:144–147`），缺少渲染时 membership ID /
角色版本的比较。后续可防止离开再加入后的新 membership 被旧页面操作，及多个管理员的陈旧覆盖。
这属于并发意图改进，现有 Controller 仍检查执行时的角色上限与最后 Owner。

### 7. 成功响应新增字段也需要迁移边界

TS `z.strictObject` 与 Rust `deny_unknown_fields` 会拒绝未知成功字段。
session metadata、server time、status envelope 等新增字段不能笼统按“加字段兼容”处理。
使用新操作 / 版本、显式协商或协调所有消费者升级；新 header 对旧 DTO 的影响则应单独核对。
当前错误 body 被旧 SDK 丢弃，其演进兼容性与成功响应不同。

`policy v3` 是策略 / 执行版本，不自动意味着所有 HTTP URL 都要改成 `/api/v3`。
现有 GET / POST、统一 200 成功、save / revoke 命名可继续工作；避免只为形式重命名所有 App / Agent 依赖路径。
节点操作出现在 SDK 主导出是包入口归属问题，调用仍受 Node / enrollment 凭据检查，不等于向浏览器授予权限。

## 轮询建议摘要

频率为初始建议，需随可见性、有效期与真实负载调整；全部 67 个操作的分类见[逐操作表](/contract-audit/2132434/spa-usage)。

| 数据 | 典型操作 | 建议 |
| --- | --- | --- |
| 身份、网络与配置 | getAccountSession、listNetworks、getNetwork、getPolicy、getMeshAccess、getTags、成员 / 邀请列表 | 导航 / 聚焦 / 相关 mutation 后读取；期限在客户端正确老化 |
| 节点与配置混合列表 | listNodes、getConnections、getManagedListeners | 按需加载基准；可见状态副本约 15–30s，不能更新草稿 base |
| 运行 / 发现状态 | getConnectionRuntimes、getEndpointDiscoveries | 相关面板可见时约 10–15s；未知和过期不当零值或健康 |
| 延迟历史 | queryLatencyHistory | 按需；可见曲线约 30–60s，遵循固定窗口分页和重叠读取规则 |
| 编译预览 | previewPolicy、previewFlow | 用户触发或编辑防抖，不定时轮询 |
| mutation / 一次性凭据 | save、revoke、create、login、authorizeDevice 等 | 不轮询；重试遵循具体合同 |

同一 query 单次在途、后台暂停、失败有界退避、旧登录结果隔离即可作为首版起点。
SSE / WebSocket、跨 tab polling leader 与 ETag 是后续选择；动态 ETag 不能仅使用 source revision / sequence，
因为时间流逝也会改变表示。HTTP 缓存策略不能替代客户端按登录 / 网络隔离的数据管理。

## Controller 静态托管的边界

按已确认同源方案，Controller 原有 API 继续直接处理 Bearer；无需新增宽泛 CORS 来使同源 SPA 工作。
开发环境可由 console 工作流配置同源代理；若以后真的需要跨源，再明确允许 origin / method / headers。
反代后的来源限流也需区分实际 peer 与可信转发信息，不无条件相信任意 X-Forwarded-For。

运行时静态目录只用于 assets / SPA 导航。`/api/v1/**` 的未知路径和方法应保持 API 错误，
缺失 JS/CSS 资源不能返回伪装成成功的 index.html。API `no-store`、HTML 更新策略与带 hash 静态资源缓存分别处理。
保留 App 的 exchange、heartbeat、configuration、DNS 四个原 API 路径及 `/authorize-device`、邀请 / reset 等页面路由；
静态托管不收集 App 密码、不向节点 API 注入管理员凭据。

## 实施顺序

1. 固定已确认 Bearer / 同源部署和客户端登录上下文约束；为新 console 提供明确操作分类。
2. 结合字段专项，先补契约中的已有边界 / 错误声明；对成功 DTO 的变化选择升级方式。
3. 实现会话 / 时间元数据、错误细节、取消与创建恢复等必要合同，不以大规模 URL 改名阻塞组网。
4. 根据真实负载再拆轻量 status read model / 读连接和后台清理；保护已有事务与租约时序。

以上接口改进尚未实施。原报告和补充记录保留于批准 TMP 的
`meshway-spa-api-audit-2132434.md` / `-addendum.md`；协调者另有固定 SQL 过滤关系的
`model-api-audit-export/history-cursor-proof.json`，不冒充实际网络采样。
