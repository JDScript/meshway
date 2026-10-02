# v11：账户、设备归属与延迟历史统一迁移

状态：已合入正式启动路径并完成本地 v9→v11 升级；完整消费者检查与独立迁移审查通过。
真实副本演练保留 15 张旧表和 4 个节点的原值，重启不改写已知起点；现网升级前另取最新备份，
并核对身份、配置、tags、网络 revision 均未变化。不会自动创建账户或认领节点。

## 迁移顺序

- 旧版本沿既有迁移到 v9，再执行已验证的 v9→v10 托管承载迁移。
- `store/release_schema.rs` 在一个 `IMMEDIATE` 事务内安装账户/ownership 与延迟历史片段、
  完整验证结构与外键、建立必要的历史启用下界，最后设置 `user_version=11` 并提交。
- v11 重启只验证已安装结构，不用 `IF NOT EXISTS` 修补缺失约束或触发器。
  账户与历史的分支专用启动安装 hook 已移除；`latency-history-fixture` feature 仅保留
  旧测试命令兼容，正常构建同样执行 v11 迁移。

## 数据与兼容边界

- 旧节点 ID、凭据摘要、公钥、地址、tags、配置/JSONC、revision、回执和 runtime lease 不改写。
  旧节点归属初始化为未认领，不生成用户，不自动给首个 Owner 分配设备。
- v10 的材料设置、nonce 与密文字节保留。真实控制面备份必须同时保留数据库、
  `admin.token` 和已有 `transport-material.key`；不得丢失材料密钥后生成替代密钥。
- 可采用结构完全匹配的未版本化开发片段，并保留其账户、历史记录、接收游标和已知 epoch 起点。
  partial/未知结构明确拒绝，不能仅凭表名接受或清空后重建。
- 迁移前已有 runtime lease 但没有历史起点时，以本次迁移时间建立保守的历史启用下界。
  不虚构旧探测，不刷新已有起点；之后新的 Agent 实例照常取得新 epoch。
- 任何安装/校验失败回滚本次 v10→v11 的全部改动和版本更新。早期已完成的 v10 迁移可在
  重启后继续，不将多阶段历史迁移或文件系统密钥写入声称为单个跨资源事务。

## 组合权限

历史查询复用账户的 `network_operation(Read)`，在同一事务内验证当前 session/membership
并读取记录。Member 可读本网历史，跨网或退出网络后拒绝；bootstrap 管理兼容入口保留。
节点上报仍只接受节点身份，不把人类 session 用作节点凭据。

## 验证入口

- `store::release_schema::tests`：独立 v10 SQL fixture 的逐单元格保留、初始化归属、历史起点、
  已知未版本化片段的游标/字节保留、partial history 回滚账户 DDL、v11 重启拒绝缺失 guard。
- 既有 v1/v2/v3/v5/v6/v8 升级及独立原始 v8 fixtures 继续核对原字段、身份和配置。
  历史 fixture 显式排除 v11-only 对象，不能把新表的初始化内容当作旧表发生改写。
- `accounts::history::historical_reads_use_current_network_membership_without_requiring_manage_permission`：
  真实 HTTP 的 Member/Owner/bootstrap 读取与退出/跨网拒绝。
- 真实旧数据副本演练、生成产物、Agent、Apple core、Web/SDK 和实验消费者检查已完成；
  Web UI 后续在开发页面手工验证，不再运行浏览器自动化套件。
