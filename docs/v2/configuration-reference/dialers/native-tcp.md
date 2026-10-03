# Native TCP Dialer

**当前能力：平台托管。** 以下选项放在 `nodes[id].dialers.native_tcp`，为下一版字段格式。

## Structure

```json
{
  "remark": "TCP 备用",
  "priority": 200,
  "connectTimeout": "5s"
}
```

## Fields

继承 [Dial Fields](/v2/configuration-reference/shared/dial) 的 TCP 子集。priority 缺省为 200；本类型不接受 VLESS／Trojan、TLS 或 WS 包装配置。

目标为 Controller 下发的获准入口。一个 TCP channel 可以双向复用，不能据此产生反向新流授权。TCP simultaneous-open 是另一项协调能力，不因为存在 TCP dialer 就宣称已经支持。
