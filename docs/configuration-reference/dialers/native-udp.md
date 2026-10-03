# Native UDP Dialer

**当前能力：平台托管。** 以下为下一版字段格式。

## Structure

```json
{
  "type": "native_udp",
  "remark": "优先 UDP",
  "priority": 100,
  "connectTimeout": "5s"
}
```

## Fields

继承 [Dial Fields](/configuration-reference/shared/dial) 的 UDP 子集。priority 缺省为 100；不接受 tcp／tls／websocket 字段。

Controller 选择获准 peer 及候选；NAT 协调必须使用同一实际 socket 的观测和认证结果。普通 UDP socket 可发送、STUN 映射存在与已经打洞成功是不同状态。

业务授权、必经节点与逻辑路径不会因 UDP 候选更快而改变。
