# Native TCP Dialer

以下选项放在 `nodes[id].dialers.native_tcp`。

## Structure

<!-- schema: NativeTcpDialer -->
```json
{
  "remark": "启用 TCP 拨号",
  "connectTimeout": "5s"
}
```

## Fields

继承 [Dial Fields](/v2/configuration-reference/shared/dial) 的 TCP 子集；本类型不接受 VLESS／Trojan、TLS 或 WS 包装配置。

目标为 Server 下发的获准入口。一个 TCP channel 可以双向复用，不能据此产生反向新流授权。TCP simultaneous-open 属于独立协调能力，不由 TCP dialer 配置本身提供。
