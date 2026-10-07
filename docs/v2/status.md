---
outline: false
aside: false
---

<script setup>
import ActionsStatus from '../.vitepress/components/ActionsStatus.vue'
</script>

# CI 与性能状态

合入后自动层（T1／T2 全矩阵／T3 固定口径）的运行摘要与指标时间序列。数据来自孤立分支 `ci-results` 的索引，由状态页脚本生成；规则见[工程流程](/v2/process)。

本地先运行 `mise run status:sync`，再用 `mise run dev:docs` 打开本页。读取站内 `/status/index.json`，浏览器不持有 GitHub token。初次尚无索引时显示空状态；索引与原始 artifact 的保留期独立。详细测量口径见[分层性能测量](/v2/node/performance)，人工验收范围仍以[验收范围](/v2/node/validation)为准。

<ActionsStatus />
