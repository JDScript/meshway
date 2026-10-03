# 存储、查询与外部 API

**架构提案，非现行数据库或 API。** JSON 定义逻辑配置，不要求每次查询都下载／反序列化整份文档，也不要求物理上永远只有一个大 JSON 单元。

## JSON 能不能只读一部分？

可以让数据库只返回子对象。例如 SQLite 支持 `json_extract`：

```sql
-- 示意：document 保存规范 JSON，source_jsonc 另存原始编辑文本。
SELECT json_extract(document, '$.nodes."example-node-id"')
FROM configuration_versions
WHERE network_id = :network_id AND revision = :revision;
```

**应用拿到的是一个节点，不是整份配置；但这不保证数据库只读了对应字节，也不保证查找是 O(1)。** 文本 JSON 可能仍需解析／遍历；SQLite JSONB 可减少解析开销，但多数操作仍是 O(N)。`json_each` 可以展开对象，却不会自动为每个 Tag 成员关系建立索引。

因此不建议将每次“按 Tag 找节点”“按 hostname 搜索”的高频 API 都写成扫描整份 JSON。

## 建议：完整版本 + 派生查询索引

```text
NetworkConfiguration 完整版本（唯一写入来源）
        │ 同一次版本提交中派生
        ├─ 节点设置索引：network / revision / node / hostname
        ├─ Tag 成员索引：network / revision / tag / node
        ├─ Group 成员索引：network / revision / group / user
        └─ 需要查询的 listener／规则引用索引
```

- 索引可从配置重建，没有独立业务身份或可单独编辑的权威内容。
- `Tag` 仍在 Configuration；数据库中有 tag 索引行，不等于重新引入 Tag Entity。
- 必须同时切换配置 current 指针与查询投影。投影落后时拒绝／明确报告，不能返回一半新一半旧。
- 常用读走索引；导出、历史比较、回滚读取完整版本。
- 内存缓存按不可变版本／digest 索引，遵守容量上限；不能只缓存一个会被原地改写的大对象。

查询可以只访问所需的投影行：

```sql
-- 示意：联合索引顺序为 network_id, revision, tag, node_id。
SELECT node_id
FROM config_tag_members
WHERE network_id = :network_id
  AND revision = :revision
  AND tag = :tag
  AND node_id > :after_node_id
ORDER BY node_id
LIMIT :page_limit;
```

源码版本与当前投影分开组织，避免每个历史版本都复制一套用于实时查询的索引。历史查询可以按需解析版本或构建有界临时投影；其成本与当前列表查询分别对待。

## API 不必等于“读写整个 JSON”

以下是操作形状建议，路径尚未定版；每个操作都检查当前登录身份和网络权限。

| 操作 | 返回／输入 | 读取来源 |
| --- | --- | --- |
| 导出配置／读取指定 revision | 完整配置、revision、JSONC | 完整版本 |
| 查询节点配置列表 | hostname／Tag 筛选、cursor、limit | 节点／Tag 索引 |
| 读取某个节点配置 | `NodeSettings`、所属 revision | 节点投影 |
| 修改某节点／某配置区段 | 类型化输入 + expected revision | 统一配置提交路径 |
| 获取当前 State／订阅更新 | 实例、sequence、时间、配置引用 | State 存储／订阅 |

局部写入也是对一份 NetworkConfiguration 的受控修改：校验影响范围和跨区引用、重新编译、CAS 原子提交并生成完整新版本。不能直接 `json_set` 后绕过权限、Schema、发布预算或 JSONC 草稿保全。

客户端不开放任意 JSONPath、SQL 或自由 `fields` 表达式；提供有界、类型化筛选和明确 DTO，沿用生成客户端／运行时 Schema。

### 分页与一致性

配置列表 cursor 绑定网络、revision、筛选条件和稳定排序。跨页使用同一版本；版本不再可读时要求重启查询，不能用旧 cursor 偷换到新版本。

如果只保留当前查询索引，翻页期间 current 已切换，则从指定历史版本进行有界查询／重建，或明确要求重新开始；不能因旧索引不存在返回一个假的空列表。具体选择和错误码随 API 定版。

State 列表是动态数据，要返回观测／接收时间和有效期，不冒充冻结历史。需要“配置 + 状态”组合展示时，分别标明配置 revision 与状态所针对的 digest／时间；失配显示未对齐，不补成已生效。

## 规模边界

Config／State 分离后，心跳、延迟和 endpoint 不会反复改写整份配置。局部 API 和节点切片减少网络传输；派生索引减少列表读取成本。

但完整保存、全局语义校验和策略编译仍可能随节点／规则数量增长。需要限制文档／条目／编译预算并测量真实成本，不能因为用了 JSONB 或 delta 就声称解决了所有扩展性问题。

若历史体积成为问题，可将不可变区块去重，用版本 manifest 重建完整逻辑配置；这属于后续物理存储优化，不把“只剩 patch 链”当作完整版本持久化，也不要求现在更换数据库。

参考：[SQLite JSON 函数](https://www.sqlite.org/json1.html)、[SQLite JSONB 的复杂度边界](https://www.sqlite.org/json1.html#the_jsonb_format)。
