# P1c · 隔离 Linux 出口验收

真实 Rust Agent 内部 `ExitPlan`、Noise hop + endpoint E2E、Linux TUN、nftables masquerade、kernel conntrack。与 controller 身份/数据库完全独立，通过 ignored tests 注入内部计划，不提供生产设备配置入口。RFC见 `docs/exit-dataplane.md`。

完整成功证据、前缀表耗时与短基线见 [RESULTS.md](RESULTS.md)。

## 执行

所有命令在仓库根目录执行。Docker build、verification及验收均经过共享performance lock，长任务使用独立 `meshway-exit-*` tmux session。

```sh
export MESHWAY_PERF_LOCK=/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/meshway-performance.lock
python3 scripts/performance_lock.py docker build \
  -t meshway-exit:p1c -f experiments/exit-dataplane/Dockerfile .
python3 scripts/performance_lock.py docker build \
  --target verification -t meshway-exit:verified -f experiments/exit-dataplane/Dockerfile .
python3 experiments/exit-dataplane/launch.py \
  --image meshway-exit:p1c \
  --artifacts /private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/meshway-exit-new-run
```

`--cases`可选单项重验；默认完整矩阵。artifact目录必须没有 `result.json`，防止覆盖已有证据。`launch.py`使用共享 `locked_launch`（fixture FD为 `MESHWAY_EXIT_PERF_LOCK_FD`），转发SIGINT/SIGTERM并持锁等待清理；`run.py`使用共享 `performance_lock(inherited_env=...)`，已有 `MESHWAY_PERF_LOCK_FD`时复用描述符，直接调用也会使用同一个规范锁。所有Docker子命令和namespace字节传输继承FD，拒绝锁外运行，不另建锁实现。Docker namespace sysctl为1用于常见只读/proc/sys，生产函数仍尝试设置与验证；无权限真实报错。

无Docker的CI单测：`python3 -B -m unittest discover -s experiments/exit-dataplane -p 'test_*.py'`。覆盖锁外拒绝、launcher继承不嵌套/不释放父锁、子命令FD传递与精确所有权清理。

## 拓扑与条件

- C: `172.29.231.10`；R可选，两个完全分离的underlay网络；E外网 `172.29.233.10`。
- S只连接外网，地址 `.20`、额外服务地址 `.21`；仅E（双出口用例还包括F）可直达。
- F外网 `.11`；L仅在C本地underlay网络，用于本地路线/direct排除验证。
- IPv4 Mesh池 `198.19.90.0/24`、节点完整64字符ID、MTU1000、NativeUDP、2 Tokio workers、每容器1 CPU/256 MiB、单TCP流3秒基线；未作优化。
- 自有 `meshway-exit-<uuid>-<case>-*`资源和owner标签；`172.29.231/232/233`三个网段必须未被其他Docker网络占用（冲突创建失败并精确清理）。所有网络`--internal`，各容器删除自己的默认物理路由；不允许Docker网桥跨网络成为备用出口。
- NET_ADMIN / NET_RAW / `/dev/net/tun`，无privileged / host network，不改宿主路由、DNS、firewall、daemon。私钥仅在私有临时目录/容器`/run`保存，不写result。

## 验收矩阵

1. `direct`：C→E→S、服务端看到E外网地址；未匹配L地址走C本地；认证路径内伪造inner source拒绝；物理非Mesh源不能把E当开放路由器；E直达/C隧道的RTT与iperf短基线。
2. `relay`：C→R→E→S；无C→E物理路由，停止R后业务失败，不能绕过；同条件短基线。
3. `unauthorized`：无ExitGrant的源失败（不能从Mesh路径grant推导Internet授权）。
   `internet-only`：相同默认捕获只授予公网权限时拒绝172.29.233/24出口LAN；其他成功用例显式授权该网段后可访问并观察NAT源。
4. `default`：非具体服务地址走默认出口，peer endpoint物理pin不被捕获，隐式LAN访问拒绝，IPv6 default unreachable。
5. `withdraw` / `lease`：真实持续TCP echo先成立；撤回/无刷新lease到期后应用流终止，新流失败，代际mark conntrack清空。
6. `kill-client` / `kill-exit`：SIGKILL真实Agent测试进程，非持久TUN消失；持续流及新流失败，客户端捕获sink不恢复物理LAN，出口保留拒绝边界。
7. `dual`：允许方案E/F整表原子代际切换，revision不变；未授权ID拒绝；旧流不迁移，S看到F地址；公共规则仍走E，方案F的direct也生效；“仅Mesh”恢复其余本地，其后公共规则仍走E；再选E后旧F的direct消失。auto模拟controller的JP位置解析→F、未知位置→仅Mesh兜底。
8. `locked`：切换其他方案或仅Mesh均拒绝，现有方案仍工作。
9. `direct-exclude`：1000个direct /32在default下走本地；全部1000个route lookup不落TUN；两张自有表fence-backed原子代际替换，旧direct前缀转为default，新direct恢复本地；明确撤回批量表。`install.json` / `replace.json` / `remove.json`毫秒实测进入result。

`result.json`提供顶层 `status`（ok/failed/interrupted）、`cleanup_errors`、每项检查及公开命令证据、image ID、git revision。失败同样进入finally清理，清理错误使总status=failed。只删除本次记录且owner标签一致的容器/网络，绝不prune。成功与失败artifact均保留，镜像可供复测。

## 原型边界

- 未连接正常controller snapshot/report/CLI；P3a/P3c必须接入同revision计划、lease、私有IPC选择、apply失败报告。内部 `select_profile(id)`只表达允许方案ID，不允许设备修改配置；`resolve_automatic`是可信controller结果入口，不开放本地IPC。
- 同revision切换保留Runtime/密码缓存，RAII交还唯一packet receiver，新pump激活前有界丢弃旧delivery。撤回/lease由Runtime所有者关闭整个Runtime并清理缓存；旧receiver不能复活已关闭的Runtime。
- 一个namespace一套独占出口表；不采用已有route/nft资源。故SIGKILL后的资源不会被无条件认领；恢复需可信持久所有权。
- 默认出口的peer/controller/STUN保护先用/32路由pin；SO_MARK socket例外、DNS接管、IPv6数据面、ICMP errors/PMTU属于后续。
- LAN与Internet都做masquerade，但授权分开：公网排除RFC1918/CGNAT/link-local，private目的只有具体网段grant才允许。成功fixture显式授权172.29.233/24；`internet-only`默认方案仍捕获 `/0`，在E拒绝LAN。备用F的路径目的表还排除公共规则指定给E的S.20，已认证客户端也不能借F绕路。
- 双表替换的原子性指packet generation fence与激活，不声称多个Netlink操作构成内核事务；短暂丢包允许，失败不恢复旧I/O。
