# 存储、查询与外部 API

配置相对静态。初版采用**完整版本入库、按版本加载、内存查询**，不预先引入多套派生数据库索引。

## 完整配置与缓存

```text
数据库中的完整配置版本
        ↓ 启动／版本变化时读取、解析、校验
不可变的当前版本对象
        ├─ 编译与逐节点切片
        ├─ Node／Tag／Group 查询
        └─ 管理 API 的筛选与分页
```

同一版本复用解析结果，不在每个 API 请求重复读库、解析大 JSON，也不为每个请求克隆完整配置。新版本准备成功后原子替换当前引用；并发读取继续持有一致的旧版本。

一万个节点若平均每项 1 KiB，节点部分原始 JSON 约 10 MiB；这是量级估算，不是实测容量承诺。解析对象、材料、规则集和编译结果另占空间。先测真实配置的加载时间、峰值内存、编译成本与 API 延迟，再决定优化。

Config／State 分离使 heartbeat、endpoint、连接和 RTT 不会反复改写配置。控制面可以读取整网配置，但节点分发仍使用必要切片；“能完整读出”不等于向每个节点发送全网数据。

Console 初次进入网络时允许加载当前权限可见的完整 Config／State，已选择由 SSE 先发送 snapshot 再推送变化。整份初始化与按需局部读取可以共存；History 和秘密材料不默认广播，见[SSE 同步](/v2/api/events)。

## 查询 API

JSON 是配置格式，不限制 API 必须整份读写。

| 操作 | 行为 |
| --- | --- |
| 导出配置／指定 revision | 返回完整配置及版本，按权限提供原始 JSONC |
| 节点配置列表 | 在该版本对象中按 hostname／Tag 筛选，稳定排序并分页 |
| 某节点配置 | 以 Node ID 查 map，只序列化对应 NodeSettings |
| 某 Tag／Group 的成员 | 初版可扫描当前不可变对象；需要时构造同版本内存索引 |
| 局部修改 | 类型化字段更新 + expected revision，仍走完整配置提交路径 |
| 当前 State／订阅 | 从独立 State 存储返回，并携带实例／序号和新鲜度 |

每个操作仍检查当前身份与网络权限。分页 cursor 绑定网络、配置 revision、筛选条件和排序；不能在翻页时偷偷切换版本。历史版本缓存被回收后可以从库中重读，或明确要求重新查询，不伪造空列表。

局部写入不是直接修改内存对象：生成新目标，校验引用与语义，CAS 事务提交完整新版本，并保留 JSONC 编辑内容。access／topology 规则没有 ID，操作以本版本数组位置或整段替换定位；remark 不是主键。

## JSON 的局部读取

SQLite 支持只返回子对象：

```sql
-- 示意表名；只向应用返回该节点。
SELECT json_extract(document, '$.nodes."example-node-id"')
FROM configuration_versions
WHERE network_id = :network_id AND revision = :revision;
```

数据库内部仍可能解析／遍历大 JSON；SQLite JSONB 可减少解析开销，但多数操作仍为 O(N)。对已经加载到内存的当前版本，没有必要每次再走一次 JSON SQL 查询。

上述 SQL 适用于未加密的 JSON 内容；加密的配置／协议材料先在有权限的应用路径中解密，再按版本缓存与查询，不能直接对密文使用 JSON 函数。管理员提供的秘密和含有秘密的 JSONC 同样需要存储保护，普通成员 API 不因此获得完整材料。

## 何时增加索引

只有实际查询成本需要时，再加 Node ID、hostname、Tag 成员等**可重建、按版本绑定的索引**。它们是查询加速结构，没有独立写入口，不是新的 Tag／Link Entity。

未来若持久化索引，必须和配置 current 指针原子切换；数据不能成为另一份权威配置。历史区块去重、JSONB、增量编译也是可测量后的优化，不是初版前置条件。

大规模瓶颈还可能来自规则展开、全对全路径或单节点投影预算，并不都在 JSON 读取。现行容量见[协议](/protocol)，不能根据原始文件大小推导数据面支持的节点上限。

参考：[SQLite JSON](https://www.sqlite.org/json1.html)、[JSONB 复杂度](https://www.sqlite.org/json1.html#the_jsonb_format)。
