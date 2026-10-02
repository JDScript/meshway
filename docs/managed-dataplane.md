# 托管 Mesh IPv4 数据面

Linux TUN 接管 Mesh IPv4 包，底层 dataplane 负责 Noise E2E 与转发，中转节点不获得业务明文。正常执行消费单一当前 `NodeMeshExecution`，不会在节点重新编译管理员源。

认证邻接、端点 grants、完整有序必经路径必须同时成立。下一跳可主动 dial 当前节点，认证双向通道供两方向复用；通道双向不产生反向授权。当前单向 grant 尚无状态型回包，往返业务需要显式双向授权。

Agent 在具备 TUN 权限的隔离 Linux 环境显式启用 `run --apply-dataplane`。准备系统底层保护、私有保存和失败关闭 sink 后，核对当前能力/epoch/revision/digest 与密钥，再安装精确路径。ACK 只代表验证并保存；实际 TUN、listener/peer 运行报告与真实 ICMP/HTTP 是不同证据。

撤回、lease 到期、更高可信 revision 与 ACK 冲突独立结束旧代际；停止 TUN 后 Mesh 地址池不应通过底层默认路由泄漏。外层 Controller/transport/probe/bootstrap DNS 使用用途受限保护，业务访问同一地址仍受 policy 约束。

当前执行只覆盖 IPv4 Mesh；完整出口、可信域名上下文、状态型回包、IPv6 overlay、自动邻接/主备与打洞仍待完成。协议、预算和正常验收入口见[执行合同](/mesh-execution)；旧 allow/reject replay runner 已退役，原始证据保留于 Git/私有产物。
