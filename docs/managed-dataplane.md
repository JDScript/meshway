# 托管 Mesh IPv4 数据面

Linux TUN已将注册节点身份、授权路径和Noise E2E接通；当前支持显式激活Mesh v3，旧源模式兼容保留。应用访问Mesh IPv4，中间节点只转发端到端密文。

当前v3执行合同见[IPv4 Mesh执行](/p3a-mesh-execution)，本页保留TUN运行边界与旧模式验收入口。完整互联网出口、Mesh状态型回包、可信域名上下文和IPv6 TUN尚未正式接通。

## 三项条件分别满足

`NYC → LAX → SHA` 需要认证邻接、路由结果与端点访问授权共同成立。SHA 可主动连接 LAX，后者复用通道向 SHA 转发；通道双向不授予双向业务权限。`direct` 不放行 Access，Access allow 不修改必经节点。

控制面联合编译精确路径，沿途身份有不可替换公钥，不重复hop。v3 Access使用叠加grants，有序路由first-match；旧模式仍保留allow/reject作为兼容迁移源，见[Access](/access-policy)。非法明确引用导致完整执行失效，源文档保留。

### 回程也是一个方向

当前不是状态型防火墙，ICMP reply、TCP 回包与反向主动访问均按方向检查。往返需要 `SHA → LAX → NYC` 路由，并以双向规则或独立反向规则授权；目标 grants / 状态回包实现不可由物理双向通道推导。

## 哪些路由规则执行

| 当前条件 | 实现边界 |
| --- | --- |
| 节点 / 管理员国家元数据 / Mesh IPv4 CIDR | 使用绑定身份和清单求值 |
| 旧版 first-match、via、direct/reject | 与 Rust 编译器一致，与独立端点授权取交集 |
| 域名 suffix | IP 包没有可信域名，相关未知条件可能改变路径时失败关闭 |
| 外部目标 / 出口 / 子网 / IPv6 | 未由本切片接通 |

纯预览可接收显式域名，不代表真实 TUN 获得该上下文。域名规则不能简单忽略后落入更宽松的 direct。

## 运行边界

Agent 编入 `managed-connections`，在提供 `/dev/net/tun` 与所需权限的隔离 Linux 环境显式运行：

```sh
meshway-agent run --state /state/agent.json \
  --apply-dataplane --tun-name meshway0 --tun-mtu 1000 \
  --interval-seconds 2 --connection-lease-seconds 15
```

平台托管VLESS/WS不传profiles；旧私有profiles只作兼容/等价迁移。保守MTU1000不是通用PMTU，须检查完整路径封装预算。普通`--apply-connections`只安装邻接。

- `getAgentDataplane` 一次读取同 revision 的配置、连接、授权和公钥，不混合新旧快照。
- 使用原节点凭据和私钥；E2E peer 信任不额外授予直连权限。
- 接收方验证 E2E 来源、完整路径、内层源 / 目标 IP 后才注入 TUN。
- 撤回、权限拒绝、lease 到期和代际更换阻止旧路径继续发送 / 注入，不等待阻塞 HTTP。
- 只接管控制面 Mesh IPv4 网段，controller / transport / DNS 底层引导不递归进入 TUN；停止后网段保留拒绝边界，不落回默认路由。
- 当前不接收 IP 分片；小包成功不证明完整 MTU / PMTU 支持。

ACK 只表示持久保存。新鲜邻接报告、实际 TUN / 路由和绑定 revision / digest 的本地代际证据共同构成运行前提，最终可达性仍须流量实测。

## 验收方法

服务绑定目标 Mesh IPv4，在源节点使用普通工具：

```sh
ping -c 3 <目标 Mesh IPv4>
curl --noproxy '*' --fail --max-time 5 http://<目标 Mesh IPv4>:<测试端口>/
```

覆盖有路由无授权、仅正向无回程、完整双向成功、旧版 reject 阻止包、必经节点故障、控制面失联 / lease、DNS 暂时失效 / 恢复及撤销。Docker underlay ping 与 Agent 邻接 nonce 是不同证据，见[CLI](/cli#手动验证-三种不同的探测)。

### 隔离的三节点验收

`experiments/managed-dataplane/run.py` 使用独立 controller、真实注册身份、两个数据 bridge 和限定目的的控制面代理，NYC / SHA 无底层直达、kernel forwarding 关闭，HTTP 服务只绑定实际 Mesh IPv4。

```sh
python3 experiments/managed-dataplane/run.py --image <已构建的-managed-镜像标签>
```

构建与参数见该目录 README，runner 输出非秘密 `result.json`，共享 performance lock。历史通过运行 `mupb51em-eaecbd9d255f1984`（Access v2，Ubuntu ARM64）有 33 项证据，验证双向规则、tag 变化、旧 first-match、default-allow 受 routing reject / via 限制，以及原有 lease、DNS 恢复和撤销；资源清理无错误。此前运行 `mup6m9qh-539ecd0828df0f80` 验证在暂缺 DNS alias 后不重启恢复真实 ICMP / HTTP。

回归另覆盖已知新 revision、ACK 409 和不完整 403 body，避免旧权限等待请求结束或普通 lease。完整阶段与镜像 digest 由 artifact 记录，历史功能测试不提供性能或跨地域结论。

## 持久 demo

已初始化 demo 的更新 / 启停使用[统一入口](/dev-services)，切换 TUN 模式用 `experiments/managed-connections/demo.py enable-dataplane` / `disable-dataplane`，完整命令与恢复见 `experiments/managed-connections/DEMO.md`。

工具不自动添加授权或改路由，须先配置当前策略；保留身份和私钥，核对新 runtime epoch。Agent 收敛为原 UID / GID 与 `NET_ADMIN`，不需 privileged / `NET_RAW`。可写开发 shell 的安装包仅属于容器可写层，镜像重建会丢失。

查询当前清单中的实际 Mesh IP 和容器名后测试，不依赖文档硬编码本机分配。已完成六方向 ICMP 与经 LAX 的 HTTP 功能验收，但同机城市标签不证明跨地域路径，普通 ping TTL 不显示密文中继数，真实 trace 尚未实现。
