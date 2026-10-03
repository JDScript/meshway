---
title: State Reference
description: 与持久配置分开发送的运行状态草案。
---

# State Reference

**状态模型草案。** State 频繁变化，主要以 RAM 中的最新事实存在；需要查询的历史按明确期限单独保存。

## Structure

```jsonc
{
  "networkId": "<network-id>",
  "nodeId": "<node-id>",
  "instanceId": "<runtime-instance>",
  "sequence": 18,
  "configurationRevision": 42,
  "nodeConfigurationDigest": "<node-config-digest>",
  "observedAt": "2026-10-03T12:00:00Z",
  "expiresAt": "2026-10-03T12:00:30Z",
  "state": {
    "reportedHostname": "linux-host",
    "capabilities": [],
    "listeners": {},
    "links": {},
    "paths": {},
    "selectedProfile": "mesh",
    "problems": []
  }
}
```

本例是执行状态消息的结构示意；引导阶段尚无配置时的状态应有明确的独立类型，不能填写假 revision／digest。

## Fields

| 内容 | 来源 | 含义 |
| --- | --- | --- |
| `instanceId` / `sequence` | 当前运行实例／发送者序号 | 排序并拒绝旧实例消息，不能当作配置历史版本 |
| 配置 revision／digest | 执行上下文 | 本次状态针对哪份许可与参数 |
| `observedAt` / `expiresAt` | 观测源及规定的有效期 | 保留实测时间；Controller 另记接收时间 |
| `reportedHostname` | 操作系统报告 | 不覆盖 Config 中的规范 hostname |
| `capabilities` | 当前构建 | 实际支持的协议、监听／拨号等能力，不等于管理许可 |
| `listeners` | 资源 owner | 实际 bind、就绪／错误、所加载材料的公有指纹 |
| `links` | 已认证通道 owner | peer、承载、实际 socket／代际、建连方向、证据与期限 |
| `paths` | 业务执行运行时 | 会话选择／使用的有序节点路径、来源与结果，不冒充逐跳 trace |
| `selectedProfile` | 用户选择／自动地区结果 | 必须在 Config 允许范围内，不能增加出口授权 |
| `problems` | 明确失败事件 | 类型化原因与上下文，不把缺报告当成具体错误 |

namespace、socket ordinal、候选验证时间等详细字段待执行侧接口统一后定版；不按 listener 推测所有拨号 socket 的归属。

## links

当前已认证承载通道的观测。配置中的 topology 只给出允许的图；允许 A—B 不表示 State 中已经有 A—B。未知、过期或缺少认证证据时不能补成已连接。

## paths

运行时为业务选择／使用的逻辑路径。routing 定义路径约束，State 报告对应执行事实；保留观测来源、配置引用及发送／确认状态，不能拿配置预览充当真实 trace。

## 分开发送

- Agent → Controller：实际能力、资源与执行报告、测量事件。
- Controller → Agent：当前配置许可下需要的 peer endpoint、协调 attempt／lease、使用选择等动态更新。
- Controller → Console／外部 API：经权限过滤的状态投影，保留来源与新鲜度。

这三类载荷不是同一份 JSON 的无条件转发。Config／State 是不同消息与版本语义，可以复用一条传输连接，具体 HTTP／stream 方式待确认。

配置依赖尚未安装时，执行相关 State 不可使用；有界等待或重取配置。旧 epoch、过期状态和无权 peer 不能激活连接。读取、重发和浏览器轮询不刷新原始观测时间。

Agent 自报告不是授权来源：User／Tag 归属、可信公钥绑定和 access 由控制面的身份／配置决定，不接受设备自行声明后直接提升权限。即使发送节点修改自身配置或报告，接收节点仍依据自己的可信配置执行[访问检查](/configuration-reference/access#enforcement)。

## 测量

建议按来源区分邻接探测耗时、端到端路径 RTT、建连耗时和 Controller 通信耗时，不汇总成一个无来源的“节点延迟”。

测量事件需要独立 ID、方向、实际承载／路径、开始／完成时间和结果；超时／缺测不填零。历史订阅消费真实事件，不能靠复制当前状态生成曲线。

当前实现仍有 5 秒健康探测与 30 秒历史采样两条主动入口；底层探测超时还会退休对应通道。**统一测量来源、让历史记录者不直接管理通道生命周期，是本轮建议而非已完成实现。** 当前行为见[延迟历史](/latency-history)。

## 持久性

最新 State 失效后保留“过期／未知”，不能因从数据库恢复就重新当成在线。选择偏好、身份撤销和必要恢复记录可持久化，但不作为管理员配置回滚的一部分。
