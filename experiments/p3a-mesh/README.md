# P3a 正常 Mesh v3 / TUN 验收

从仓库根目录运行：

```sh
python3 scripts/performance_lock.py docker build -f experiments/p3a-mesh/Dockerfile -t meshway-managedtest:p3a-execution-final .
python3 experiments/p3a-mesh/run.py meshway-managedtest:p3a-execution-final
```

镜像使用独立 Cargo cache、`CARGO_BUILD_JOBS=2`。运行完整持有共享 performance lock；每次创建全新的 Controller、三台正常 Agent、独立 Docker bridge 与私有 tmpfs。节点仅 `NET_ADMIN` 与 `/dev/net/tun`，drop ALL，无 privileged、host network、宿主 state bind 或宿主网络配置改动。Controller 仅发布随机 localhost API 端口；退出清理全部所创容器/网络。结果与失败证据保留在批准 TMP。

`NYC` / `LAX` / `SHA` 是复用 canary 的测试标签，不代表实际地理位置。没有 GeoIP 或 STUN 成功的假数据。仅创建平台托管 UDP listener，用真实生成管理 API 保存 listener、候选与 v3 policy；Agent 命令没有 profiles、STUN 或 v3 模式参数，只有既有 TUN opt-in。

场景覆盖：

- 无 Mesh 规则自动选择直接候选；有直接 A-C 候选时，first-match 必经 B 仍执行 A-B-C；重排规则改变路径。
- 单向 grant 的真实请求进入目的内核，状态型回包和反向主动业务不获隐式授权；空 grants 拒绝。两个独立单向 grants 取并集，交换 grants 顺序不改授权。
- connection block 优先于 allow；不能绕开必经 B 使用直接候选。
- 正常 Agent 重启重新协商 epoch；Controller revision 提升在 execution GET stall 前撤旧代；全 HTTP stall 下独立 lease watchdog 撤回且 Mesh 网段不落回底层。
- 必经节点撤回后不能直接绕行。

使用真实 ICMP/HTTP 与内核计数，执行 journal 的 revision/digest 与正常 Manager 的 TUN 生效证明交叉核对；journal 本身不当作 trace。单向 grant **尚无状态型 Mesh 回包闭环**。完整系统 underlay 保护准备 API 接线、Mesh 外出网与 IPv6 overlay 也不属于此 fixture 的通过结论，见 `docs/p3a-mesh-execution.md`。
