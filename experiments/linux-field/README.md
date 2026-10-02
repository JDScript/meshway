# 远端固定链路验收

第一轮以固定、已验证后端源码构建的 Linux 镜像，在用户登记的 Docker 主机运行真实
Controller → 生成 SDK / Agent → TUN。`run.ts` 先验证东京 / 洛杉矶，再按私有库存可选扩展纽约 / 首尔。
脚本只创建带本轮 label 的容器 / bridge；停止、重启和清理逐项核对 owner，不恢复宿主配置快照。

## 本轮边界

- 镜像固定为已验证 `4d106e0`，与当时主线后端逐文件一致；未审定的 v3 / underlay 候选不参与。
- 测试控制面使用独立数据目录和临时 CA 验证的 HTTPS。CA 仅装入测试进程的信任路径，
  不修改宿主 trust store。`container.py tls` 只是固定 loopback upstream 的测试 TLS 终结器。
- Node、listener、VLESS WebSocket/TLS 材料、连接候选、路由和 Access 通过正常接口管理。
  不安装设备端 profiles。当前实现仍需 `--apply-dataplane` / TUN / `--dns-listen` opt-in，
  在 fixture 内固定；这不代表这些本地参数已收敛成平台托管功能。
- 每容器最多 1 CPU / 384 MiB；节点仅 `NET_ADMIN` 和 TUN，rootfs 只读，非 privileged / host network。
  非 root SSH 用户新建的私有 bind 目录先由一次性、无网络、仅 `CHOWN` 的准备容器转交 uid 0；
  只接受尚无 Agent state 的固定 bootstrap 文件，运行中的 Agent 不增加 `DAC_OVERRIDE` 或 `CHOWN`。
- 网络使用 `198.18.42.0/24` overlay 与 `172.31.249.0/28` Docker bridge；初始化时检查与现有非默认宿主路由冲突。
  Docker 正常创建自有 bridge / 端口规则，不手工改宿主 RPDB / DNS / 防火墙 / daemon。
- 公网端口：控制 HTTPS 28420，VLESS 28443；预留 UDP 28444 不计为已验收的打洞。
- HTTP canary 仅绑定控制面分配的 Mesh IP，ICMP / HTTP 使用 Agent 安装的实际路由。
  DNS 检查通过 Agent 真实 loopback UDP/TCP listener 完成 PTR / A 查询。
- 第一轮成功不代表自动建连、主备、打洞、GeoIP 或正式全 IP socket 保护完成。

## 私有运行库存

运行配置不提交到仓库，与 `ca.pem`、`server.pem`、`server.key` 放在 mode 0700 的本轮目录。
实例 admin token 仅保存在该目录的私有文件及进程内存，不写公共结果。

```json
{
  "run_id": "r1-unique-random-id",
  "source": "full-verified-commit",
  "controller_url": "https://controller-public-IP:28420",
  "hosts": [
    {"name":"us-lax","ssh":"registered-ssh-target","sudo":false,"image":"sha256:immutable-image-id","endpoint":"public-endpoint","listener":true},
    {"name":"jp-tyo","ssh":"registered-ssh-target","sudo":true,"image":"sha256:immutable-image-id","endpoint":"unused-for-dial-only","listener":false}
  ]
}
```

四地扩展时另提供 `us-nyc`（listener=true）和 `kr-icn`（listener=false）；所用两种架构镜像须先送达。
SSH 用现有 known-hosts 严格校验，所有 helper 输入经 stdin，避免秘密进入 SSH / Docker 命令行。
测试注册令牌只会在隔离容器内的短生命周期 CLI argv 出现，是当前 Agent enroll 接口的已有边界。

```sh
node node_modules/typescript/bin/tsc --noEmit -p experiments/linux-field/tsconfig.json
NODE_EXTRA_CA_CERTS="$RUN/ca.pem" node experiments/linux-field/run.ts "$RUN/run.json"
```

首轮先 native build 两种架构，镜像 digest / binary SHA / 来源与构建日志单独保存。
不因构建前端失败换依赖或放宽 lockfile，不上传当前目录的秘密和其他项目。

## 证据与收尾

`result.json` 每阶段增量保存公开观测，区分 runtime / TUN / DNS 和实际包流。
正向三次 ICMP + HTTP、同容器重启身份不变、可选四地有序路径 / 必经点故障不绕过、控制面失联 / lease / 恢复分别记录。
重启后的 current / authenticated 状态不代表业务已经恢复。恢复阶段以真实双向包流做最多 120 秒的有界检查，
逐次保存失败与成功以及从重启开始的实际耗时，不裁掉旧 E2E cache 等待窗口。
四地存在首尔→洛杉矶的备选边，必经纽约被停止时，东京↔首尔必须失败；不能因备用物理边可达绕过明确路径。

失败保留本轮 namespace 以诊断，不自动重放 enrollment 或覆盖 state。
定位并修复部署问题后，可在同一私有库存后追加 `--resume`：先保留原失败报告，再核对资源 label、
immutable image 和控制面 / 本地节点 ID，复用已有容器与身份。若节点尚未创建，才进行新的注册；
同一 run 不盲目再次 create 或替换已存在 identity。该入口是 fixture 恢复，不是设备端新增配置流程。
成功时删除精确 label 匹配的测试容器和 bridge；镜像及远端私有 state 目录保留供审计 / 同身份再验收，
不会混入用户既有数据。再次运行使用新 run ID；同一 run 的恢复须先核对现存资源，不直接再次执行 create。
原宿主 SSH、RPDB、DNS 和既有容器状态由前后只读清单核对；差异需解释，不能回滚用户并发变化。
