# Linux socket-level underlay routing fixture

独立机制实验；生产 Agent、P1c `/32` bypass、DTO 与运行时未修改。推荐机制与实际集成缺口见 [`docs/underlay-routing.md`](../../docs/underlay-routing.md)。

## 可重复命令

在仓库根执行（macOS host 使用 Python 3.10+、Docker Linux VM）：

```sh
python3 experiments/transport/performance_lock.py python3 -m unittest discover -s experiments/underlay-routing -p 'test_*.py' -v
python3 experiments/underlay-routing/run.py
```

runner 自己调用已有 `performance_lock.locked_launch`，通过继承 FD 保持**同一把共享锁**直到精确清理完成，包括镜像构建。不要再在外层手动打开另一把锁；继承相同 FD 的现有 launcher 可复用。锁默认路径：

```text
/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/meshway-performance.lock
```

可以用 `--skip-build` 复用已构建镜像。修改 helper/fixture 后必须重新构建：

```sh
python3 experiments/underlay-routing/run.py --image meshway-underlay-fixture:local
```

默认产物为批准 TMP 的 `meshway-underlay-artifacts/mw-underlay-<id>/result.json`，另含 client stdout/stderr 与 observer 的真实 `recvfrom`/accept JSONL。`status`、每 check 的证据、失败原因及 `cleanup_errors` 均记录；还记录 kernel、镜像 ID 与 fixture 源码 SHA256。`--skip-build` 会核对镜像与当前源码，拒绝陈旧镜像。网络/权限/IPv6 不支持导致失败，不伪造 pass。输出绝对路径由 runner 打印。

## 隔离与证据

- 唯一 owner label + 随机资源名；内部 Docker bridge `172.30.247.0/24`、`fd74:6d77:247::/64`，fixture 端口 `41861–41865`，不向 host 发布端口。
- 两个独立容器：服务端 `.20/::20` 与客户端 `.10/::10`；客户端另设 `.11/::11` 作为新 underlay 源、`.13/::13` 与 `.14/::14` 作为模拟 E/F 源。IPv6 secondary 地址使用 `nodad`，并等待主地址/链路本地 DAD 完成后保存路由快照，只用于受控 fixture。
- `--cap-drop ALL --cap-add NET_ADMIN` 与 `/dev/net/tun` 仅在容器；SETUID/SETGID 只用于测试子进程降权后 SO_MARK 真正拒绝。无 NET_RAW、host network、privileged、宿主路由/DNS/防火墙修改。
- 服务端真实标准 STUN binding/XOR-MAPPED-ADDRESS 从 `recvfrom` 得到映射；E/F 是同 namespace 中不同地址的**模拟出口源对照**，不是真实 E2E/出口 NAT 或公网地址。普通 STUN 故意被 TUN sink 丢弃，因此没有返回可供伪报 region 的出口映射。
- 同一个客户端 Python 进程同时创建 marked 内部用途 socket 与 unmarked 普通业务 socket，对同一 host、同一服务端口发包。每个普通请求须 timeout/拒绝且由真实 TUN 读取证实被捕获；protected 请求须成功且其源端口不出现在 TUN。
- `SO_MARK` + 正向 `fwmark/iif lo goto` 只跳过连续的自有 capture block，保留原 RPDB；exact-priority 的 NOP 目标先安装，并验证 goto 没有 unresolved 标记。实测 `not fwmark … iif lo` 会反转整个 selector 并误捕获 forwarding，因此记录该反例与修正后的 forwarding lookup，不将否定组合当推荐机制。
- 具体 prefix → `/0` → 第二 profile 捕获表（模拟选择 E→F，业务均进入 sink，不实际转发到 E/F）；STUN 仍观测 underlay，保护不依赖目标 IP bypass。HTTP、STUN UDP、plain TCP/TLS、bootstrap DNS UDP/TCP、nonce echo 探测分别验收。TLS 使用镜像内临时自签根，仅在 fixture client trust 中安装并校验 hostname。
- 额外系统 rule/table 与低位 mark 在 capture 之后仍生效，STUN 确認不同源 IP；live priority/table 冲突拒绝且资源不变。host-only tests 覆盖 mask 冲突、未知 selector、socket 用途、权限错误无 fallback、代际、IPv4/IPv6 wire parsing 与精确 owner 清理。
- 换网模拟 prepare 新 guard → invalidate/close 旧 probe/socket → activate 新 socket；旧 socket send 实际报错，旧观测不当 fresh。observer 仍实际收到退休前已发送的 STUN，证明本地 close 不召回已发送字节。模拟是源地址/代际切换，**不是真实网卡 roam 或 NAT 行为**。
- 关闭两个 TUN 后 unreachable routes 留存，普通业务失败而 protected controller 连通。最后只逐条删除自建规则/routes，RPDB 必须恢复快照；容器/网络按 owner label 精确销毁，不 prune。镜像保留以便复现。

## 限制与接入

helper 是实验模型，无产品 node identity。nonce echo 不证明认证；TLS 成功不证明 VLESS/WS；双栈 underlay 不证明 IPv6 overlay；两份同构表不证明真实 policy 编译或业务授权。所有 grants/via、packet-generation fence、lease/持久恢复、DNS policy、真实 controller reqwest 与 carrier socket 仍需生产接入与行为测试。尤其 reqwest 0.13.5 没有直接 pre-connect mark 配置，不能用 `interface` 或 connector middleware 假装解决。

需要的 CI 条目交给协调者：上述 host-only unittest；独立具备 Docker/TUN 的 Linux job 运行 `run.py`，沿用同共享锁/FD 契约与结果上传。此目录没有修改根 manifest/lockfile/CI。
