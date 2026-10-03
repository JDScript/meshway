# 存储与访问

**V2 存储方案提案。** 推荐先使用一个 SQLite 数据库保存持久数据，版本化配置按版本缓存，最新 State 放内存。下列记录说明职责，不代表每行必须拆成一张 SQL 表。

## 总览

```text
SQLite
  ├─ 身份、成员、密钥绑定与地址
  ├─ 用户会话、接入请求／消费结果
  ├─ 完整配置历史、材料和资源版本
  ├─ 执行发布／实例隔离／恢复记录
  ├─ 用户使用选择
  └─ 有界观测历史

Controller RAM
  ├─ 当前配置与编译事实缓存
  ├─ 每网络一份最新 State
  ├─ 按需候选、DNS／GeoIP 派生缓存
  └─ SSE 重放窗口、订阅与短租约观察任务
```

“相对静态 Config、动态 State”是领域区分，不是“所有 State 都不落盘”。使用选择、反重放／恢复下界和必要回执具有跨重启语义；它们持久化也不进入管理员配置历史。

## 持久数据清单

### 身份与接入

| 记录 | 保存什么 | 读取／修改入口 |
| --- | --- | --- |
| User | 稳定 ID、邮箱、状态、私有密码哈希与 credentialVersion | 账户服务；公开视图不含哈希 |
| Network／Membership | 网络元数据、成员角色及加入／撤销记录 | 网络与成员服务 |
| Node／NodeKey | 归属、有效身份类别、公钥用途／代际、撤销与创建来源 | 接入／身份服务，配置只能受控触发身份转换 |
| IPAM | 地址池、节点地址分配与唯一性依据 | 网络创建／接入事务 |
| UserSession | token 摘要、User、凭据版本、期限和撤销 | 登录／鉴权／退出；不是纯 RAM session |
| EnrollmentRequest | 请求绑定、公钥、PKCE、返回方式、用户批准及期限 | 浏览器设备批准和 Agent 领取／兑换 |
| EnrollmentKey | token 摘要、网络、预分配 Tag、期限和消费绑定 | 网络管理签发及设备首次接入 |
| EnrollmentResult | 原请求摘要、已绑定 Node、消费结果与恢复依据 | 精确重试恢复；过了首次兑换期限也不能换身份 |
| 幂等记录 | 主体、操作、请求摘要和已提交结果 | 如网络创建；同 key 不同内容拒绝 |
| 身份变更记录 | 身份／密钥／成员变化的来源、时间与代际 | 与身份事务保存；用于追溯，不作为恢复旧授权的依据 |

未消费的临时请求可以到期清理；已消费绑定必须保留可防重放、恢复原身份的最小记录，不简单跟随兑换码 TTL 删除。记录不保存密码、verifier、token 原值或设备私钥。

当前不设计账户邀请注册记录。账户恢复若继续使用一次性 reset 凭据，其摘要、绑定、期限和消费记录仍应持久化；恢复入口的形式需单独定版，并不要求实例初始化 HTTP API。

### 配置与不可变依赖

| 记录 | 保存什么 | 读取／修改入口 |
| --- | --- | --- |
| ConfigurationVersion | 完整 source JSONC、规范化值、固定默认值、actor、revision／摘要 | 配置提交服务；历史只读 |
| 当前配置指针 | network → current revision | 与新版本原子提交，支持 CAS |
| 版本引用绑定 | UserId、NodeId、listener 资源代际、材料／数据集引用 | 配置准备和发布事务；与版本一起固定 |
| 资源生命周期 | 同名 listener 删除／重建、自动材料所属代际 | 配置提交内部维护，不成为独立 listener 编辑源 |
| MaterialVersion | 平台生成或管理员提供的协议参数／证书等必要材料、版本、吊销 | 材料服务；秘密加密保存 |
| RuleSetArtifact | 来源、下载与规范化内容、摘要、转换结果和版本 | Controller 下载／编译，发布固定引用 |
| GeoIP 数据集 | 数据版本、来源及可重建查询内容 | Controller 派生地区；不由用户 country 覆盖观测 |

初版建议不可变内容直接存 SQLite BLOB／JSON，复用事务和备份，不必先建设对象存储。大型数据集以后可用内容寻址文件；一旦采用文件，须先私有原子落盘并确认持久，再提交 DB 引用，清理只移除无引用内容。

包含明文协议秘密的 source JSONC 和规范化值同样需要加密，不能只加密自动生成材料。密码哈希／token 摘要与可解密材料是不同存储方式；哈希不能用于恢复协议密码。

材料是配置版本依赖，不是第二份可独立修改的配置。重复内容可引用去重，管理员编辑、自动生成和轮换最终都通过新版本绑定。历史资源缺失／吊销时回滚明确失败。

### 执行、使用选择与历史

| 记录 | 保存什么 | 读取／修改入口 |
| --- | --- | --- |
| ExecutionPublication | 持久顺序、源 revision、身份／材料／规则集依据、参与节点 digest、撤回依据 | 编译发布服务 |
| NodeConfigurationArtifact | 已实际发布的完整切片及摘要；必要材料受保护 | 发布／重发；建议保留有效和恢复所需产物，不为每个源版本预建所有节点对 |
| NodeInstanceFence | 当前实例与单调代际、被替代实例的拒绝依据 | 实例 acquire／resume 事务 |
| 配置保存回执 | Node、实例、发布、digest、保存时间 | ACK；最后一次 saved 不等于重启后仍 applied |
| RoutingSelection | Node 当前有效主体的用户选择及选择版本 | 受 allowed／locked 约束的使用操作；建议 Controller 持久保存、设备缓存 |
| ObservationHistory | 真实连接／测量事件、方向、实例／发布、事件时间、接收时间、缺口 | 批量去重写入与有界查询 |
| 历史接纳／覆盖依据 | 已接纳 ID／摘要、重试窗口和历史截断范围 | 防重复计数及解释 retention／容量缺口 |

执行产物虽可从固定输入生成，原发布重发仍应得到相同内容和 digest。编译器／算法升级后若不能重现原字节，则创建新发布，不能覆盖旧发布标识。有效引用与恢复下界保留完毕后，才清理旧切片；配置历史按完整版本保留。

用户方案选择绑定当前主体，身份转换后不得把前任用户的选择当作新用户意愿；恢复仍检查 allowed／locked。它不随管理员配置回滚倒退。

## 内存数据清单

| 数据 | 索引／读取 | 重启后 |
| --- | --- | --- |
| 当前 Configuration | `NetworkId → Arc<version>`；有界历史版本缓存 | 从完整版本重载 |
| 编译事实／按需候选 | 源版本＋实际身份／资源依赖 | 重编译或从有效发布恢复；不能复用过期授权 |
| NetworkState | `NetworkId → NodeId → report + receivedAt + freshness` | 换 stateEpoch，等待 fresh 报告 |
| 能力、接口、socket endpoint、links／paths、ACK／applied 最新视图 | 随所属 Node／资源代际归入 State | 历史最后值不伪装成当前运行 |
| STUN／协调 attempt、拨号计划运行状态 | 绑定执行发布、实例、socket／候选代际，有期限 | 重新协调，不复活旧 attempt |
| DNS 缓存／域名解析关联 | 网络、发布、源范围、名称／地址和 TTL | 缓存重建；扩大授权的解析关联须重新受控发布 |
| GeoIP／自动方案派生值 | 实测底层公网 IP＋数据版本＋期限 | 等有效观测；没有时走已定义 fallback |
| SSE cursor／事件窗口 | 当前视图、epoch、授权上下文与有界队列 | reset 后 full sync |
| Node 认证挑战／短期控制凭据（若握手采用） | 公钥绑定、唯一挑战、上下文／期限及实例 | 未完成挑战失效并重新持钥认证；不能因 RAM 丢失接受旧证明 |
| Realtime observation | 任务、订阅者权限、实例、短租约与最新样本 | 旧任务不自动续租；UI 可重新创建，已保存样本仍可查 |
| 速率预算、下载中状态、编译队列 | 各用途的有界内存结构 | 重建；不作为新的业务授权依据 |

内存是最新观测的权威缓存，Node 报告是其事实来源。每次查询仍按当前时钟计算过期，SSE 到期事件即使无新上报也要产生。State 丢失不影响持久撤销／membership／实例隔离下界。

## Controller 访问路径

### 数据库

沿用 SQLite／rusqlite，推荐一个**有界专用数据库执行线程**拥有连接与事务。异步 handler 提交类型化操作，SQL 不运行在 Tokio executor 上。首次实现保留单写入序列；读连接池在实际瓶颈需要时增加。

建议启用 foreign_keys、WAL 和合适的 busy timeout，并保持身份／配置提交所需同步持久性。当前源码已使用 foreign_keys、synchronous=FULL 与 spawn_blocking＋Mutex，但没有显式启用 WAL；专用线程和 WAL 是本轮提案。

数据库接口是有语义的事务操作，不是让 handler 任意拿连接或逐表拼接 CRUD。必要的唯一索引、复合外键、session 摘要索引与历史时间索引正常建立；“先不做派生索引”只针对完整配置 JSON 的冗余查询投影。

### 配置读取／修改

1. 已鉴权读取复用对应版本的不可变对象，只序列化有权查看的部分。
2. 保存时在事务外完成有界解析、规范化、资源准备和纯编译。
3. 写事务重新检查 session、成员权限、expectedRevision 及全部身份／资源输入仍一致。
4. 同事务提交完整版本、依赖绑定、必要身份变化、发布记录与 current 指针。
5. 提交后交换当前缓存并通知执行发布／SSE；崩溃恢复从已提交指针重建。未完成缓存切换期间不发布拼接的新旧视图。

不跨 await 持有 SQL 事务，也不在写锁内做密码 KDF、远程下载或大规模编译。准备失败与 CAS 冲突不发布新版本。

### State／SSE

Node 认证与实例／发布校验 → 接受当前序号的报告 → 更新对应 Node 的内存值 → 推进 State 版本 → 当前权限裁剪 → SSE。上报不反复写完整 Config，也不复制整张 NetworkState。

身份／权限提交后，要使旧读取上下文和订阅失效。Config 初始快照截点、State 与后续事件使用一致发布顺序；积压无法补齐就 reset。数据库权限缓存不能无期限代表真实 membership。

### 历史

采集队列有界，批量入库。按 `(networkId, sourceNodeId, eventId)` 去重并核对内容摘要；相同 ID 不同内容拒绝。历史已裁剪后仍须在接纳窗口内防止重试重新制造样本。

查询按网络、源／对端、时间窗口、测量种类与稳定 row cursor 分页；固定查询上界，迟到样本通过下一次查询获取。历史权限使用当前身份，不因事件曾可见就永久可读。retention／容量裁剪记录真实覆盖缺口，不补零。

## 文件与客户端存储

| 位置 | 数据与访问 |
| --- | --- |
| Controller 配置文件／环境 | 监听地址、数据目录、注册开关、静态资源目录等部署参数；不进入网络配置回滚 |
| Controller 私有数据目录 | SQLite、锁、材料加密主密钥；WAL／SHM 也受目录权限保护 |
| Agent 私有接入记录 | Controller origin、本次接入密钥身份、公钥／Node 绑定、pending 登录／兑换状态；首次请求前原子保存 |
| Agent 配置缓存 | 完整 NodeConfiguration、发布／digest 与必要秘密材料；保存 ACK 以真实持久成功为准 |
| Agent 系统恢复 journal | 网络 namespace／boot／用途／owner、route／NAT／sink 等恢复义务；underlay 拥有，不能混进 State 缓存 |
| Agent 运行 RAM | socket、连接、加密会话、conntrack、DNS cache；崩溃后不从观测历史重造活动流 |
| Apple | Keychain 保存身份／pending，Rust core 消费生成客户端；继续保留平台私有存储边界 |
| Console | 既定不透明会话存储；当前网络缓存、SSE cursor 与草稿按身份／网络／revision 隔离；协议秘密不默认写 LocalStorage |
| 运维日志／临时文件 | 脱敏、有大小／保留上限；不作为身份、配置或观测历史的权威存储 |

备份需要一致的数据库内容和对应材料主密钥。在线 SQLite 使用受支持的备份方式，不能只复制主 DB 而遗漏有效 WAL；备份中的已撤销依赖不因此恢复有效。材料主密钥丢失报错，不能新生成一把假装已恢复。

配置回滚与整库灾难恢复不同。旧备份无法推知备份之后发生的撤销；整库恢复必须单独协调当前节点信任与执行代际，不宣称恢复旧文件就保留了所有后续撤销。

## 现状与下一步

当前权威结构在 `apps/controller/src/store/schema/schema.sql`：Node 行仍含名称／Agent 版本／last_seen／ACK，Tag、listener、策略源分别保存；runtime reports 与延迟历史已有持久部分，endpoint discovery 放 Store 内存。**当前不是本页的完整版本 Config＋独立 RAM State 结构。**

V2 实施时按[字段归属](/v2/entities#字段归属速查)收敛，保留身份、权限、原子消费及恢复回归。表名、DML 和数据库版本将在实体／存储方案确认后设计；本页不授权清库或修改运行服务。

配置查询细节见[存储、查询与外部 API](/v2/configuration-reference/storage-api)，发布边界见[同步与分发](/v2/configuration-reference/synchronization)。
