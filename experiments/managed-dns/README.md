# Managed DNS：本机隔离验收

这是当前 MagicDNS 的**第一段**：控制面分配的节点名称，在真实 Agent 显式绑定的 `127.0.0.1:5353` 上提供 UDP/TCP nameservice。工具不配置系统 DNS，不修改 `/etc/resolv.conf`，不测试 NAT 打洞或互联网域名策略。远端真实联测尚未授权。

## 主协调者运行

本轮交付仅完成 host 检查；**主协调者负责镜像构建、长 tmux 运行和完整 Docker 验收**。使用当前主线二进制（含 DNS generation fence / disabled cache / self-IP pin 修复），Agent 必须编译启用 `meshway-agent/managed-connections`。镜像必须与 host 和 Docker engine 三者同架构、Linux；工具使用完整不可变 image ID 启动，不允许 emulation。

已有 Ubuntu/Debian 镜像需包含 `meshway-controller`、`meshway-agent`、Python 3、`ip`、`ss`、`dig`，并支持 root 私有 tmpfs。若缺 dnsutils，主协调者可基于自己的实验镜像派生独立专用 tag：

```sh
docker build -f experiments/managed-dns/Dockerfile.tools \
  --build-arg BASE_IMAGE=meshway-managedtest:YOUR_NATIVE_TAG \
  -t meshway-manageddns:YOUR_NATIVE_TAG .

python3 experiments/managed-dns/run.py --image meshway-manageddns:YOUR_NATIVE_TAG
```

不要对 live 容器安装包。工具不构建/删除镜像，不操作 Docker daemon、宿主机路由/DNS/防火墙，不使用 host network 或 privileged。唯一发布端口是 `127.0.0.1` 的随机 controller host port。可用 `--artifacts-dir EXISTING_PRIVATE_PARENT` 指定工作区之外的现有产物父目录。

入口在整个运行与 finally 清理期间持有共享锁：

```
/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/meshway-performance.lock
```

每次使用新 run ID、唯一项目标签 `io.meshway.managed-dns.run`、一条新 bridge、一个新 controller 与三个登记节点。controller DB 和每个 Agent 身份/cache/log 均在其自己的 `/private` mode-0700 tmpfs；不挂载任何已有 `.meshway`、demo/root 数据目录。两个 DNS-only 节点 `Twin` / `tWIN` cap-drop ALL 且不发布连接 key；一台 `Solo` 同进程 data+DNS，仅容器 namespace 内加 NET_ADMIN 与 `/dev/net/tun`。三个 Agent 均 lease=10s、poll=1s。

## 登记与证据边界

所有 HTTP 调用均通过现有生成 SDK 和运行时 schema：`getNetwork`、`saveNetwork`（ADMIN CAS）、`createEnrollmentToken`、`enrollAgentOperation`、`listNodes`、`getAgentDnsOperation`、连接/数据面查询与撤销。没有手写 endpoint 或 HTTP DTO。

当前 CLI enrollment 只有 `--token`，所以 fixture 不调用该命令：helper 接收 stdin grant，在私有 tmpfs **先原子持久保存** Agent 的 `Identity` 本地格式（`apps/agent/src/state.rs`），host SDK 用同一 credential 登记，再原子回填服务端 ID，并通过真实 `meshway-agent status` 验证。该本地磁盘格式是 fixture 唯一耦合点；协议响应始终采用生成类型。grant/admin/node credential 不进入任何子进程 argv 或日志，admin 从不进入 Agent 容器。真实服务是 `meshway-agent run --state /private/agent.json --dns-listen 127.0.0.1:5353`；data 节点再加 `--apply-dataplane`。

`wire.py` 仅是查询客户端，不是 DNS server：随机事务 ID、请求/响应 question 和类型核对、TCP framing 与精确读、压缩 name 边界/循环、rcode/RA/AA/TC、RR owner/class/type/TTL 全部解析和校验；不缓存结果。另以 dnsutils 的 `dig` UDP/TCP header 作独立 smoke check。没有以空 stdout、dig exit=0 或 controller 配置 ACK 作为查询成功证据。

## 必须通过的阶段

1. fresh controller 默认 DNS disabled：生成 projection records 空，三台真实 Agent 显式双协议 bind，但回复 SERVFAIL/REFUSED，零 answer。
2. ADMIN CAS 启用 `fixture.meshway.test`、TTL10，无 Agent 重启：controller 实际分配 IPv4/IPv6 的 A UDP、AAAA TCP、v4/v6 PTR；完整 64 字符 node ID 两个 32 字符标签 canonical；casefold 与 terminal dot。
3. `Twin` / `tWIN` readable alias NXDOMAIN，但 canonical 各自正确；keyless 节点仍有 DNS，且未发布 data identity、grants 空。unknown in-zone NXDOMAIN，out-zone REFUSED，RA=0，ANY/AXFR 拒绝，真实 dig 双协议 header。
4. 单个 `Solo` 进程的实际 TUN UP、分配 IPv4 与 DNS coexist；第二实例使用另一个 DNS port 仍被同一 state lock 拒绝；两个 DNS-only 节点没有 TUN。所有节点 resolv.conf digest 全程不变。
5. 改 zone：旧 zone 不再 positive，新 zone 正确；disabled 再次零记录/零 positive；恢复新 revision，controller TTL300 但回包 TTL <=10s（Agent lease），且 Agent PID 不变。
6. 停 controller 超过 lease：Agent PID 仍活，重复真实 UDP/TCP SERVFAIL 且零 records。此时停并离线重启 DNS-only Agent，保留同一真实 state/cache 文件；必须能 bind，但仍只能 SERVFAIL。
7. 用同 DB 重启 controller：network ID、revision、三个 node ID 不变，新的实际 fresh UDP/TCP positive 后才通过恢复。
8. 撤销 `tWIN`：自身生成 DNS API 403，进程退出，UDP/TCP listeners 全关闭；survivor canonical/reverse 不再包含它，同时其他 canonical 仍 positive。移除重名后 `Twin` readable alias恢复。未知 reverse name 可以 REFUSED（已不属于本机发布的 reverse owner）或 NXDOMAIN。

本工具验证 nameservice 与最小 data+DNS 共存，不声称建立跨节点数据通路。pending-send generation race 的严格 packet unit tests 由 Agent 主线负责。

## 产物与清理

私有外部目录的 `result.json` 仅包含公共 evidence：image ID、network ID、节点 ID/IP/name、revision、查询事务 header/TTL/记录、PID/listener/TUN 等 allowlisted state、阶段/检查名与 cleanup 结果。没有原始日志、admin/grant/node credential、私钥或 profile UUID。失败输出只显示明确阶段/检查与错误类别，不回显 HTTP body、helper stdin、argv 或异常细节。

SIGINT/SIGTERM 先交给 Node finally。清理仅移除本次成功创建并再次核对 ownership 标签的容器/网络；不 prune，不删 image。`cleanup.status=FAIL` 会让总体失败并列出需主协调者检查的本次资源名。SIGKILL/主机宕机无法执行 finally，主协调者按 result run ID 核对标签后手动恢复，勿使用全局 prune。

## Host-only 验证

```sh
node --test experiments/managed-dns/checks.test.ts
PYTHONDONTWRITEBYTECODE=1 python3 experiments/managed-dns/wire.test.py
pnpm exec tsc --noEmit -p experiments/managed-dns/tsconfig.json
```

依赖沿用根 frozen pnpm（Node 24 / pnpm 11），无需改 manifest/lock。测试验证 parser 的错事务/错 question/截断/压缩循环、TTL 与 false-positive 排除、明确失败 header、私有身份原子持久化和 CLI 参数隔离，不启动任何 Docker 或模拟 DNS listener。
