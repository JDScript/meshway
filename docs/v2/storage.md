# 存储与访问

**V2 设计。** 以下页面逐字段枚举数据，不以分类摘要代替字段定义：

- [持久数据库](/v2/storage/database)
- [Controller 内存](/v2/storage/memory)

共同记法：`T?` 表示明确可空，`T[]` 表示数组，`Map<K,V>` 表示类型化 map；记录字段默认必填。它们是逻辑类型，SQL 列名／HTTP 编码仍需落定。数据库字段存在不代表对外 API 可读取。

## 持久边界

| 内容 | 放在哪里 |
| --- | --- |
| 协议参数、自动凭据与证书／私钥 | 完整 ConfigurationVersion，不再单独保存材料版本 |
| 一次性加入链接／接入密钥 | 私有摘要、角色／Tag／模板绑定及消费结果；原值不进入列表 |
| 管理操作历史 | ActivityEvent，与成功 mutation 同事务；与 Node 观测历史分开 |
| listener 创建代际 | 配置版本内的 bindings.listeners |
| 节点执行切片 | ExecutionPublication.nodes，发布与产物一次保存 |
| 应用回执 | ExecutionPublication.nodes 内，保存实际结果与来源 |
| 建网幂等结果 | Network 的原请求绑定，记录本身就是结果 |
| 历史去重 | ObservationEvent 主键／摘要；正文清理后有界保留墓碑 |
| 地址池与节点期望分配 | 完整 ConfigurationVersion；修改／重分配同样走 CAS |
| 旧地址退出窗口 | AddressRetirement，独立于配置回滚保留 |

完整 JSONC 与规范化配置分别保留编辑意图和确定值。执行发布固定按需路径／动态数据依赖，所以与源配置版本分别编号；永久吊销、实例隔离与租约也有独立生命周期。

## 读写路径

| 数据 | 写入 | 读取 |
| --- | --- | --- |
| 账户／成员／接入 | 有语义的事务操作，最终检查当前身份与权限 | 显式网络作用域，返回允许的字段 |
| Config | 准备完整目标 → 引用／语义校验 → CAS 保存版本与绑定 | 当前版本不可变缓存；历史按 revision 读取 |
| NodeConfiguration | 共享配置库编译 → 固定依赖 → 提交执行发布 | 按 Node／publication 取确定产物，Agent 严格校验 |
| State | 认证报告 → 实例／序号／发布校验 → 更新该 Node 内存值 | 当前时钟计算 freshness，按权限 query／SSE |
| History | 有界批量、事件 ID／摘要去重写入 | 时间窗口／源／对端过滤，稳定 cursor 分页 |
| SSE | 持久操作提交或内存事实变化之后发布 | full → ready → delta，断档 reset |

配置 identity 与地址期望由唯一配置版本解释。批准注册在同一事务创建 Node、消费批准，并写入 `nodes[id].identity`／`addresses`；身份切换与改址提交新的配置及必要依据，不更新第二份 Node.subject 或地址副本。删除 Node 同事务吊销凭据、移除配置、记录地址退出／执行撤回，保留最小拒绝依据。

停用使用配置 enabled；永久凭据吊销是独立认证事务，并撤回业务许可。两条路径都不能因安装失败或 HTTP 请求结束而恢复旧准入。

## 数据库执行

建议继续 SQLite／rusqlite，一个有界专用执行线程拥有连接／事务。异步 handler 提交操作；不跨 await 持 SQL 锁，密码 KDF、下载和大规模编译在事务外有界执行。

建议 foreign_keys、WAL、busy timeout 与身份／配置提交所需的同步持久性。现有源码使用 foreign_keys、synchronous=FULL 和 spawn_blocking＋Mutex；WAL／专用线程尚是提案。

必要的唯一索引、复合外键、session 摘要索引和历史时间索引正常建立。配置 JSON 的可重建查询索引按实测增加，初版按版本加载／缓存即可。

## 提交与缓存

1. 使用当前配置和固定身份／资源输入准备新目标。
2. 事务内重新校验身份、权限、expectedRevision 和输入仍有效。
3. 原子保存完整版本、引用绑定、必要持久记录及 current 指针。
4. 提交后按顺序切换期望缓存／发布完整目标，节点各自同步并应用；避免拼接新旧输入。崩溃恢复从已提交指针、快照和回执重建。

普通成员、节点 query 和 SSE 使用当前权限裁剪。身份变化使旧订阅范围和队列失效，不能让缓存长期代替真实权限。

Node 连续步骤／下载进度留 RAM，saved／applied／failed 回执经校验保存。旧回执在实例／租约失效后只作最后已知证据；重启后仍需确认当前执行许可和实际报告。

改址提交同时核对地址唯一性、退出限制和所有相关执行依赖，记录旧绑定退出下界。普通保存不擅自替换越界的显式地址，Web 重分配操作可以一次生成并提交合法完整目标；算法／缓存不是新的地址权威。

## 保留与恢复

| 内容 | 规则 |
| --- | --- |
| 完整配置历史 | 每个 revision 保留原文、有效值与固定依赖；回滚生成新 revision |
| 已消费接入 | 保留最小绑定／去重结果，首次兑换期限不是身份恢复期限 |
| 协议凭据／规则集 | 凭据值在配置内，数据集按历史绑定读取；吊销／缺失则回滚失败 |
| 执行产物 | 保留有效及恢复所需发布；重发同一发布必须同 digest |
| 地址退出 | 保留旧绑定安全退出下界；改址／回滚不复活旧流或提前复用 |
| 最新 State／SSE | RAM 有界；重启更换 epoch，重新 full sync，不延长旧观测 TTL |
| History | 正文按预算清理，同一记录保留必要接纳墓碑与独立覆盖缺口，不重复计数或补零 |

不可变资源初版可存 SQLite BLOB。改用文件时先原子持久落盘再提交引用；清理检查引用。备份同时保留一致数据库与材料主密钥，在线备份正确处理 WAL。整库恢复不能推知备份后的撤销，须单独协调节点信任／执行代际。

## 文件与客户端 {#local}

| 位置 | 字段／内容 | 访问方式 |
| --- | --- | --- |
| Controller 部署配置 | listen、dataDir、registrationEnabled、consoleDist 等 | 配置文件／环境加载，字段名待部署契约定版 |
| Controller 私有文件 | DB／锁／材料主密钥与必要 WAL／SHM | 私有目录；丢密钥报错，不生成替代冒充恢复 |
| Agent 接入 journal | controllerOrigin、私钥、公钥、pending 请求／PKCE、已绑定 networkId／nodeId | 首次网络请求前原子私有保存；pending 与身份一致恢复 |
| Agent 配置缓存 | publication、digest、完整 NodeConfiguration、所需材料 | 验证并持久保存后 ACK |
| Agent underlay journal | boot／netns／owner／用途／代际、route／NAT／sink 恢复义务 | underlay 库按资源所有权恢复 |
| Apple | 身份与 pending 使用 Keychain，其他缓存按平台规则 | 不用明文文件替代 Keychain 失败 |
| Console | session token、身份／网络隔离的缓存与草稿；cursor 绑定对应缓存 | 沿用既定会话存储；编辑原文／秘密不默认落 LocalStorage |
| 日志／临时文件 | 脱敏事件和中间产物 | 大小与保留有界，不作为配置或观测数据库 |

此处 Node 私钥与协议 UUID／password 是不同用途。数据库保存摘要的凭据不能反向恢复；需要重交付的一次性结果采用明确的短期加密记录，见[接入记录](/v2/storage/database#enrollmentrequest)。
