# Native UDP Listener

原生 UDP 监听配置。

## Structure

<!-- schema: Listener -->
```json
{
  "type": "native_udp",
  "remark": "Mesh UDP 入口",
  "listen": "0.0.0.0",
  "listenPort": 8421,
  "endpoints": []
}
```

## Fields

采用 [Listen Fields](/v2/configuration-reference/shared/listen) 的 enabled、remark、bind、endpoints 与 UDP 适用的 socket 字段。

`type` 固定为 `native_udp`。不接受 tcp、tls、websocket、VLESS/Trojan 凭据字段。Node 身份与认证来自 Meshway，不存在另外的代理账户。

## Runtime

外层 UDP datagram 与 Mesh IP 包不同；实际外层包长须满足路径／加密开销后的预算。发现信息关联实际 UDP socket，配置一个端口不代表 NAT／防火墙已允许它。
