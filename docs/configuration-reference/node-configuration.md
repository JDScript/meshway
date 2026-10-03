# NodeConfiguration

Controller 给某个节点的**完整执行配置**。它包含实际协议参数和所需材料；“不需要管理员重复手填”不等于“不属于 configuration”。以下为下一版结构提案。

## 参数从哪里来 {#sources}

| 参数 | NetworkConfiguration 中的来源 | 下发结果 |
| --- | --- | --- |
| 本机 bind／监听端口 | 本节点 listener | listener 的实际设置 |
| 目标 host／port | 目标 listener 的公告 endpoints | 获准的出站候选 |
| VLESS UUID／Trojan password | listener 显式值；省略时生成并保存 | 监听端与获准拨号端使用同一值 |
| TLS 证书／私钥 | 管理员提供或平台生成 | 服务端获得私钥，客户端获得验证材料 |
| SNI、WS path／Host | 目标 listener 设置 | 对应客户端参数 |
| 拨号优先级／timeout | 本节点 dialers 或 defaults | 本地拨号参数 |
| peer 公钥／地址 | 已认证 Node 身份与分配记录 | 可信身份和授权引用 |
| access／paths／DNS | 整网配置编译 | 本节点可执行规则 |

管理员修改源配置中的协议参数，Controller 重新生成所有受影响的节点配置。生成值和材料引用与源版本一起持久绑定，只读查询／重发不会重新生成密码。

## 示例：A 拨 B 的 VLESS listener {#example}

管理员配置节选：

```json
{
  "nodes": {
    "<node-a>": {
      "hostname": "a",
      "dialers": { "vless": {} }
    },
    "<node-b>": {
      "hostname": "b",
      "listeners": {
        "public-ws": {
          "type": "vless",
          "listen": "0.0.0.0",
          "listenPort": 443,
          "endpoints": [{ "host": "edge.example.com", "port": 8443 }],
          "uuid": "11111111-1111-4111-8111-111111111111",
          "stream": "websocket",
          "tls": { "serverName": "edge.example.com" },
          "websocket": { "path": "/meshway", "host": "edge.example.com" }
        }
      }
    }
  }
}
```

在 topology／access／routing 允许的前提下，A 收到的执行配置包含以下内容。这里只展开出站和材料部分：

```json
{
  "networkId": "<network-id>",
  "nodeId": "<node-a>",
  "configurationRevision": 42,
  "digest": "<node-a-config-digest>",
  "configuration": {
    "dialers": {
      "<compiled-candidate-key>": {
        "type": "vless",
        "peerNodeId": "<node-b>",
        "listenerKey": "public-ws",
        "priority": 300,
        "endpoints": [{ "host": "edge.example.com", "port": 8443 }],
        "stream": "websocket",
        "flow": "",
        "credentialsRef": "vless-b-7",
        "tls": { "serverName": "edge.example.com", "materialRef": "tls-b-client-7" },
        "websocket": { "path": "/meshway", "host": "edge.example.com" }
      }
    },
    "materials": {
      "vless-b-7": { "type": "vless", "uuid": "11111111-1111-4111-8111-111111111111" },
      "tls-b-client-7": {
        "type": "tls-client",
        "certificateChain": ["<certificate PEM>"],
        "certificateSha256": "<certificate digest>"
      }
    }
  }
}
```

源配置的 dialers 按协议选择，**生成的 dialers 是具体获准出站实例**，因此有内部 candidate key。它不是管理员还要维护的一张连接表。

B 的配置包含本机 listener、同一 UUID，以及对应服务端证书／私钥材料。A 不拿 B 的服务端私钥。两份切片关联同一次发布和材料版本，各自 digest 不必相同。

## 完整配置包含什么

除了上例，完整切片还有本节点 hostname／地址、相关 peer 身份、公钥、授权规则、允许路径、出口方案和 DNS。示例省略这些部分不代表可以跳过校验或授权。

materials 是配置的私有部分，可以用引用去重，但 Agent 激活前必须取得并验证完整集合。UUID／password／证书既可以手工配置，也可以自动生成；它们的来源不改变其配置属性。

## Config 与 State

管理员公告的 endpoint 和协议参数属于 Config。STUN 当前映射、一次协调 attempt、实际选中的通道、RTT 属于 State。

动态 endpoint 更新只改变本配置已经允许的目标如何到达，不能替换 peer 身份、UUID、TLS 信任或业务授权。State 引用确切配置版本／digest，失配时不用于执行。

下发顺序及 full／delta 处理见[同步与分发](/configuration-reference/synchronization)。
