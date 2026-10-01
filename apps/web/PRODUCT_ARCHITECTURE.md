# Console product architecture · 2026-10-01

## 审查与页面职责

| 页面 | 用户任务 | 本轮发现 / 收敛 |
| --- | --- | --- |
| 概览 | 找待处理事项，进入节点或策略 | 移除无数据的吞吐/丢包占位墙；保留真实联系、回执和可行动事项；控制面地址/版本放高级详情 |
| 节点列表 | 找到节点、筛选联系/回执、进入动作 | ID 从默认重复行收进既有复制菜单/详情；保留搜索、筛选与 NodeAction deep links；联系不包装成在线 |
| 节点详情 | 查地址、管理标注/撤销、配置连接 | 身份/构建/回执详情复用；高级启用帮助替代默认命令卡；连接观测优先，端点与手工配置可展开 |
| 网络 | 改名称、DNS、查只读网段 | 保存意图和 CAS 基准仍可见；本地 DNS 启用/诊断帮助折叠，不挪动草稿或基准 |
| Access / routing | 编辑有序规则、解决校验/冲突、保存 | 已保存 flow 统一交给拓扑；routing 草稿预览留在编辑器。JSONC 文本/注释、导航保护、规则语义保留 |
| 连接 | 网络级查配置并进入节点处理 | 保留旧 /connections；连接身份与节点详情复用；保留手工监听/候选编辑，明确 profile 只是当前实现引用 |
| 拓扑 | 看保存意图、上报邻接、检查已保存 flow | 保留三个独立图层、来源/失效标记和节点动作入口；不增加第二个 flow 工作台 |
| 诊断（/monitoring） | 按问题找下一步 | 已保存 flow 链接到拓扑；运行状态链接到节点；本地命令按问题折叠，避免复制观测详情 |

## 分层与复用

- 默认：用户选择的地址/规则/监听、已保存意图、最后联系、回执状态、观测来源/有效性、失败原因与下一步。
- 高级：原始 ID、公钥/构建能力、revision/digest/epoch/sequence/generation/socket、current/stale、启动模板与本地 profile 引用。
- `PageBody` 固定桌面/移动 gutter 24/16；复用 section header、status summary、diagnostic details、failure notice；沿用 shadcn，不另起设计系统。
- 高级详情使用原生键盘可展开的 details，内容持续挂载。折叠不是时钟/租约边界；过期地址继续禁用复制，来源与 TTL 始终可见。
- 编辑器不因观测刷新、折叠或切换页面移动 CAS 基准；错误/冲突、无效 JSONC 与未应用表单草稿仍由原 draft store 保护。

## 当前边界与产品目标

已确认目标：首次接入后协议配置、凭据、监听及变更由平台管理下发，日常 Web 完成。V2Ray / Shadowsocks 等仅作为 Meshway 内部承载；未来终端由自研 Meshway App 接入，普通代理客户端入口、代理账户和订阅不属于当前目标。默认根据访问授权、路由必经约束与可达性按需自动建连，Web 可手动约束，不要求用户预先逐对配置。主入口围绕节点、访问和路径。

统一 Node 模型：节点没有服务器 / App / 中继资格等固有产品类别，不增加角色开关。路径经过谁谁中转；实际能力/可达性由内部处理，授权和必经约束有效。路径的源/中间/目标只描述本次路径上下文。

当前 managed-secret pipeline 与自动组织邻接均未交付，VLESS 等仍引用本地 profiles；本轮把现有监听/候选编辑收进节点高级手工连接区（有保留草稿或无效保存配置时展开），旧 /connections 保留网络级入口。保持当前可用操作，不显示「自动已运行」或没有后端动作的保存/部署按钮。

## 验证与交接

严格 TS、lint、现有行为 unit、production build；新增 console-product e2e 验证默认折叠/键盘展开、过期复制资格、节点 deep link、390/1440 布局、成功保存不自动收起反馈、无效 JSONC/注释导航保留与 CAS 冲突。主线程整体运行依赖隔离 controller 的现有 e2e；旧端点/profile/DNS/数据面断言需先展开对应高级区，不能通过取消过期检查修测试。

本轮验证：`pnpm --filter @meshway/web check`、`test`（126 项）、`build`、`pnpm typecheck:e2e` 通过；隔离 fixture 下 `console-product.spec.ts`、原 `access.spec.ts` 与 `routing.spec.ts` 全通过。可设置 `MESHWAY_PRODUCT_SCREENSHOTS=1` 留概览/节点的桌面与移动审查图。首次临时 symlink 的 Turbopack 构建受 filesystem root 限制，改用 worktree 自有 frozen-lockfile 安装后正常生产构建；未修改锁文件。

旧断言迁移（主线程负责）：
- `endpoint-discovery.spec.ts`：节点页先展开「UDP 地址观测与当前启用帮助」；generation/socket 前展开「承载 socket 技术详情」，无报告启动参数前展开「当前 STUN 启用帮助」。监控 #netcheck 保持直达观测。
- `connections.spec.ts` / `control-plane.spec.ts`：节点监听/候选按钮前展开「高级手工连接配置」；公钥复制前展开「连接身份与回执详情」；profile 输入在「高级」中，空必填值/错误会自动展开。
- `network.spec.ts` / `dns-guidance.spec.ts`：DNS 命令前展开「DNS 诊断与当前启用帮助」，`#dns-diagnostics` deep link 自动展开。
- 数据面启动模板前展开「当前节点启用帮助」；节点身份 ID 前展开「节点身份技术详情」。「配置连接」概览按钮改为「查看连接」。
- `connectivity-guidance.spec.ts`：问题区仍用 `summary`，来源 badge 在展开内容；若定位原 summary 的来源文字需改为标题定位。
