# 手动验证连接

当前 Docker demo 建立的是 **认证邻接**。三个容器位于本机，城市名只是标签：

```text
demo-nyc ←── Native UDP ──→ demo-lax ←── VLESS/TLS ──→ demo-sha
```

SHA 主动连接 LAX，但 LAX 可以复用该双向通道发起邻接探测。NYC 与 SHA 没有直接邻接；托管模式尚未安装 TUN 或授予经 LAX 转发的业务路径。

## 进入容器

先在主机找到本项目 demo 的容器名称：

```sh
docker ps --filter label=io.meshway.demo.project=managed-neighbors \
  --format '{{.Names}}\t{{.Status}}'
docker exec -it <容器名称> sh
```

诊断镜像提供 `meshway`（`meshway-agent` 的简写）和标准 `ping`。容器内 `MESHWAY_STATE=/state/agent.json`，需要时仍可显式使用 `--state`。

## 测试正在运行的认证通道

在 NYC 容器中：

```sh
meshway ping demo-lax --count 3 --timeout-seconds 3
```

在 LAX 容器中分别执行：

```sh
meshway ping demo-nyc --count 3 --timeout-seconds 3
meshway ping demo-sha --count 3 --timeout-seconds 3
```

在 SHA 容器中：

```sh
meshway ping demo-lax --count 3 --timeout-seconds 3
```

这个命令通过私有本地控制 socket 请求**已经运行的 Agent**发送一次新的加密邻接 nonce，不新开 Agent、不重建身份，也不使用历史成功状态冒充本次结果。输出包含实际 peer、承载、建连发起方向和 RTT；成功才返回零退出码。也可用完整节点 ID 代替名称，重名时必须使用 ID。

`meshway ping` 是邻接控制探测，**不是 ICMP，也不是多跳 mesh trace**。从 NYC 执行 `meshway ping demo-sha` 应明确失败，因为 SHA 不是它获准使用的直接邻居；命令不会自动借 LAX 转发。

## 测试 Docker 底层网络

```sh
ping -c 3 lax.mesh.test
```

普通 `ping` 测试的是 Docker DNS 与底层 IP/ICMP。即使 LAX Agent 停止，容器网络仍可能回应，因此它不能证明 Meshway 认证或承载协议成功。

控制台中分配的 `100.96.x.x` 是 Mesh 地址；当前托管邻接 demo 尚未把这些地址安装到 TUN，不能据此用普通 ICMP、SSH 或 iperf 验证完整组网。

## 页面和回执怎么看

- 节点页的“已保存”只表示配置缓存与回执。
- “连接配置已应用”表示连接管理资源已经安装，不代表所有 peer 都已连接。
- “已认证”配合新鲜探测结果，表示 Agent 最近完成了真实邻接往返。
- 拓扑的认证邻接、配置意图、规划路径各自表示不同证据；它们都不自动授予业务 IP 流量。

`/topology` 默认展示近期认证邻接：双侧报告一致时画实线，单侧或不完整报告单独标记。配置意图可按需开启；规划路径使用虚线。过期报告、旧配置版本、已撤销节点或不匹配的候选不能提供当前连接证据。缺少运行数据时显示未知，而不是把读取失败画成节点隔离。

完整的 full-mesh IP 体验需要继续接通业务授权、路由执行与 TUN。它不要求永远保有每对节点之间的物理直连。

## 本机 demo 验证记录

诊断镜像 `sha256:97d83046a3b73c0a2abddd49c6307d1d96efdc04316ee422eb5f7bc9bb9194f9` 已用于持久 demo。四个方向各三次主动探测均成功：NYC→LAX、SHA→LAX、LAX→NYC、LAX→SHA；本机实测 RTT 约 175–622 微秒。LAX→SHA 的输出为 `accepted (peer initiated)`、`vless (TLS)`，说明复用了 SHA 发起的连接。NYC→SHA 返回非零退出码并说明不是已配置直接邻居。

普通 ICMP 到 `lax.mesh.test` 的三次探测无丢包。这只证明 Docker underlay；认证探测使用的是另一套实际加密控制往返。

镜像升级核对了原节点 ID、公钥、身份 / 私钥 / profile 文件字节以及当前用户配置。升级完成前要求三个新进程的 epoch 都高于停止旧进程后的基线，并重新通过双侧认证探测；不能用重启前仍新鲜的旧报告冒充成功。详细生命周期和恢复命令见仓库 `experiments/managed-connections/DEMO.md`。
