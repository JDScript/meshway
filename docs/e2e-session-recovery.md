# E2E packet 接收状态丢失后的恢复

端到端认证的有界`resync-v1`已合入并完成独立审查/消费者验收；固定`4c9a5be`正常Controller/Agent/TUN的同身份直连与四地有序路径恢复复验已PASS。原`4d106e0`的真实TTL失败窗口保留不变，见[远端证据](/linux-field-validation)。

## 恢复语义

端点重启后，存活端按完整有序路径缓存的 `packet-v1` sender 仍可能使用重启端已丢失的 receiver key。邻接恢复、新 echo 和反向新 packet 都不是旧 receiver 状态仍存在的证明。

当前实现仅在接收端发现 packet session **不存在或已过期**时，沿该 packet 原授权路径反向发出一个小型 unknown-session hint。已知 session 的路径/模式不匹配、MAC 错误或 nonce replay 不发 hint。hint 未经 E2E 认证，不能直接清除、替换或延长 sender。

发送端只对当前 cache ID 和完整路径匹配的 hint 记录一个布尔标志。下一次正常 `send_packet` / batch 调用，在该路径已有 setup permit 内执行：

1. 新随机 control session ID、新 Noise IK、当前配置 pinned endpoint key。
2. 独立模式 `resync-v1`；Noise prologue 绑定 control ID 和完整有序路径。
3. 新密钥加密查询：旧 packet session ID（16 bytes）+ 旧 cipher 的 Noise transcript hash（32 bytes，非秘密）+ fresh challenge（32 bytes）。
4. 接收端只在成功解密查询、确认 initiator 持有 fresh key 后，检查同一 origin、exact ID/hash、完整路径、packet 模式与 expiration。响应为原查询 80 bytes 加一个 `exists` byte（0 / 1），使用新 E2E key 加密。旧 packet Init 在重启后被重放可能重建相同公开 ID，却生成不同 responder key；不能把它误报为旧 cipher 仍存在。
5. 发送端校验模式、pinned key、响应 phase、旧 ID/hash、challenge、长度及状态值；仅 authenticated `exists=0` 才 compare-and-invalidate **该路径的 exact 旧 ID/hash**。再走原 packet-v1 建立流程。

`exists=1`、伪造/错绑响应、容量不足、不支持或超时均保持 active key；普通 TTL 仍独立生效。等待控制往返后重新检查 expiration，不能延长过期 key。迟到回复没有 pending owner，或不能通过 fresh cipher/challenge；也不能踢掉新的 cache ID。

控制往返按原 path 向前/向后行进，每跳检查 exact allowed path 与正确上一跳。反向控制响应属于原请求，不是反向业务授权。没有反向业务 grant 时 resync 仍可完成，反向主动 packet 继续拒绝。

## Wire 与混合版本

packet envelope version 仍为 1，原 phases 1–5、`packet-v1` 和 `echo-v1` 的内容不变。新增内部 phases：

| Phase | 值 | 方向 |
|---|---:|---|
| UnknownSession | 6 | 原路径反向 |
| ResyncInit | 7 | 原路径正向 |
| ResyncReady | 8 | 原路径反向 |
| ResyncQuery | 9 | 原路径正向 |
| ResyncAnswer | 10 | 原路径反向 |

UnknownSession body 为精确 `unknown-session-v1`；ResyncInit / Ready Noise payload 为精确 `resync-v1`。Query / Answer 只属于这个独立会话。不会占用 echo-v1 的任何 magic payload，任意合法 echo payload 仍原样 echo。业务仍使用 packet-v1，不引入 Controller/HTTP 能力字段或新的设备配置。

这是**机会式内部扩展**，不是已协商的全路径能力合同。旧节点的固定 Phase decoder 拒绝新增 phase：旧 receiver 不发 hint，旧 sender 不理解 hint，旧中转无法转发 hint/resync，旧 endpoint 不响应 resync。以上组合保持原 packet-v1 / echo-v1 和原自然 TTL 语义，不降加密、不因不支持而清 key。快速恢复要求这条路径参与节点均支持扩展；这里没有宣称混合版本也快速恢复。

真实旧endpoint/relay/sender互操作已验证：拒绝或丢弃新phase时保留旧key及自然TTL行为，原echo payload（含扩展名字符串）仍原样返回；不以“全新版快速恢复”替代旧版本边界。

## 有界工作与生命周期

- 所有 hint/query 状态在初始化时从已配置路径建表（P 个条目），随机 ID 不新增条目；每条路径只保存当前 ID、一个 hint 位和三个时间门限。
- hint 发出、query 发起、resync Init 处理共享 burst 8 的全局控制预算，耗尽后最早一秒补充；各路径 emission/query/init 另有一秒门限。无法用反复假 hint 无界创建 control work。
- control receiver 池独立上限为 `max_pending_probes=L`，原 endpoint packet/echo 池不被占用。control receiver 按原 probe timeout 到期，dispatcher 定期清理且 admission 当场清理；总接收 cipher 上限由原 3L 增为 4L。发送方 pending 沿用原全局 L 池。
- resync 在正常调用内、同路径 singleflight，receive dispatcher 全部同步，不 await 同 channel 握手，不派生后台 payload 任务。共享 sending/nonce 锁不跨网络 await。
- 查询最多使用原调用 timeout 的一半；整个操作继续受原调用 deadline、shutdown 和 PacketGeneration fence 限制。调用取消会释放 permit / PendingGuard，不发布迟到 cache reset 或业务 payload。
- 新 recovery phase 的入口、转发及外层写出受 packet fence 限制；入口同时保留 socket fence 和 carrier token 检查。发送操作通过现有 Fence 的逐 poll admission/cancellation，网络 await 期间撤回也能终止。
- receiver active/pending 晋升、packet TTL、Noise 密钥、nonce assignment 和 128-packet anti-replay 没有放宽。外层重连、普通双向 rollover 或 fresh incoming 本身不清 reverse sender。

## 恢复时间与交付边界

健康通路上，收到 hint 后的下一次正常发送执行两次控制 RTT（Init/Ready、Query/Answer），再一次 packet setup RTT 后发送业务。首个 stale packet 已经丢失，不保存或重放；`send_packet` 成功仍只表示本地入队。实际应用恢复还取决于下一次发送/应用重试、排队和 RTT。

门限/预算可能延后查询；丢失 hint、持续故障、竞争或混合旧版本仍可能回到原 TTL，不能承诺无条件毫秒级或应用总恢复上界。host TCP 观测是机制证据，不是远端 Controller/Agent/TUN 验收。

查询不清除 receiver 的 active/pending packet keys。攻击者在重启后抢先重放旧 packet Init，仍可能占用原 packet pending 预算，或让首个旧业务 packet 遇到已知 ID 的 MAC 失败而不产生 hint；这属于上述重放/竞争下的可用性边界。不能为了加快这种情况而把 known-session MAC 失败升级为未经认证的 reset，或绕过原 pending/key-confirm 防线。已发起的 authenticated 查询则通过 transcript hash 区分同 ID 的不同 cipher。

行为覆盖包括真实一跳/多跳重启（源端–中转 socket 不变）、仅正向 grant、原外层重连、真实 60s 双向滚动、假 hint 与 authenticated positive、重放 Init/query、错误 ID/path/key/mode/challenge/密文、迟到 missing answer、nonce replay/MAC 错误不发 hint、全局/逐路径限速与 receiver/pending capacity、caller 取消及网络 await 中 packet generation 撤回。

## 验证入口

`crates/dataplane`的实际Runtime行为回归覆盖一跳/多跳、单向业务grant、旧wire互操作、错误绑定、限速/容量、取消和packet fence；正常Linux同身份恢复另由`experiments/linux-field/retest.ts`验收。局部机制时延不是现场SLA。

批准TMP保留`meshway-e2e-session-review-6262bc1.md`、`meshway-e2e-integration-40d2bd8.md`及对应日志；原失败窗口/旧库来源不改写。新完整复验入口见[Linux验收](/linux-field-validation)。
