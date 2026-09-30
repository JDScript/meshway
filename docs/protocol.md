# 协议与类型安全

**已确认的要求：接口自动生成并强制类型安全。** 当前实现选择 Rust 类型与实际路由关联的 OpenAPI 作为 HTTP 契约来源。Agent 使用生成的 Rust 客户端，Next.js 使用生成的 TypeScript SDK 及运行时验证。不能分别手写三份“看起来相同”的协议。

## 契约生成链

```text
Rust 请求 / 响应类型 + 实际 API 路由声明
                    │
                    ▼
                 OpenAPI
                /       \
               ▼         ▼
      Rust control client  TypeScript SDK + 运行时 schema
               │                         │
               ▼                         ▼
           Rust Agent                 Next.js Web
```

路由声明关联 OpenAPI 很重要：一个与运行中服务无关的手工文档，即使可以生成客户端，也无法证明 endpoint、状态码和响应一致。

`crates/protocol` 承载共享 Rust 契约基础。当前生成位置为：

- `packages/protocol/openapi.json`：从 controller 实际路由导出的契约。
- `apps/agent/src/generated`：Agent 的 Rust control client。
- `packages/control-client/src/generated`：TypeScript 类型、客户端与 Zod schema。

生成工具与路径由 `scripts/generate-protocol.mjs` 统一维护；修改源类型 / 路由后再生成，不直接编辑生成文件。

### 当前生成器与支持范围

- Utoipa 6 与 utoipa-axum 0.3 导出原生 OpenAPI 3.1。
- `openapi-to-rust` 0.19 生成 Agent 当前使用的 `reportAgent` 操作与 serde 模型。
- Hey API 0.99 生成 TypeScript DTO 与 Zod schema；小型 OpenAPI 驱动的操作生成器负责调用经过校验的 HTTP transport。
- 当前 TS 操作生成支持 JSON GET / POST、组件引用 schema、必填 POST body 与单一 HTTP 200 成功响应。新增不支持的契约形状会让生成失败，需要先扩展生成器。

Rust 响应通过 serde 校验声明的字段类型与时间格式，Web 通过 Zod 校验。当前响应模型允许忽略额外字段；请求的非空及长度约束仍由 controller 执行。Zod 的字符串长度以 UTF-16 计数，controller 以 Unicode scalar 计数，因此包含大量非 BMP 字符的报告可能被 TS 上报操作更严格地拒绝；Web 当前只调用读取操作。后续扩展协议时需要统一这个边界。

## 当前开发 API

默认地址：`http://127.0.0.1:8420`。这些接口只演示进程报告和读取，不是生产入网协议。

| 方法与路径 | 请求 | 响应 |
| --- | --- | --- |
| `GET /api/v1/overview` | 无 | `{ name, version, node_count }` |
| `GET /api/v1/nodes` | 无 | `{ nodes: [{ id, name, agent_version, last_seen }] }` |
| `POST /api/v1/agents/report` | `{ node_id, name, agent_version }` | `{ node: { id, name, agent_version, last_seen } }` |

表格是字段概览，不是完整 JSON Schema。精确类型、约束、可空性、时间格式及状态码以生成的 OpenAPI 为准。

### 报告记录的语义

- `node_id` 是开发态记录标识，由调用方提交；不证明密钥所有权。
- `name` 与 `agent_version` 是报告信息，不是可信能力声明。
- `last_seen` 是最近一次报告的时间，不能直接解释成在线状态。
- `node_count` 统计内存中的报告记录，不是健康隧道数量。
- controller 重启清空记录；当前没有持久化、生产认证或网络转发承诺。

## Web 如何消费真实契约

前端通过 `@meshway/control-client` 导出的 `getOverview(baseUrl)`、`listNodes(baseUrl)` 和生成类型读取 API。

```ts
import { getOverview, listNodes } from "@meshway/control-client";

const controllerUrl = "http://127.0.0.1:8420";
const [overview, nodes] = await Promise.all([
  getOverview(controllerUrl),
  listNodes(controllerUrl),
]);
```

具体返回类型由 SDK 生成声明确定。调用方需要处理网络失败、非成功状态码和 schema 验证错误。

TypeScript 编译只能检查代码中声明的形状，不能验证 HTTP 对端实际返回的 JSON。SDK 应在边界使用生成 schema 验证响应，验证通过才交给页面。不能使用 `any`、未验证的类型断言或“缺字段就补假数据”掩盖协议漂移。

## 前后端一起交付的含义

每个功能以一条可运行的纵向切片推进：

1. 定义 Rust 契约、路由与明确的业务语义。
2. 实现 controller 行为与必要的行为测试。
3. 生成 OpenAPI、Rust 客户端、TS SDK 与运行时 schema。
4. Agent / Web 消费生成接口，呈现真实的结果、空状态和错误状态。
5. 验证真实请求往返、生成物无漂移，并同步文档中的状态。

因此“前后端一起”不是先做虚构流量仪表盘。当前只交付概览和进程报告列表；未来的策略、路径与 trace 页面应随对应 API 与数据面能力一起交付。

## 漂移与兼容性检查

当前生成脚本与漂移检查可以直接从客户端包运行：

```sh
pnpm --filter @meshway/control-client generate
pnpm --filter @meshway/control-client check:generated
pnpm test:protocol
```

`check:generated` 将结果生成到临时目录，再与仓库产物逐文件比较，不依赖已有 Git 提交。完整检查链还应包括：

- 重新导出 OpenAPI 并重新生成全部客户端，与仓库产物应无差异。
- Rust 与 TypeScript 编译通过，Agent 和 Web 不依赖手写 DTO。
- 使用生成客户端对真实测试服务执行一次报告与读取往返。
- 对无效响应验证运行时 schema 能拒绝，并向上返回可处理的错误。
- 路由、方法、成功与错误响应契约都能关联到实际处理器。

首版使用 `/api/v1` 命名空间，但开发阶段的契约仍可能变更。生产版本兼容窗口、协商、废弃流程和错误模型需要后续明确。

## WASM 的位置

如果后续 Web 与 Rust 必须运行完全相同的策略编译 / 解释逻辑，可以评估共享 Rust 库编译为 WASM。它是可选的业务逻辑复用方式；HTTP 协议一致性由 Rust 契约、OpenAPI、生成客户端及边界验证解决。
