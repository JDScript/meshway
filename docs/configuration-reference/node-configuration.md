# NodeConfiguration

**下发格式草案。** Controller 从网络配置生成本节点需要的执行内容，不是管理员独立编辑的第二份配置。

## Structure

```jsonc
{
  "networkId": "<network-id>",
  "nodeId": "<node-id>",
  "configurationRevision": 42,
  "digest": "<node-config-digest>",
  "configuration": {
    "hostname": "backend-1",
    "addresses": ["100.96.0.2/32"],
    "listeners": {},
    "dialers": {},
    "peers": {},
    "access": [],
    "meshPaths": [],
    "egress": {},
    "dns": {},
    "materialRefs": []
  }
}
```

示例数组／对象用于说明字段位置；完整编译结构需要结合当前执行合同定版，不能据此直接激活业务。

## Fields

| 字段 | 内容 |
| --- | --- |
| `hostname` / `addresses` | 配置规范名称与 Controller 分配的 Mesh 地址 |
| `listeners` / `dialers` | 合并默认值和节点设置后的本机执行参数 |
| `peers` | 本节点被允许使用的 peer 身份及可信公钥等必要信息 |
| `access` | 已编译授权；不是要求 Agent 再展开用户／组／Tag |
| `meshPaths` | 已授权完整路径及备选优先级、状态型回复约束 |
| `egress` | 本节点可执行的方案／规则、出口资格及目的授权 |
| `dns` | 本节点需要的解析规则和名称记录 |
| `materialRefs` | 当前发布绑定的材料；材料按节点权限通过私有载荷提供 |

配置和材料在激活前必须作为一致目标校验，不能将不匹配版本的材料与新配置拼装。digest 核对内容，不替代控制面认证。

## Config / State

本对象包含**执行许可与参数**。动态 endpoint、连接状态、当前方案选择、探测结果走 [State](/state-reference) 消息。

State 不能添加这里没有授权的 peer、扩大 access、改变可信公钥或绕过必经路径。相同源配置 revision 的节点切片也可能因身份／能力输入变化而有不同 digest；执行相关 State 必须绑定确切切片与运行实例。

## Validation

Agent 校验目标身份、完整 Schema、引用、材料绑定和本机构建支持，再检查端口／TUN 等实际资源。保存 ACK 与执行报告分开。

基础不匹配的增量获取全量；配置或授权撤回仍按 lease／代际规则停止旧执行，不因正在等待 State 而无限维持旧许可。详见[同步](/configuration-reference/synchronization)。
