# Native TCP Listener

原生 TCP 监听配置。

## Structure

<!-- schema: Listener -->
```json
{
  "type": "native_tcp",
  "remark": "Mesh TCP 入口",
  "listen": "0.0.0.0",
  "listenPort": 8421,
  "endpoints": []
}
```

## Fields

采用 [Listen Fields](/v2/configuration-reference/shared/listen)，包括 TCP socket 设置。

`type` 固定为 `native_tcp`。本类型使用原生 Meshway 认证／加密，不接受 VLESS、Trojan、WS 或 TLS 包装字段。需要 VLESS/TLS 时选择对应类型，不能通过添加 tls 对象改变此协议。

TCP listener 可被获准 peer 拨入并建立双向通道。业务方向仍由 access 决定。
