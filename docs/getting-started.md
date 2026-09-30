# 开始使用

先运行一个本地纵向切片：Rust Agent 上报 → Rust controller 保存内存记录 → Next.js 读取同一套 API。VitePress 独立提供中文设计文档。

::: info 开发演示的边界
控制面默认绑定 `127.0.0.1:8420`。报告不带生产认证，状态保存在进程内存中，重启后丢失。界面中的记录只表示收到过进程报告，不表示节点身份已验证或网络隧道已建立。
:::

## 准备环境

- Rust 1.98.0 与 Cargo；仓库的 `rust-toolchain.toml` 固定工具链版本。
- Node.js 24 与 pnpm 11；精确 pnpm 版本见根 `package.json` 的 `packageManager`。
- 在仓库根目录执行以下命令；各开发服务分别使用一个终端。

```sh
pnpm install
pnpm --filter @meshway/control-client generate
```

生成步骤让 Rust Agent 客户端、TypeScript SDK 与控制面契约保持同步。工作区脚本与实际参数以仓库根配置和 Rust CLI 的 `--help` 为准。

## 1. 启动控制面

```sh
cargo run -p meshway-controller -- serve
```

读取概览：

```sh
curl http://127.0.0.1:8420/api/v1/overview
```

响应字段为 `name`、`version` 和 `node_count`。启动时没有报告记录，`node_count` 应为 `0`。

## 2. 发送一次 Agent 报告

```sh
cargo run -p meshway-agent -- report \
  --controller http://127.0.0.1:8420 \
  --name nyc \
  --node-id 00000000-0000-4000-8000-000000000001
```

这是一次进程信息报告，尚不是持续运行的数据面 daemon。示例 UUID 用来关联报告，不能当作认证凭据。

```sh
curl http://127.0.0.1:8420/api/v1/nodes
```

节点列表形状为 `{ "nodes": [...] }`，每条记录包括 `id`、`name`、`agent_version` 与 `last_seen`。`last_seen` 表示最近报告时间，不是连通性探测结果。

## 3. 启动管理界面与文档

```sh
pnpm dev:web
```

打开 `http://localhost:3000`。Next.js 服务端通过 `@meshway/control-client` 读取控制面真实数据，默认地址为 `http://127.0.0.1:8420`，可用 `MESHWAY_CONTROLLER_URL` 调整。请求失败会显示错误状态，不以示例节点替代真实结果。

另一个终端启动本文档：

```sh
pnpm dev:docs
```

打开 `http://127.0.0.1:5173`。也可以直接运行文档包：

```sh
pnpm --filter @meshway/docs dev
pnpm --filter @meshway/docs build
pnpm --filter @meshway/docs preview
```

`preview` 在 `127.0.0.1:5173` 提供构建结果，与文档开发服务共用端口，应先停止开发服务。

## 这个切片如何验收

1. 控制面启动后，概览返回空记录数量。
2. 执行 Agent 报告，列表和 Web 显示同一条真实报告记录。
3. 关闭控制面后，Web 能呈现连接失败。
4. 重启控制面，内存记录清空。
5. 生成客户端与服务端契约检查通过，文档构建没有死链接。

网络转发的验收是单独的后续里程碑，见[开发路线与验收](/development)。
