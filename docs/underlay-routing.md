# Linux socket 级系统保护通路

**Linux `managed-connections` 构建已将系统保护接入正常接入、控制同步与 TUN 生命周期。** 独立 namespace 中已验证正常 Controller/Agent 二进制、真实 TUN 包流和退出恢复；远端尚未部署，不将隔离验收扩称为所有宿主/VPN环境均受支持。

## 必须保持的边界

- controller HTTP、STUN、认证建连探测、transport 外层 socket 及其必要 bootstrap DNS，按内部用途获得保护，不进入 Meshway 自身 TUN。
- 不按 UID、整个进程、目标 IP 或域名全局豁免。普通业务访问同一地址仍受 grants、业务路由与必经节点约束；业务 `direct` 不能调用 protected factory。
- IPv4 / IPv6 均先设置并读回 `SO_MARK`，再 bind/connect/listen/send；失败关闭，不能重试 unmarked socket。TCP listener 必须 premark，accept 后读回只作验证，不能补救已经发出的 SYN-ACK。
- 保护只跳过 Meshway 自有捕获 block，继续原系统 RPDB；不强制查 main、不绑定物理接口绕过 VPN，不删除 foreign rule / route / mark。
- 配置、路径、公钥来自同 revision 快照；业务撤回与 lease 到期独立于 HTTP 请求 fence 旧 packet generation。controller 恢复通道与 carrier 使用独立生命周期。
- TUN 停止后保留 owned Mesh unreachable sink，包含本地与转发流量，不能落回底层默认路由。socket 保护不放宽 E2E 或路径授权。

## 当前实现

`apps/agent/src/underlay` 提供 namespace owner、用途受限 socket factory、HTTP/TLS connector、bootstrap resolver 和代际退休。正常 Linux daemon 在启动承载前安装 prepared owner，托管 Native UDP / TCP 与 VLESS 外层共用 owning 注入边界；未准备保护不能回落到 `SystemUnderlay` 继续执行。

Linux owner 要求 Linux 5.7+ 的非零 `SO_NETNS_COOKIE`、namespace 内 NET_ADMIN、TUN 和必要 iproute2 / nft / legacy-save 审计工具。应用 crate 保持 `unsafe_code = forbid`；cookie getter 的细边界位于受审依赖。

### 路由所有权

owner 从实际双栈 RPDB 分配共同空闲的连续 priority block、未被占用的 mark bit，以及不被 rule / route / rt_tables alias 引用的数字表。正向 mark guard 以 `goto` 跳过自有 capture，目标必须是实际存在的稳定 NOP。Mesh reject-only lookup 也在 guard 覆盖范围内，且不能含 default、gateway 或 TUN route；普通本地业务 capture 使用 `iif lo`，不把出口转发卷入本地方案。

未知 selector、先于 capture 消耗业务的非 local lookup、mark / alias / netfilter 冲突均拒绝激活并保留诊断。`not fwmark … iif lo` 会反转整条 selector，不能当作两个独立条件。RPDB 多对象更新不是内核原子事务；准备、捕获、恢复及 capability 交付须重检，外部并发变化的持续监控仍是集成边界。

### 持久恢复与退休

私有 `underlay.json` Journal v3 绑定 boot、非零 netns cookie、节点、网络和完整 numeric kernel 属性；dev/inode 仅作定位与一致性证据。目录 0700、文件 0600，随机临时文件、fsync、rename、目录 fsync。netns abstract AF_UNIX singleton 与 state lock 保证排他；退休后仍持有 wrapper 的 FD 继续持锁。

完整 operation intent 在首个 mutation 前持久化，包含 baseline、期望 resource set 与 Install/Rollback/Converge phase。不能用单条 `pending=None` 推断整组完成。只有整组双栈拓扑与 kernel evidence 验证通过才发布 capability；ambiguous pending 保留并拒绝，不按资源形状 adopt/delete。已确认未 mutation 的失败可精确 rollback，rollback 失败不返回可用工厂。

同 boot/cookie 恢复核对完整 ownership；新 boot/cookie 先审计新 scope，再私有归档旧 journal、初始化 fresh scope，不恢复旧 socket/packet generation。旧 v1/v2、缺失/零 cookie、身份不符均拒绝并保留。已消失的非持久 TUN 收敛为 sink-only，随后由新授权和真实 `TunBinding` 重建；不得把相同 name/ifindex 当作原设备。

退休 fence 覆盖应用 I/O 与公共 dispatcher admission，withdraw/join 后不再提交旧 ingress 或转发。不能召回已发到网络的字节，也不能把仍持有但已退休的 FD 描述为全部销毁。

### HTTP 与 bootstrap DNS

生成客户端通过 checked `HttpTransport` 注入受保护连接，保留生成 URL、bearer、status-first 和有界成功解码。HTTP/1 connector 使用受保护 TCP 后再 TLS，校验证书 / hostname / trust roots，不跟随 redirect，不支持 proxy 环境变量；取消、超时或 response drop 终止独占 driver。不得 fallback 到默认 reqwest send。

`SystemDnsSnapshot::from_system` 支持 plain ordered system/VPN upstream、`hosts: files dns` 的 `/etc/hosts` 优先级和真实 Docker `127.0.0.11` resolver。UDP 和截断后的 TCP DNS 都走同代际用途工厂；校验来源、transaction、question 与对应 CNAME/A/AAAA，缓存有界并按用途/代际与真实 TTL 隔离。只解析绝对名称，不扩展 search suffix。

systemd-resolved、split/per-link scope、其他 loopback daemon、NSS 扩展、scoped IPv6 和未支持 resolver options 明确拒绝；不能展平为公共 DNS，或把 `lookup_host/getaddrinfo` 当作 protected resolver。

## 正常主循环接线合同

内部入口是 `LinuxUnderlay::prepare`、独立 `controller_channel` / `socket_generation`、`retain_mesh_sink`、`Tun::open_with_underlay`、`withdraw_tun` 与 fresh-binding `activate_tun`。

无网络绑定的身份先使用窄 **guard-only bootstrap**，仅允许原 controller 上的生成 enroll、heartbeat、`get_agent_dns` 及必要解析。取得并校验真实 node/network 后，原子保存 `.network-binding.json`；退休并 join 窄 channel，在同一 namespace lease 下精确清理 bootstrap 规则，再准备严格绑定的 owner。不能手造网络 ID 或提前发放业务 capture、carrier、STUN 能力。

1. capture 前准备 owner、系统 DNS 与独立 controller channel，并注入所有 enrollment / heartbeat / material / DNS HTTP 消费者。
2. 安装独立 carrier generation，再用实际 TUN binding 发布自有 sink/capture。
3. endpoint / DNS / network 变化先准备新保护，再撤旧观测、fence/join 旧 carrier 并切换；业务 lease 不退休必要 controller 恢复 owner。
4. packet fence/join 后撤 TUN route，保留 Mesh sink；明确取消接管才精确 relinquish 自有资源。

`control` 统一提供受保护 HTTP，预先创建的 DNS consumer 共享同一替换点；每次业务 DNS 发布都校验持久网络绑定，不能仅依靠已有 DNS cache。`daemon` 负责单一运行循环，DNS 错误也须取消并等待该循环清理，保留原错误。中断 withdraw 的等待不得丢失原代际 join handle。CLI 输出使用有界队列和独立线程，不能因终端背压卡住网络清理或退出。

同进程需要立即关闭并重用身份时，`close_control_plane().await` 收拢 Controller driver；Drop 立即退休 I/O，但不是同步 join，仍持有的 capability/FD 会继续保持 namespace 排他锁。

既有原型 main route 与目标 `/32` pin 尚待迁移。必须证明真实 ownership 才能移除，不能按形状删除，也不能长期共存后声称普通同地址业务已受策略约束。

默认 STUN / 平台观察点、GeoIP、真实公网 NAT/打洞、换网与完整出口不由本模块验收证明。TCP/VLESS-only 的独立 UDP probe 必须有自己的 socket/source/generation，不能冒充 listener 映射或 TCP 候选。

## 重现与证据

```sh
CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=target/underlay-host cargo test --locked -p meshway-agent --all-features --lib underlay:: -- --test-threads=1
python3 -m unittest discover -s apps/agent/tests/underlay_fixture -p 'test_*.py'
python3 apps/agent/tests/underlay_fixture/main_loop.py
# 使用上述结果里的 image ID 复用同一编译产物：
python3 apps/agent/tests/underlay_fixture/run.py --skip-build --image <image-id>
python3 apps/agent/tests/underlay_fixture/review.py --skip-build --image <image-id>
```

fixture 的当前参数与环境见 `apps/agent/tests/underlay_fixture/README.md`；独立机制实验见 `experiments/underlay-routing/README.md`。Docker 测试使用同一 `scripts/performance_lock.py` 锁，仅独立 namespace、非 host network / 非 privileged，精确清理 owned 资源。

批准 TMP 内保留 `meshway-underlay-main-loop-runs/` 与 `meshway-underlay-artifacts/` 的源码/镜像摘要、正常包流、syscall/恢复证据和旧失败。镜像必须带完整输入路径/哈希 manifest；旧顶层文件校验不能覆盖迁移后的子模块与独立 crates。
