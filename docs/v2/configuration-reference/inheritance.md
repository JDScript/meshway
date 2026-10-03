# 字段复用与默认继承

下一版配置采用一层默认集合，加字段自身的缺省值；不引入递归的 JSON `extends` 链。

## 通用字段如何 extend

这是 **Schema／类型的组合**，不是配置文件中需要填写的字段：

```text
VLESS Listener = ListenFields + VLESS 入站字段
Trojan Listener = ListenFields + Trojan 入站字段
VLESS DialOptions = DialFields + VLESS 出站选项
Trojan DialOptions = DialFields + Trojan 出站选项
```

`remark`、bind、timeout、TCP 选项只在通用页定义一次。协议页只说明新增字段、适用范围及协议特有约束。同一字段的类型和语义不能在两个协议页各写一套。

## defaults 与节点覆盖

```json
{
  "defaults": {
    "dialers": {
      "native_udp": { "priority": 100 },
      "vless": { "priority": 300 }
    }
  },
  "nodes": {
    "<node-a>": { "hostname": "a", "dialers": {} },
    "<node-b>": { "hostname": "b", "dialers": { "vless": {} } }
  }
}
```

这是与继承相关的配置节选。

| 情况 | 结果 |
| --- | --- |
| 省略 defaults.dialers | 使用平台默认协议集合 |
| defaults.dialers 明确为 `{}` | 网络默认不主动拨号 |
| 节点 dialers 省略或为 `{}` | 使用网络 defaults.dialers |
| 节点 dialers 非空 | 用它替换整个默认集合，不隐式保留其他协议 |
| 协议选项为 `{}` | 该协议启用，其字段使用 Schema 默认值 |
| 某协议 enabled=false | 该协议在选定集合中禁用 |

上例 A 使用 UDP 与 VLESS，B 只使用 VLESS。若网络默认非空，某节点要关闭主动拨号，可提供非空集合并将其中各项设为 disabled。

选定集合后才填字段默认值，例如 VLESS priority=300、connectTimeout=5s。节点替换集合后，不再逐字段继承网络中同名协议项；这避免“整项替换”和“深层 merge”混用。

## 显式参数与平台生成

1. 管理员明确填写的 listener 协议参数优先。
2. 可自动生成的参数首次省略时，由平台生成并持久保存。
3. 对已有资源再次保存，省略自动材料表示继续使用该资源已保存的材料，不是每次随机换一套。
4. Controller 从 listener 参数和拨号选项生成完整节点配置；字段缺失、引用冲突或不可兼容组合明确报错。

**生成是填默认值的方式，不是禁止管理员编辑的理由。** 稳定材料的生成／轮换属于持久化操作，不在纯 diff 或只读查询时执行。

## 加一种协议需要扩展什么

协议声明统一登记：稳定 type、入站／出站专用字段、默认值、组合校验及材料需求。Controller 据此生成目录、Schema 和节点配置；Agent 注册对应运行实现与实际能力。

新增协议不复制 Listen／Dial 通用逻辑，也不通过任意 JSON 动态下载执行代码。当前工程尚未完成统一 registry，现状见[承载支持状态](/v2/configuration-reference/transports)。
