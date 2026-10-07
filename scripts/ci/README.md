# CI 状态索引

`collect.py` 使用已登录的 `gh api`（CI 用 `GH_TOKEN`，也接受 `GITHUB_TOKEN`），只读取 GitHub Actions 元数据和 artifact；Python 标准库，无构建依赖。页面入口为 `docs/v2/status.md`。

## 命令

```sh
# 最近 200 个 API run；刷新 Check / Validate / Perf / Build，保留窗口外的历史索引
mise exec -- python3 scripts/ci/collect.py
mise exec -- python3 scripts/ci/collect.py --limit 20 --repo JDScript/meshway

# 本地孤立分支发布：不 checkout、不改当前 index、不 push
mise exec -- python3 scripts/ci/collect.py --publish

# CI：先取 origin/ci-results 作为增量基线，再提交并 push（非 force）
mise exec -- python3 scripts/ci/collect.py --publish --push

# 文档读取静态快照；首次不存在分支时写入显式空索引
mise run status:sync
mise run status:sync -- --local
mise run dev:docs
```

默认输出 `.meshway/ci/index.json`，可用 `--output` 指定；非 publish 模式优先从该文件增量更新。publish 模式从 results 分支更新；第一次用 `git hash-object`／`mktree`／`commit-tree` 创建无父提交。后续提交仅以 results 历史为父；`update-ref` 使用旧值 CAS。push 遇到分歧失败，不能覆盖另一发布者。工作树、staging index 与运行服务均不参与发布。

`status:sync` 默认 fetch origin 的 results ref；远端尚无分支时使用本地 `ci-results`，两者都没有时生成空索引。`--local` 完全离线。目标 `docs/public/status/index.json` 被 Git 忽略，写入为原子替换；浏览器通过 VitePress base 下的 `/status/index.json` 读取并逐字段校验。GitHub 私有 artifact 链接使用浏览器自己的登录会话；token 不进入页面或索引。

## index.json schema v1

所有时间为 API 原始 ISO 8601（`updatedAt` 顶层为采集 UTC 时间）。缺测／不适用用 `null`，列表可为空。字段新增可向后兼容，改变含义或类型必须递增 `schemaVersion`；当前读端拒绝其他版本。

| 对象 | 字段 |
| --- | --- |
| 根 | `schemaVersion: 1`, `repository: string`, `updatedAt: string|null`, `runs: Run[]` |
| Run | `id: number`（唯一键）, `attempt: number`, `workflow`, `event`, `branch: string|null`, `commit`（完整 SHA）, `message`（首行）, `createdAt`, `updatedAt`, `status`, `conclusion: string|null`, `url`, `jobs: Job[]`, `artifacts: Artifact[]` |
| Job | `id`, `name`, `runner: string|null`（实际 runner 名）, `status`, `conclusion: string|null`, `startedAt/completedAt: string|null`, `seconds: number|null`, `url` |
| Artifact | `id`, `name`, `bytes`（API ZIP 大小）, `expired: bool`, `url`, `jobId: number|null`, `runner: string|null`, `seconds: number|null`, `state: ignored\|ok\|expired\|error`, `error: string|null`, `reports: Report[]`, `matrices: Matrix[]`, `builds: Build[]` |
| Report | `reportId`（原路径目录名）, `kind: circuit\|dataplane\|crypto`, `revision: string|null`, `dirty: bool|null`, `quick: bool`, `sourcesUnchanged: bool|null`, `loadMin/loadMax: number|null`（首尾一分钟 load 范围）, `metrics: Metric[]` |
| Metric | `label`, `unit`, `scope`（排序后的参数 JSON 字符串，固定标量白名单）, `value: number|null`（成功重复中位数）, `samples`（有效值数）, `attempted`（对应格尝试数） |
| Matrix | `reportId`, `short: bool`, `rows: MatrixRow[]` |
| MatrixRow | `row`, `passed: bool|null`, `reportId: string|null`, `seconds: number|null`, `load: string|null`（适配后的 JSON 展示文本，最多 256 字符） |
| Build | `target`, `binary: meshway\|meshwayd`, `bytes`（解压后的文件大小）, `sha256`（实际文件完整 SHA-256） |

### 采集与归属

- runs、jobs（`filter=latest`）、artifacts 均分页。默认刷新最近 200 个 API run，忽略 Status 与 `ci-results` 分支；历史 Run 按 ID 更新，不因刷新窗口移动而删除。重跑更新 attempt／job／结论。
- 每个刷新 run 重新读 jobs／artifact 元数据。已成功解析的 artifact 按 ID 复用摘要；过期或删除的已采集摘要保留，标记 `expired`；尚未采集即过期的没有虚构指标。下载／格式错误写入 `state=error`，下次采集重试；API 元数据获取失败则整次停止，保留旧索引。
- artifact 与 job 的显式映射：`perf-arm64-* → perf-arm64`，其余 `perf-* → perf`，`validate-matrix-* → matrix`，`node-<target>-<40-char-sha> → node-linux (<target>)`。未知／多义 job 不猜 runner。早于当前 job 的 artifact 不换绑到重跑后的 runner；缓存归属保留原 job。
- `perf-*` ZIP 内路径须含 `circuit`／`dataplane`／`crypto` 及 `<reportId>/results.json`；validate ZIP 内读取 `<reportId>/matrix.json`。文件只在 ZIP 内读取，不展开、不执行。ZIP／总展开大小上限 256 MiB、单报告 32 MiB、成员 10,000；拒绝重复／越界路径。
- `node-*` 中读取 `meshway`／`meshwayd`，流式计算完整 SHA-256。构建耗时是 job 墙钟，包含 setup，不伪装为纯编译命令耗时。

### 性能指标与比较

- L0：只取 2／3／5 节点、1280 B、profile 0；pps 原值，CPU µs/packet = `harnessCpuNs / measured.deliveredPackets / 1000`。
- L3 `secure-direct`／`secure-relay`、L4 `manager-relay`：profile 0；TCP `goodputMbps`，UDP `packetsPerSecond`。RTT 为每重复 TCP sender `tcpStreams[].mean_rtt` 的算术均值，从 µs 转 ms；重传为该重复 streams 的 `retransmits` 合计；再取重复中位数。缺少任意 stream 值不补零，不从 Mbps 倒推 pps。
- 原语：1280 B、按 environment 与 operation 分开；ChaCha／ring AEAD 及 HMAC 系列，`cpuNs / iterations / 1000`。new／prekey／clone、alloc／inplace、v5 transcript 不互混。
- scope 区分 batch、profile、workers、allocator、CPU／内存 quota、MTU、payload、UDP rate、stream 数、seconds／omit 等真实参数。平台始终是 runner 名，不从 target 推断测量机。
- 页面曲线仅 main、同 runner／指标／scope；成功值中位数保留有效数／尝试数及 load。short/quick、dirty、源码改变或不完整重复不进入变化基线；报告 revision 与 run SHA 不同的残留文件不进入曲线。两值变化的绝对比例 >10% 标橙；上一值为 0 或缺测时百分比未知。颜色只是变化提示，不判断回归或验收。
- 一个 artifact 内可有多个报告，均保留 ID；同 run 的独立报告不拼接重复、不择优。原始 stderr、commands、身份材料与主机环境 dump 不进入索引。

## CI1 matrix.json 适配接口

`parse_matrix(document, report_id)` 是唯一输入适配位置，当前假定：

```json
{
  "short": false,
  "rows": [
    { "row": "nat/off", "passed": true, "reportId": "20261007-example", "seconds": 125.5, "load": [1.2, 1.0, 0.8] }
  ]
}
```

`short` 必须为 bool；`passed` 可为 bool／null，缺失按未知。报告 ID／耗时可缺失；load 接受 JSON scalar／数组／对象并转为有限长度文本。顶层报告 ID 使用 artifact 路径；单行 reportId 优先。待 CI1 确认实际行名称、失败／跳过表达、load 结构、报告 revision／产生时间字段后，在该函数适配并补 fixture。当前不会把缺少矩阵或未知行当作通过。

## 触发与验证

`Check | Validate | Perf | Build completed → Status.workflow_run → collect (--publish --push) → ci-results`。Status 也接受手动触发，固定从 main checkout collector，`status-index` concurrency 串行。权限仅 `actions:read`／`contents:write`。结果分支使用 GITHUB_TOKEN push，GitHub 不再触发常规 push 链；采集及 workflow 条件也排除 results 分支。

```sh
mise run test:scripts
mise run check:docs
```

离线测试覆盖真实 API 投影、分页、重跑／增量／过期、错误重试、三种报告口径、缺测、矩阵输入类型、ZIP 内容与 SHA，以及独立临时 Git 仓库的无父发布／历史续接／工作树保持。测试不需要 gh 登录、Cargo、Docker 或现存 `.meshway`。
