# 状态采集测试数据

这些文件是 2026-10-07 获取的真实响应／报告的字段投影；数值、样本顺序和字段名不改写。只删除 collector 不消费的原始日志、命令、profile dump 和环境详情，降低 fixture 大小。

- `actions.json`：`gh api repos/JDScript/meshway/actions/runs?per_page=100`，以及 runs `37359803215`／`37191584450` 的 `jobs?filter=latest`、`artifacts`。投影保留 run／job 元数据，两个都是历史 Check。
- `circuit.json`：`.meshway/perf/circuit/planck-20261007T050400Z-6ba8cc86/results.json`，36 个原始重复／profile 格。
- `dataplane.json`：`.meshway/perf/dataplane/20261007t204126z-cbe7469c/results.json`，8 个 L3／L4 TCP／UDP 原始样本，包括 tcpStreams。
- `crypto.json`：`.meshway/perf/crypto/planck-20261007T051345Z-51ab43c8/results.json`，host／linux 全部原语重复。

fixture 表示解析输入，不代表当前 CI 已跑过新 workflow 或该旧报告已经关联到新的 GitHub run。matrix 测试为暂定 CI1 契约构造输入，真实契约由 CI1 确认。
