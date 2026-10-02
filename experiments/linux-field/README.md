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

## 已清理第一轮的同身份复验（准备完成，未执行）

`retest.ts` 是独立入口。原 `run.ts` / `--resume` 及第一轮 FAIL 证据保留原用途。
复验不重新注册、不生成 token、不 claim owner、不保存策略；Controller 使用原 DB 正常启动迁移，
Agent 直接复用原 `run.enabled` 和 `agent.json` 执行原 helper 的 `boot`。
当前没有本脚本可使用的已审核新镜像；**协调者先完成源码审核、构建、版本来源库存与远端镜像交付，再单独执行**。
本次开发未执行 Docker、SSH 或任何部署，也未读取现场秘密。

### 私有输入由协调者创建

配置文件须 mode 0600，放在新的私有目录；不要提交。CLI 显式提供新 artifact 目录，该目录必须不存在，
且不得位于原 baseline 目录内。每次执行生成 `retest-result.json`，不修改旧 reports、台账、source 或 CA。
`--mode validate` 只读本地输入并校验，不创建 artifact、不调用 SSH/Docker/HTTP。

```sh
node experiments/linux-field/retest.ts \
  --config "$PRIVATE/retest-config.json" --artifacts "$PRIVATE/attempt-direct" \
  --case direct --cleanup retain --mode validate

# 仅由协调者在审核、构建和库存核对后执行：
NODE_EXTRA_CA_CERTS="$ORIGINAL/ca.pem" node experiments/linux-field/retest.ts \
  --config "$PRIVATE/retest-config.json" --artifacts "$PRIVATE/attempt-direct" \
  --case direct --cleanup retain --mode execute
```

输入字段：

- `run_id`：原 `r1-5038bd8a8e3f`；`attempt`：不同于 run ID 的新唯一字符串（10–48 位小写字母、数字、`-`）。
- `controller_url`、`network_id`、`admin_token`：原 Controller 的 HTTPS origin、原 network ID 和既有管理员凭据。
  不通过 helper 提取管理员凭据；token 只进入进程内存和正常生成 HTTP operation 的 Authorization。
- `ca_file`：原 `ca.pem` 的本地绝对路径；Node 启动时必须用同一路径设置 `NODE_EXTRA_CA_CERTS`。
- `baseline_directory`、`baseline_sha256`：固定旧证据目录及下列五个文件各自的 SHA256：
  `round-summary.json`、`runtime-cleanup.json`、`final-public-control.json`、`underlay-fib-lookups.json`、`result.json`。
  原 `result.json` 必须仍为 FAIL。库存只列这五项。
- `control_inventory`：`network` / `policy` / `access` 三份审核过的原 source 快照，分别通过现有生成的
  `NetworkSnapshot` / `PolicySnapshot` / `MeshAccessSnapshot` schema。
  原 final-public-control 只有 nodes / connections / runtime，因此这些缺项需由协调者根据已保存来源提供并核对，
  不能拿候选启动后的响应充当旧基线。任何无法证明的旧源都应阻塞执行，不临时保存“等价新配置”。
  协调者现已保存公开的 `TMP/meshway-field-baseline-inventory-r1-5038bd8a8e3f/control-source-inventory.json`
  和 `source-provenance.json`：前者可直接作为本字段，后者记录原 v11 只读副本、生成 GET、原 OpenAPI 和来源证明。
  对应 `public-inventory.json` 的 `hosts[host]` 可作为下述 `public` 字段；无需重建基线或访问私有 copy。
- `hosts`：固定四项 `jp-tyo`、`us-lax`、`us-nyc`、`kr-icn`；每项包含原 `ssh` / `sudo` / `listener`，
  以及 `image: { id: "sha256:<64hex>", source: "<40hex verified commit>", verified: true, version_evidence: "<review/build inventory reference>" }`。
  不内置候选提交、镜像 tag 或自动 pull。两种架构的库存分别明确来源。
  Docker image inspect 的 `Config.Labels["org.opencontainers.image.revision"]` 必须等于显式完整 `source`；
  不能只依赖新容器自填的 source label。离线校验 `run` 和恢复 `create` 都使用 `--pull never`。
- 每个 host 的 `public.agent` 是下述离线白名单输出的**先前核对库存**；LAX 还需 `public.controller`。
  不能跳过或把首次复验读取结果自动接受为库存。

`public_state.py` 的固定输出 shape：

```text
agent = {
  seed: { role, name, controller, mesh_prefix, mesh_ipv6_prefix },
  hashes: { "field.json": SHA, "container.py": OLD_HELPER_SHA, "ca.pem": SHA,
            "agent.json.connection-key.json": SHA },
  node_id, public_key, state_sha256,
  http: { address: ORIGINAL_MESH_IP, body: ORIGINAL_CANARY }
}
controller = {
  seed: { role, name, controller, mesh_prefix, mesh_ipv6_prefix },
  hashes: { "field.json": SHA, "container.py": OLD_HELPER_SHA, "ca.pem": SHA,
            "server.pem": SHA, "server.key": SHA }
}
```

协调者可显式调用 `remote.py` 的 `restore-public` 取得该白名单供人工核对：stdin 使用上述 host 库存，
附 `action`、`run_id`、`attempt`、`role`、`image`、`source`、`verified`、`sudo`、`public_source`、`restore_source`、
`expected_public`；最后三项分别为 `public_state.py` / `restore.py` 源码字符串和已审核白名单。
所有 restore 角色动作都会先要求观测与 `expected_public` 完全一致。
尚未建立公共库存时，由协调者在官方镜像中仅运行 `public_state.py` 的固定 `public_state(role)`，
使用与 restore 相同的 `--network none --read-only --cap-drop ALL` 和单角色只读 bind；该动作不由复验 runner 自动执行或信任。
无需 SSH 用户读取 uid-0 0700 角色内容，不加 DAC_OVERRIDE / CHOWN / privileged，不宽泛复制 state。
校验器仅打开固定文件、输出 seed/public ID/hash/canary，拒绝 symlink，不提供任意路径或日志读取入口。
Controller 检查原 `controller.sqlite3` 和 `admin.token` 存在且非空，但不返回其内容。
hash 校验保留原 CA/HTTPS 绑定和 old helper SHA；此版本**不替换 helper**，CLI 不兼容应失败并保留证据。

### 恢复边界与冲突

`remote.py` 新增的 `restore-*` 分支载入独立 `restore.py`，不走原 bootstrap owner 转交。
每台 host 的 parent `owner` 和角色 `*.removed-id` 必须存在；原容器/bridge 未清理或角色 `.id` 仍存在则拒绝。
原角色目录必须保持 uid 0 / 私有权限；直接绑定原路径，不迁移身份、DB、配置或材料。
新资源名为 `meshway-field-<run_id>-<attempt>[-<role>]`。
label 包含原 owner、attempt、角色与 source。重试仅接纳名称、label、immutable image、bind 路径、
capability、入口和端口均一致的容器，不删除/接管未知资源；stopped owned 容器才 start，缺少资源则 create。
能力只兼容 Docker 的 `NET_ADMIN` / `CAP_NET_ADMIN` 两种已知写法，精确比较并拒绝多余或重复能力；
Controller CapAdd 仍为空、CapDrop 必须恰为 `ALL`。
resume 同时核对声明的两个 SSL_CERT 环境路径、sysctls、128 pids、384 MiB、1 CPU 和 init；
不对镜像未由 fixture 设置的其他环境变量或外部字段作一致性保证。
bridge 核对 label/name/IPAM/非默认宿主路由冲突。豁免仅限完整 own bridge ID 对应的设备、kernel protocol、
原 gateway prefsrc，以及精确 subnet unicast（main/link）、gateway local /32（local/host）、
IPAM broadcast /32（local/link）；foreign alias、不同 dev/属性、overlay route 均拒绝。
Agent 仍为 cap-drop ALL + NET_ADMIN + TUN；Controller 无 capability。宿主改动只来自 Docker 本轮 bridge/端口，
不修改 route/DNS/firewall/daemon，不使用 host network。

### 测量与结果

`--case direct` 测 TYO↔LAX；`--case ordered` 测 TYO→LAX→NYC→KR 及反向。
**两种 case 均恢复原四节点配置**，不把 full ordered 策略改成 two-node 配置。
节点 ID/IP、公钥来自旧 generated-schema 校验后的 public control；目的 IP/canary 来自原 public seed 库存。
每个方向先预热存活 source→将重启 target 的真实 ICMP/HTTP，再只重启 target，恢复前不发反向业务。
逐次保留失败/成功 sample、耗时、last failure 和 first success；实际失败窗口只按离散采样界定，不承诺连续上界。
验证 source epoch 未变化；多跳时还验证 source 下一跳 epoch 未变化。
direct 的 target 同时是下一跳，所以其 restart 是唯一预期变化，不额外重启 source。
target 前后 public ID、Mesh IP/canary、pinned key 和 state hash 必须不变。

业务恢复和报告新鲜度分开：原 Controller stop 后等待原 30 秒 lease 的 42 秒预算，采集真实流量失败与 kernel no-fallback；
正常 Controller start 后验证业务，再对四节点进行至少 65 秒的单次在途异步查询。
保存每次服务端 `reported_at` / epoch / sequence，要求 current、非 stale、多个新 sequence/report，
且每个节点服务端时间跨度至少 60 秒。轮询不写 latency history，不把 heartbeat/config ACK 或一次 fresh 判作修复。
读生成的 listNetworks/listNodes/getNetwork/getConnections/getPolicy/getMeshAccess，对比源和公钥；仅允许 revision/receipt/观测更新。
整个测试没有管理 mutation API。

结果含旧 immutable evidence refs、candidate image/source、samples、strong/gaps、cleanup 状态。
每个 case 未选的另一条矩阵明确为 gap；KR→NYC 原 underlay FIB 经 WARP 的范围继续保留。
`--cleanup retain` 无论成功/失败均保留运行资源；`--cleanup owned` 显式精确删除当前 attempt 已核对容器/bridge，
保留所有原私有 DB/身份/keys/CA/config/materials/old reports。冲突或清理失败会保留资源并标为 FAIL。
中途 Docker create 成功但连接中断等不确定结果须由协调者用相同库存调用 `restore-cleanup`（逐角色）和
`restore-cleanup-network`，它们仍核对 label/路径/image；网络还有 attached 容器时拒绝删除。
新 artifact 目录用于安全重试，不覆盖上次证据。

### 本地检查

```sh
python3 -B -m unittest discover -s experiments/linux-field -p 'test_restore.py'
python3 -c 'import ast,pathlib; [ast.parse(p.read_text(), filename=str(p)) for p in pathlib.Path("experiments/linux-field").glob("*.py")]'
node node_modules/typescript/bin/tsc --noEmit -p experiments/linux-field/tsconfig.json
```

worktree 缺少 SDK `zod` 解析链接时，协调者可用 TMP tsconfig `extends` 本配置，
`compilerOptions.paths.zod` 指向根仓库已有 `packages/control-client/node_modules/zod`，
`typeRoots` 指向根 `node_modules/@types`，再由根现有 TypeScript 执行检查；不安装 console 依赖、不改 root manifest/lockfile。
本地拒绝/重试测试使用 fake subprocess 边界，不是镜像、TUN 或远端复验通过的证据。
