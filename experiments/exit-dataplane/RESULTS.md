# P1c · 已验证结果（2026-10-01）

首轮完整源：`d1be62c3a986158f716a2351ce4251c4ca17cec5`，基于main `47a1ba0`。随后已按要求rebase `de6f531`，采用统一launcher并再次完整通过；最新证据见下节。两个被测工作树均干净（`source_dirty=false`），本页仅增加说明，不改变数据面。

- artifact根：`/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/meshway-exit-20261001-p1c/`
- 最终完整JSON：`final5/result.json`；Linux构建/Clippy/行为测试/验收日志：`final5.log`；wrapper退出码：`final5.exit` = 0。
- 首轮image：`sha256:fd79758009753cd1e8233278f7da33422d16323041f14c6029fc9568624c792a`。`meshway-exit:p1c`与 `meshway-exit:verified` tag已更新到下节的统一launcher复测。
- `status=ok`、`cleanup_errors=[]`，12个场景、105个检查全部成功。容器/网络均按精确记录与owner标签清理，无prune、host network或privileged。
- Docker 29.4.0、ARM64、OrbStack Linux `7.0.14-orbstack-00380-ga7e0a2dc9535`，VM 12 logical CPU；每节点1 CPU/256 MiB、2 Tokio workers、TUN MTU1000、NativeUDP、hop Noise + endpoint Noise E2E。

## 验收

| 场景 | 检查数 | 结果 |
| --- | ---: | --- |
| direct | 10 | C→E→S成功，S看到E地址；未匹配走C本地；伪造inner source、出口主动UDP、非Mesh物理绕路均拒绝 |
| relay | 7 | C→R→E→S成功；无C→E物理路由；SIGKILL R后不可绕过 |
| unauthorized | 3 | 无出口grant时拒绝 |
| internet-only | 5 | `/0`仍捕获private目的，但公网授权不能经E NAT进入172.29.233/24；E自身可达S；正向用例显式授权此LAN后成功 |
| default | 6 | 非具体S.21走默认；承载pin在物理网卡；隐式LAN拒绝；IPv6有实际unreachable兜底及rule |
| withdraw | 7 | 已建立TCP echo截断、新连接失败、owned mark conntrack清除 |
| lease | 7 | 无HTTP刷新，独立到期终止既有及新流，conntrack清除 |
| kill-client | 7 | SIGKILL Agent；TUN退出后client默认sink保留，不回落本地 |
| kill-exit | 6 | SIGKILL E，既有及新流失败，残留拒绝边界阻止回程/开放路由 |
| dual | 26 | 整表方案切换、revision不变、旧TCP不中途迁移；公共规则不变、方案direct完整替换；未授权ID拒绝；认证恶意客户端不能借F绕公共E规则；auto JP→F、未知→仅Mesh |
| locked | 10 | 其他方案与仅Mesh选择均拒绝，原方案正常 |
| direct-exclude | 11 | 1000个direct前缀完整正确；全部lookup不入TUN；双表fence-backed替换、旧排除删除、新排除生效及明确撤回 |

成功用例的private S网段由 `ExitAuthorization.networks` 显式授权，不把隔离LAN伪装为autogroup:internet。公网、LAN授权与路径目的投影分别校验，并共同进入nft interval set过滤。

## 前缀表实测

| 1000个direct /32 | 耗时 |
| --- | ---: |
| 安装 | 8.959108 ms |
| 整表代际替换 | 7.573001 ms |
| 明确取消捕获/批量撤回 | 3.553976 ms |

时间由namespace内Rust安装器单调时钟记录，包含route/rule子进程与内核操作，不含Docker exec传输。替换的原子性是packet generation fence：先停旧pump、准备完整备用表、发布/退休lookup、drain后激活新pump；不是多个Netlink syscall的内核事务。允许短暂丢包，不允许新旧准入同时运行。1000条实测不是8k吞吐/延迟承诺；预算与大表后续要求见RFC。

## 可复现短基线

iperf3单TCP流3秒，ICMP 5次；只作为环境与完整隧道成本参考，不是包装增量损失或优化结论。

| 路径 | 接收吞吐 Mbit/s | 平均RTT ms |
| --- | ---: | ---: |
| E→S，direct用例的物理参考 | 70902.09 | 0.120 |
| C→E→S | 418.77 | 0.519 |
| E→S，relay用例的物理参考 | 102734.80 | 0.078 |
| C→R→E→S | 361.63 | 0.558 |

物理参考保留内核offload，TUN关闭offload，资源与负载参数保留在JSON；两者不是相同协议/加密成本的matched baseline。未做真实WAN、长负载、性能优化或DNS无泄漏声明。

## 检查与实施边界

- `cargo fmt --all --check`通过。
- Linux `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`通过（覆盖要求的workspace/all-targets并检查功能路径）。
- dataplane：63个行为测试通过；Agent managed-connections：116个通过、4个ignored fixture；原有packet/withdrawal/lease/HTTP stalled/crypto rollover测试保留。
- 本机严格Clippy与出口准入/路径回程/公网分类行为测试通过；`pnpm --filter @meshway/docs build`通过。
- `crates/dataplane`最小改动：泛化packet admission以复用泵；公开严格IPv4解析；RAII交还唯一receiver并在再激活前有界drain；shutdown清空待交还receiver。真实多跳测试证明缓存保持、旧delivery丢弃、唯一消费与shutdown不复活。
- 未改protocol/controller/web/apple或根manifest/lockfile。正常controller下发、失败报告RPC及公开IPC/CLI尚待P3a/P3c接入；内部选择只接受允许方案ID，地区解析只接受可信controller结果。入口与失败/恢复边界见 `docs/exit-dataplane.md`。

## 历史产物

`smoke*`、`matrix*`、`final*`前序产物均保留，不覆盖失败证据。早期修复包括OrbStack tmpfs归档copy改为namespace字节传输、nft保留字chain命名、PID身份校验、同revision选择保留Runtime反向密码缓存；rebase后verification补齐只读OpenAPI与policy文档依赖。`final3`为补充公网分类前的完整成功结果；`final4`因policy文档未复制在验证阶段失败，未启动验收；`final5`及以下 `launcher-final`是完整有效证据。

## 统一launcher最终复测

- source：`1ce4b15261e9f0c2cefb5654c2adc850bab5f8ee`，main baseline `de6f531`；`source_dirty=false`。
- 最新JSON：同artifact根下 `launcher-final/result.json`；日志 `launcher-final.log`；`launcher-final.exit` = 0。
- 最新image：`sha256:554f770309495664107090a0f2e0ed15eea2b81072e3820f519081df60ab2b34`。
- 共享wrapper→launcher→runner复用同一FD：无嵌套flock；12场景/105检查、`status=ok`、`cleanup_errors=[]`。所有Docker子命令和私有文件字节传输继承锁FD。
- Linuxworkspace严格Clippy、63个dataplane测试、116个Agent测试再次通过；3个无Docker fixture单测通过并加入 `.github/workflows/check.yml` 的Python矩阵。
- 1000个前缀安装9.073860ms、整表替换8.210804ms、撤回4.720704ms。
- C→E→S单次基线466.55Mbit/s、平均RTT0.906ms；C→R→E→S为278.56Mbit/s、0.938ms。两轮单样本波动保留，不选择最佳值作为长期性能声明。
