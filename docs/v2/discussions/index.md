---
title: 设计讨论
description: 从当前源码、实测与外部实现出发，审视下一步设计选择。
---

# 设计讨论

这里用于共同审视设计：把**已核对的实现、测量边界、候选方案和待决定问题**放在同一处，先说清楚要保留什么语义，再判断值得改变哪一层。

讨论中的候选方案均未因写入本文档而成为可用 API 或已交付功能。现行行为仍以对应的 [Node](/v2/node)、[Configuration](/v2/configuration-reference) 和 [State](/v2/state-reference) 参考为准；结论确定后更新唯一规范位置。本区保留问题与论据，逐轮聊天和原始运行日志不作为正文累积。

| 主题 | 我们要回答什么 | 状态 |
| --- | --- | --- |
| [Packet：字节账本与固定开销](/v2/discussions/packet) | 每个字段多带多少 bytes、由谁验证、能否移入上下文？如何让开销不随网络／路径规模增长？ | 讨论中：规模约束与条件预算 |
| [包流分层、封装与加密](/v2/discussions/packet-layers) | 一个包经过哪些 owner、多少封装和密码操作？哪些成本来自安全语义，哪些来自实现组织？ | 讨论中 |
| [EasyTier 源码对照](/v2/discussions/easytier) | 同样是 overlay / relay，EasyTier 的帧、密钥、buffer 和调度如何组织？哪些可以类比？ | 固定提交源码研究 |

建议从 [Packet 字节账本](/v2/discussions/packet)开始，再结合[包流与待讨论问题](/v2/discussions/packet-layers#待讨论问题)看各字段承担的产品语义与运行成本。每项决定至少记录：目标语义、所依据的源码／测量、接受与排除的方案、需要补充的验证。
