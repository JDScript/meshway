# Mesh Linux kernel canaries

`container.py`、`container-runtime.py`、`checks.ts` 提供当前 Mesh/DNS/VLESS runner 共用的隔离 TUN、HTTP、路由与日志观测 helper。当前正常 policy/TUN 验收入口是 `experiments/p3a-mesh/run.py`。旧有序allow/reject专用重放runner已退役；原始运行身份、测量与FAIL证据保留于Git/私有产物，不由这些helper修改。
