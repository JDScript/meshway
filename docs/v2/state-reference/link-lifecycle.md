# 连接生命周期与观测

连接状态来自 Node 实际报告。topology 允许一条边不代表这条边已建立；查询历史也不要求该连接继续存在。

## 状态与依赖

| 事实 | 表达 |
| --- | --- |
| 通道认证完成且仍存在 | LinkState.phase=authenticated |
| 通道有业务依赖 | businessDependencies 大于零 |
| 通道用于诊断 | observationIds 包含相应任务 |
| Node 报告关闭 | phase=closed 或真实 link_closed 历史事件 |
| Server 未收到新报告 | freshness 过期；不推断物理 socket 关闭 |

按需建连、通道复用和空闲回收属于 Node 连接调度，运行范围见 [Node](/v2/node)。回收须考虑活跃流、半关闭会话和必要路径依赖，不能只按最近是否出现数据包判断。

## 三种观测

| 行为 | 用途与口径 |
| --- | --- |
| NAT keepalive | 维持仍需使用的 socket 映射；本身不是 RTT 样本 |
| 连接健康探测 | 为连接 owner 的重试／故障切换提供证据 |
| 延迟历史采样 | 保存真实计时结果，不通过重复心跳或查询造点 |

只有存在对应 request/reply 计时边界才可记录往返耗时。冷启动建连耗时和认证通道上的 probe 分别使用 connect／neighbor 测量类型；不能合成一条稳定 RTT 曲线。

## 无样本与无连接

Node 确认没有 link 时可表达无连接。Server 只有过期报告时则是未知。每对节点的产品级结论（connected／idle／unreachable／unknown）由 Server 汇总两端报告给出，见 [Adjacencies](/v2/state-reference/network#adjacencies)。没有样本不表示 RTT=0、节点离线或丢包率=100%。

历史若有明确 idle 关闭事件，可展示“空闲、未测量”；缺少事件时只显示“无样本／原因未知”。[历史查询](/v2/state-reference/observations)不隐式发起测量。

[实时观察](/v2/state-reference/realtime)有独立的短租约诊断许可。最后一个订阅结束后释放诊断依赖；普通业务依赖和连接生命周期仍由 Node 管理。观察超时不能直接命令拆除业务通道。
