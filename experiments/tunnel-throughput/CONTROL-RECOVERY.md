# 邻接控制包短暂丢失引发的 channel 退休 / blackout

## 独立复现与最小修复

真实 NativeUDP 两节点，源端仅 listener、对端主动 dial；实际 E2E 业务先成功。
测试只丢弃首个邻接 probe reply，完整 payload/nonce/身份/路径验证均由原 Runtime
执行。probe deadline 固定300ms，没有改配置、lease、packet generation 或 E2E key。

| 事件 | 修复前 | 修复后 |
| --- | ---: | ---: |
| 首个 reply 丢失 | 0.418ms | 0.235ms |
| 此后实际 E2E 业务已送达 | 0.923ms | 0.430ms |
| probe 完成 | 302.671ms，Timeout | 103.680ms，成功 |
| channel 退休 | 是 | 否 |
| peer 再次可用 | 6089.613ms | 103.691ms，原 channel |
| E2E cache 改变 | 否 | 否 |
| packet generation 退休 | 否 | 否 |
| authentication drop | 5 | 0 |

修复前 acceptor 注销仍能传业务的通道，但没有 outgoing candidate；dialer 仍持有
旧 channel，直到独立失效检测后重连。故一次控制包损失能变成约6秒的业务中断。
这是有明确因果的控制包丢失反例，不能据此把全部历史低吞吐/blackout都认作同因。

修复只在 `neighbor::probe_observed` 的原 deadline 内增加最多两次重发：最多3个
请求共享原随机 request ID、同一 captured channel token、同一 pending slot。
每次仍经过原 carrier writer，分配新的 hop cipher nonce；没有重放旧密文。
收到有效 reply 才成功，最终 deadline 仍注销 exact channel，不影响并发 replacement。
没有延长认证/lease/deadline、取消 probe、修改 wire/prologue 或放宽 replay/grant。

## API / history 适配说明

公开函数签名和 `NeighborProbeObservation` 序列化形状均未改。
`round_trip` 从**首次请求开始**计时，包含 queueing 和丢失/重试等待，不是去掉重试
后的“纯网络 RTT”。一个 API 调用仍只产生一个 observation，重发不得被 history
记录成多个采样。最终 Timeout 仍是主动退休语义，采样并非纯被动观察。

初次本地 enqueue 满仍立即返回 Capacity；后续重发的 enqueue 满不丢弃已经在途
的 reply，继续等待原 deadline。原 responder 8 requests/s 限额、全局 pending
上限、每 exact channel 单个 in-flight probe 均保留。调用者取消、Runtime shutdown
或 channel replacement 会取消后续重发；没有分离出去的 retry task。

新 history runner 不在这些冻结镜像内，未用其解释已有断流；已有 managed fixture
自身每5秒用原 API 探测、timeout2秒。history作者继续复用公开API，后续按上述
first-request elapsed 口径统一解释。此提交不改 history、stream 或 generation 模块；
新 WS dirty-buffer 修复由其作者独立处理。

## 回归与证据

- 一次 reply 丢失时业务仍成功，probe 可恢复且 token 不变。
- 真实 E2E reverse business 填满一个有界 outbound queue，原 enqueue 确实拒绝
  reply；原队列恢复后不需换 key/channel 即可完成 probe。
- NativeUDP acceptor 的完整上述时间线。
- 已进入 retry 后 caller cancellation / shutdown 不再发送、不遗留 pending slot。
- 既有全丢失 timeout、replacement、迟到/错误 reply、重放、8/s响应预算回归保留；
  全丢失用例同时解密至多2个重发记录，验证新 nonce 且最终确实 EOF/退休。

原始日志位于本轮 ART（见 NEXT.md）：`control-recovery-red3.log`、
`control-recovery-green.log`、`control-recovery-native-all.log`。修复前3个反例全部失败；
修复后4个新增回归通过。本机 Clippy all-targets `-D warnings` 与全 dataplane
行为测试通过；native Linux 合并验收另行记录，不把本机结果冒充 Linux。

首版饱和测试曾误读 VLESS 而实际 fixture 为 NativeTcp，并在 paused-clock busy
loop 中等待，导致挂起；已停止准确的本任务进程，改正 counter、改为实时时钟，
并用独立30秒进程组 hard deadline 运行，失败日志保留，没有用挂起样本作证据。
