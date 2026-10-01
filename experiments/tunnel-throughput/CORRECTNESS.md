# 性能专项的 Runtime correctness 前置修复

## R3：packet admission retirement

原审查的真实 duplex gate 反例已转为行为回归：旧 pump 的包在底层 writer
开始写之前，或只写出 6 bytes（length prefix + 部分密文）时被阻塞；停止旧 pump，
激活空准入，再放开 gate。旧 payload 不再到达对端。

共享 `packet_io::run*` 的签名不变。唯一 receiver lease 同时持有本地 packet
generation；join / drop 旧 pump 时同步 retire。generation 跟随 outbound batch
及已认证 ingress，退休后不再调用底层 packet write poll，也不再 dispatch 旧包。
退休与同步 dispatch / 单次 Future poll 使用同一短锁线性化，不跨 `.await` 持锁。

- 尚未开始的旧 batch 被丢弃，保持 carrier。
- 已进入 writer 的 record 若取消，保守关闭整个 stream 并要求重新认证：即使
  `AsyncWrite` 尚未返回写入数，adapter 也可能已有内部部分进度，不能复用半帧。
- UDP 每个 datagram 的发送 poll 同样 fenced，取消无需破坏 datagram carrier。
- endpoint Cipher / nonce / replay cache 独立保留。回归在 carrier 重认证后证明
  新包继续使用原 E2E key，且旧 payload 不出现。
- 退休时同步排除旧 ingress dispatch；重新激活前有界 drain 旧 delivery。
  ingress 代际按完成 hop 认证并本地 enqueue 的时刻归属。已经发送到网络的字节
  无法召回；无 wire generation 的远端在途包不能据此识别为远端旧方案流。
- 邻接诊断、E2E setup 等控制记录不绑定 packet admission generation；其生命周期
  仍由 Runtime / carrier 控制。没有全 Runtime shutdown 或 wire/prologue 变更。

调用者仍须先停止并 join 旧 pump，再 drain 平台设备、激活新 pump。直接使用
Runtime packet API 的实验默认拥有初始 generation；一旦 pump 退休，该 Runtime
的业务发送须等待新 pump 激活，不能经原 API 绕过退休 fence。没有新设备配置项。

新增回归：blocked/partial stream record、queued outbound、queued ingress，以及
E2E cache 保留后的真实加密多跳恢复。旧“停泵后继续填 delivery”测试已收紧为
停泵即拒绝 ingress；既有 lease / closed watch / full callback cancellation 保留。
