# 字段复用与默认继承

配置采用一层默认集合，加字段自身的缺省值；不引入递归的 JSON `extends` 链。

## 通用字段

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
      "native_udp": {},
      "vless": {}
    }
  },
  "nodes": {
    "<node-a>": { "hostname": "a" },
    "<node-b>": { "hostname": "b", "dialers": { "vless": {} } },
    "<node-c>": { "hostname": "c", "dialers": {} }
  }
}
```

这是与继承相关的配置节选。

| 情况 | 结果 |
| --- | --- |
| 省略 defaults.dialers | 使用保存此配置时固定的平台默认协议集合 |
| defaults.dialers 明确为 `{}` | 网络默认不主动拨号 |
| 节点 dialers 省略 | 使用网络 defaults.dialers |
| 节点 dialers 明确为 `{}` | 禁用该节点主动建连，不影响获准 listener 接收 |
| 节点 dialers 非空 | 用它替换整个默认集合，不隐式保留其他协议 |
| 协议选项为 `{}` | 该协议启用，其字段使用 Schema 默认值 |
| 某协议 enabled=false | 该协议在选定集合中禁用 |

上例 A 使用 UDP 与 VLESS，B 只使用 VLESS，C 不主动拨号。关闭不依赖枚举所有协议，因此以后平台增加协议也不会意外重新打开 C 的出站。

选定集合后才填字段默认值，例如 VLESS connectTimeout=5s。节点替换集合后，不再逐字段继承网络中同名协议项；这避免“整项替换”和“深层 merge”混用。

## 显式参数与平台生成

1. 管理员明确填写的 listener 协议参数优先。
2. 可自动生成的参数首次省略时，由平台生成并持久保存。
3. 对已有资源再次保存，省略自动材料表示继续使用该资源已保存的材料，不是每次随机换一套。
4. Server 从 listener 参数和拨号选项生成完整节点配置；字段缺失、引用冲突或不可兼容组合明确报错。

稳定材料的生成属于持久化操作，不在纯 diff 或只读查询时执行。管理员可显式填写协议参数。

### 版本固定

每个持久版本保存原始 JSONC，以及包含完整默认参数和生成材料的规范化配置。软件升级不能在读取旧版本时再次补默认值，导致增加协议或改变有效参数。原文记录编辑意图，解析后的有效值是确定结果，不是另一份独立可编辑源。

### 资源重建

已提交删除后再创建同名 listener，按新资源生成自动 UUID／password／证书和运行代际，不按名称认领旧材料。管理员明确填写的值仍以输入为准；已吊销材料不能通过回滚重新有效。

创建代际作为该配置版本的内部 listener 绑定保存，可使用创建 revision；正常编辑沿用，删除后重建换新。材料值保存在配置本身，运行 socket 另有实际 generation。

省略不是轮换；在同一个尚未提交的草稿中删除又加回完全相同的内容，也不会凭空产生一次资源重建。资源修改仍走 CAS 和完整配置提交。

## 加一种协议需要扩展什么

协议声明登记稳定 type、入站／出站专用字段、默认值、组合校验及材料需求。Server 据此提供目录、Schema 和节点配置；Node 须有对应运行实现并声明实际能力。

新增协议复用 Listen／Dial 通用逻辑，不通过任意 JSON 动态下载执行代码。目录与能力匹配见[承载](/v2/configuration-reference/transports)。
