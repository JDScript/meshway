# Linux 跨地域 Docker 验收

**固定 `4c9a5be` 的同身份直连与四地有序路径恢复复验均 PASS。** 验收走正常 Controller 下发、Agent 启停与真实 TUN ICMP / HTTP / DNS；它验证固定底层可达条件下的 Mesh、E2E、多跳与恢复。

## 已取得的证据与限制

- 东京、纽约、首尔、洛杉矶的固定链路双向包流及必经节点故障约束通过；必经点停止时没有绕过。
- 复用原 Controller DB、身份、公钥、CA、源配置，双向分别预热存活端并仅重启目标；无重新注册、策略重置或旧快照回写。
- 四节点 controller 失联后 lease 到期、packet proof 撤回、Mesh sink 保留；业务失败，未落回底层默认路由。TUN 设备可仍存在，不能把 proof 撤回写成设备已删除。
- 恢复后继续取得跨至少 60 秒的新 runtime 报告，并再次验证业务。**持久 latency history API 未查询**，不据 runtime 更新证明历史查询通过。
- 两轮分别精确清理测试容器与 bridge，错误 0；私有 DB/身份/CA/源配置和证据保留。采样含启动、调度与应用请求，不是连续断流上界、握手时延或 SLA。

原 `4d106e0` 现场 FAIL 与 `f399931` 组合 FAIL 保留。原 E2E 重启窗口和运行报告/首历史样本问题已分别修复，新的复验是独立结果，不能改写旧失败。[E2E 恢复合同](/e2e-session-recovery)保留新旧 wire 与 TTL 边界。

**NYC 位于防火墙后；原 KR→NYC 转发 FIB 为 CloudflareWARP/table 65743。** Docker 发布端口不证明上游防火墙放行，应用互通不能反推出纽约普通公网可拨入。第一轮没有证明纯公网直连、Meshway 自身 NAT 穿透、自动建连、主备或逐包 trace。

批准 TMP 的必要证据入口：

- `meshway-field-retest-4c9a5be/`：`recovery-summary.json`、`postflight-comparison.json`、`direct-attempt2/retest-result.json`、`ordered-attempt/retest-result.json`。
- `meshway-field-r1-5038bd8a8e3f/`：原 FAIL、完整失败窗口、FIB 与 cleanup/postflight。
- `meshway-field-recovery-integration-f399931.md` / `-logs/`：中间组合原失败；Git 保留修复与复核历史。

## 部署与构建边界

| 主机别名 | 架构 |
| --- | --- |
| `jp-tyo` / `kr-icn` | ARM64 |
| `us-nyc` / `us-lax` | AMD64 |

别名仅是测试库存，不是实测地图定位。凭据、连接地址与持久身份独立私有保存，不写入文档。

后续联调优先 `https://meshway-dev-api.jdscript.dev`（本机 Mac `:8420`）。四地正常 DNS/TLS 已可访问，生成 GET/schema 核对为同一本机网络；curl 入口检查不等于原生 Agent 接入验收。原 LAX Controller 是隔离停机/恢复测试安排，不是产品部署要求，其数据与 Mac 开发网络分开保留。

构建优先 Mac 的 Linux ARM64 环境与 NYC AMD64，固定源码和摘要。仅普通非 root、cap-drop-ALL、CPU/内存有界的编译容器，使用既有普通 builder；不创建 privileged buildx builder，不用 host-network/insecure entitlement，不修改 Docker daemon。原首轮两个 privileged builder 的执行偏差已披露并精确移除、缓存和 inspect 保留；原镜像/FAIL 不改写。

运行容器只在独立 namespace 开放必要 NET_ADMIN/TUN，不用 host network/privileged。不手改宿主路由、DNS、防火墙或 daemon，保持 SSH 与现有服务；测试网段、端口和目录先核对冲突，按 owner label/精确 ID 清理。部署升级保留节点身份和私有状态，listener/凭据/路径由生成管理 API 配置。

## 后续验收

1. **系统保护接线**：正常主循环准备受保护 controller/carrier 与真实网络绑定，先保护后捕获，验证重启、endpoint/DNS 更新、lease/撤回与 Mesh sink。模块 fixture 通过不代替此项。
2. **真实自动组网与 NAT**：两端仅出站，无发布端口/手工对端候选；Controller 下发、认证探测、实际转直连，UDP blocked 经允许中转仍保持 E2E/Access/必经节点。按 UDP/TCP socket 能力、反向拨号与中继分别记录，不能把 WARP 或 STUN mapped endpoint 算作 Meshway 打洞。
3. **四地恢复与观测**：直连/有序路径、反向 dial、主备与回切、端点重绑定、重启、controller 失联/lease、授权撤回；保留失败窗口，独立查询真实 latency history。
4. **完整出网**：正式下发公共/方案/兜底、域名/IP、出口开关与授权，验证 NAT/状态回包、DNS一致、局域网独立授权、切换与无泄漏。它仍是首版必需闭环。

runner 与参数见 `experiments/linux-field/README.md`（当前 `run.ts`）。Docker 功能/性能实验使用共享 `scripts/performance_lock.py`；每次记录源码、镜像摘要、拓扑、revision、命令、观测时间、结果与精确清理状态，不恢复旧宿主快照覆盖用户工作。
