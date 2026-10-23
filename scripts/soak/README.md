# 现场 soak 工具

v0.1.0 [里程碑 C](../../docs/v2/process.md#v0-1-gate) 的 7 天 prod soak：采集、日志、按日程注入故障、日报与最终判定。运行在采集机 **kr-icn-arm64**（`ubuntu@193.123.237.166`；planck 出站 ssh 被封，所以不在 planck）。Python 3.8+ 标准库，压缩用系统 `zstd`（Python 3.14 起用内置 `compression.zstd`）。全部只读，除了 `fault.py` 按日程执行的注入。

| 文件 | 作用 |
| --- | --- |
| `common.py` | 配置、经 gate 的远程调用（ssh／本机／前缀）、时间、JSONL 追加与按日压缩、30 天保留、磁盘下限、`flock`、gate spec 渲染 |
| `gate.py` | 装在每台被采集主机上的 forced command：只执行允许表中的命令形（见下） |
| `sample.py` | 每 5 分钟：各节点 `status`（`meshway --json status`＋daemon PID 与启动时刻）、Server 健康与容器状态 |
| `resources.py` | 每小时：60 秒 `schedstat` 每线程 CPU、进程合计、已回收子进程（`nft`／`sh`）、RSS、fd、socket、netns 包计数 |
| `logs.py` | 每小时：journal（cursor 续读）／`docker logs`（不重叠时间窗）增量，缺行检测 |
| `report.py` | 每天：日报；`--final` 整段判定；`probe` 子命令做每日 ping 与吞吐 |
| `fault.py` | 每分钟 `due`：到点执行故障并写 `faults.jsonl` 标记；`run`／`recover`／`list`／`check` |
| `render.py` | 从 `config.json` 渲染 gate spec、`authorized_keys` 行与 gate 安装命令、crontab 块 |
| `install.sh` | 采集机上的幂等安装／`--uninstall` |
| `config.example.json` | prod 配置模板（`REPLACE` 占位全部填完才能加载）；协调者的私有草稿在 `.meshway/prod/soak/config.json` |

## 目录

```
~/meshway-soak/
  config.json             配置（0600；t0、主机、节点、边、Server、故障日程、门槛）
  bin/                    本目录的脚本
  keys/collector_ed25519  采集专用 key（0600）
  known_hosts             钉住的各主机 host key
  gate/<host>.json        渲染出的 gate spec（kr 本机直接用；其余拷到对应主机）
  faults.jsonl            故障开始／结束／跳过标记（只追加）
  state/                  logs.json（journal cursor、docker 时间窗）、log-gaps.jsonl、*.lock、各任务 *.log、cron.log、disk-stop.jsonl
  samples/<node>/<日>.jsonl            5 分钟 Status 样本（node = it7 jp kr planck hk server）
  samples/<node>/resources/<日>.jsonl  每小时资源样本
  samples/probes/<日>.jsonl            每日 ping／吞吐
  logs/<node>/<日>.jsonl[.zst]         日志（当日未压缩追加，过日压缩；node 含 server）
  reports/<日>.md|.json, final.md|.json
```

日期都是 UTC。`samples/`、`logs/`、`reports/` 中带日期的文件保留 30 天；样本与日志过日后压缩。磁盘剩余 < 10 GiB（`min_free_gib`）时所有写入停止（`faults.jsonl` 标记与 `state/disk-stop.jsonl` 除外，均为几十字节），日报首行标红；空间恢复后自动继续写入。

## 节点形态（prod）

| 节点 | 主机 | 形态 | gate 账户 |
| --- | --- | --- | --- |
| it7 | us-lax-it7（用户的机器） | systemd `meshwayd` | root |
| jp | jp-tyo-arm64 | systemd `meshwayd` | ubuntu（`sudo -n`） |
| kr | kr-icn-arm64（采集机本机） | 容器 `meshway-kr`（`/opt/meshway-field`，`sudo docker`）；Ubuntu 22.04 的 resolved 后端不可用（D4，DNS-2 修复后回物理机，届时只改 `nodes.kr` 为 `unit`） | 本机，无 key |
| planck | ai4ce-planck | 容器 `meshway-planck`；Server 容器 `meshway-prod-server` 也在这里 | zj2631（docker 组） |
| hk | cn-hk-dmit | 容器 `meshway-hk` | root |

节点 `kind` 只有两种：有 `unit` 的是 systemd 节点（journal、`systemctl`、`MainPID`），有 `container` 的是容器节点（`docker logs`、`docker exec`、容器内唯一的 `meshwayd`）。只有 it7 是出口（`relay_or_exit`）；prod 全直连，没有中继。

## 采集 key 与 gate

采集 key 在每台主机的 `authorized_keys` 中只能运行 gate：

```
command="python3 ~/.meshway-soak-gate/gate.py --spec ~/.meshway-soak-gate/<host>.json <host>",from="193.123.237.166",no-pty,no-port-forwarding,no-agent-forwarding,no-X11-forwarding,no-user-rc ssh-ed25519 AAAA… meshway-soak-collector
```

gate 读 `SSH_ORIGINAL_COMMAND`，用 shlex 切词（不交给 shell），要求与允许表中的一个命令形完全一致、每个参数逐个校验（节点必须属于本机，容器名／unit 来自 spec，cursor、时间、端口、秒数、版本号各有严格格式，ping 目标与 iperf 对端来自配置的边与吞吐设置）；不符的一律以 126 退出并写 syslog（`meshway-soak-gate`，auth facility）与 stderr。允许的命令以固定参数数组 `execvp` 执行；只有 gate 自带的固定脚本（PID／启动时刻、60 秒 schedstat 窗口、curl 循环、nft 兜底）在 `sh -c` 中运行，请求方的字只以校验过的整数（秒数）进入这些脚本。kr 本机由采集脚本以 `--command` 直接调用同一个 gate。

| 主机 | 读取（本机节点） | 故障（只有日程点名的） |
| --- | --- | --- |
| it7 | `status it7`、`resources it7 <s>`、`inspect it7`（`systemctl show`）、`journal it7 --after-cursor C｜--since T`、`ping it7 <jp/kr/planck/hk 的 Mesh 名>` | `restart it7`（`systemctl restart meshwayd`） |
| jp | 同上（jp）、`iperf-client jp <kr Mesh IPv4> 5299 <≤30 s> <≤100 Mbit>` | `restart jp`；若 D6 选 jp：`exit-select jp us-exit`、`exit-default jp`、`curl-loop jp <s>` |
| kr | `status`／`resources`／`inspect`／`docker-logs`／`docker-logs-count kr`、`ping kr …`、`iperf-server kr 5299 <s>` | `kill9 kr` |
| planck | 同上（planck）；`inspect server`、`docker-logs`／`docker-logs-count server` | `nft-drop planck udp <s>`、`nft-clear planck`；Server：`server-upgrade <vX.Y.Z[-pre.N]>`、`server-stop`、`server-start`、`server-rotate-key` |
| hk | 同上（hk） | `docker-restart hk`、`nft-drop hk tcp443 <s>`、`nft-clear hk`；若 D6 选 hk：`exit-select hk us-exit`、`exit-default hk`、`curl-loop hk <s>` |

所有主机另有 `self-test`（返回 spec 中的节点与故障，以及 `sudo -n` 是否可用）。完整的形与参数格式见 `gate.py` 文档字符串，测试 `scripts/test_soak_gate.py` 逐条覆盖允许表、越权与注入（`;`、`$()`、反引号、换行、多余参数、越界数值、别的主机的节点）。

## 安装（协调者在采集机上执行）

1. kr 上：`sudo apt-get install -y python3 zstd util-linux`；`timedatectl` 确认时区为 UTC（cron 按系统时区解释时刻）。
2. 从 Mac 拷脚本：`ssh ubuntu@193.123.237.166 'mkdir -p -m 700 ~/meshway-soak-src ~/meshway-soak' && scp scripts/soak/* ubuntu@193.123.237.166:meshway-soak-src/`，再拷私有配置 `scp .meshway/prod/soak/config.json ubuntu@193.123.237.166:meshway-soak/config.json`（先填好剩余的 3 个 `REPLACE`）。
3. kr 上 `sh ~/meshway-soak-src/install.sh --no-cron`：复制脚本到 `~/meshway-soak/bin/`、建目录（0700）、生成采集 key（已存在则不动）、渲染 `gate/<host>.json`、打印每台远端主机的 gate 安装命令与 `authorized_keys` 行。它不写任何其他主机；没有 `config.json` 时先放一份模板并停下。
4. 在已有各主机访问权的机器上执行打印出的命令：建 `~/.meshway-soak-gate/`（0700），拷 `gate.py` 与该主机的 spec（可在 Mac 上用 `python3 scripts/soak/render.py --config .meshway/prod/soak/config.json gates <dir>` 渲染出同样的 spec），`self-test`，再把那一行追加到该账户的 `authorized_keys`。先在 kr 上 `curl -4 https://ifconfig.me` 核对出站地址是 193.123.237.166（ssh 默认 `AddressFamily=inet`）。
5. host key：对每个目标 `ssh-keyscan -t ed25519 <host> >> ~/meshway-soak/known_hosts`，与目标上 `ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub` 的指纹带外比对（配置里 `StrictHostKeyChecking=yes`）。
6. 冒烟（不注入）：
   ```sh
   cd ~/meshway-soak
   python3 bin/fault.py --config config.json check      # 每台 gate 的 self-test 与日程一致；kr 容器的 RestartPolicy、systemd 的 Restart=
   python3 bin/sample.py --config config.json && tail -1 state/sample.log
   python3 bin/resources.py --config config.json --window 5 && tail -1 state/resources.log
   python3 bin/logs.py --config config.json && tail -1 state/logs.log
   python3 bin/report.py --config config.json probe && tail -1 state/probe.log
   python3 bin/fault.py --config config.json --dry-run --force run d6-planck-udp-drop   # 只打印 gate 调用，不写标记
   ```
   冒烟样本落在 t0 之前，报告按 `[t0, t0+7d)` 截取，不计入。
7. `sh ~/meshway-soak/bin/install.sh`（不带 `--no-cron`）安装 cron 块：只替换 `# BEGIN meshway-soak`／`# END meshway-soak` 之间的行，其他 crontab 行不动。`sh install.sh --uninstall` 只删除该块。

改 `config.json` 中影响 gate 的部分（节点、边、吞吐、故障日程、D6 节点）后重跑 `install.sh` 并重新分发对应主机的 spec；`fault.py check` 会发现 spec 与配置不一致。

## cron

`install.sh` 渲染的块（`$S` = `~/meshway-soak`）：

```
*/5 * * * * flock -n $S/state/sample.lock    python3 $S/bin/sample.py    --config $S/config.json >>$S/state/cron.log 2>&1
1 * * * *   flock -n $S/state/resources.lock python3 $S/bin/resources.py --config $S/config.json …
3 * * * *   flock -n $S/state/logs.lock      python3 $S/bin/logs.py      --config $S/config.json …
30 23 * * * flock -n $S/state/probe.lock     python3 $S/bin/report.py    --config $S/config.json probe …
10 0 * * *  flock -n $S/state/report.lock    python3 $S/bin/report.py    --config $S/config.json …
20 0 * * *  flock -n $S/state/final.lock     python3 $S/bin/report.py    --config $S/config.json --final --no-collect …
* * * * *   flock -n $S/state/fault.lock     python3 $S/bin/fault.py     --config $S/config.json due …
```

- 资源窗口 :01:00–:02:05 与 5 分钟样本、日志任务错开（Status 读取本身会唤醒 daemon）。
- 日报前 `report.py` 在内部 `logs-inner` 锁下补跑一次日志采集，保证前一日的尾部已存。
- `fault.py due` 每次最多执行一个故障；长故障（D3 30 分钟）持有锁，期间其余分钟的 `due` 直接退出。
- `final.md` 每天更新为截至当日的累计判定；D8 00:20Z 那一次即最终结果。

## 采集口径

- **Status**：`meshway --json status` 原样保存，另附 `@@proc`（systemd 节点取 unit 的 `MainPID`，容器取容器内唯一的 `meshwayd`；PID 与 `/proc/<pid>/stat` 启动时刻）。进程身份以 Status 的 `processId` 为准，启动时刻只在探到的 PID 与之相同时使用。
- **Server**：`GET /api/v1/auth/options`（与 `install.sh` 的健康检查相同），以及宿主上 `docker inspect` 的 `StartedAt`／`RestartCount`／镜像。
- **CPU**：[空闲 CPU 口径](../../docs/v2/node/performance.md#idle-cpu)——60 秒窗口内每线程 `schedstat` 运行时间差、进程合计（窗口内两次都在的线程之和）、已回收子进程 `cutime+cstime` 差（出口节点的 `nft`／`sh`）。单位为一个核的百分比。
- **转发包数**：同一窗口内节点 netns 的 `/proc/net/snmp` UDP 收发数据报＋TCP 收发段之和的一半（每个转发包一进一出）。它包含宿主其他流量，是上界：预算 = 固定周期开销 ＋ pps × 29 µs 因此只会偏宽。
- **日志**：journal 条目保存 `t`（`__REALTIME_TIMESTAMP`）、`pid`、`boot`、`prio`、`sys`（PID 1／systemd 自身的行，如 `Started …`）与 `msg` 原文；docker 行保存时间戳与原文。日志原文是 daemon／Server 的结构化输出，按[运行日志](../../docs/v2/logging.md)不含凭据。

## 容量估算

| 数据 | 实测单位量（dev 演练 `.meshway/validation/soak1/`，4 节点 1 小时） | 7 天 prod 估算（5 节点＋Server） |
| --- | --- | --- |
| Status 样本 | 每条 2.2–4.3 KiB（随 peer／socket 数增长）；zstd 后 1/15–1/17 | 5 × 288 × 7 × ≤ 8 KiB ≈ 80 MiB 未压缩，压缩后 ≈ 5 MiB |
| 资源样本 | 每条约 1.5 KiB × 24／天／节点 | < 2 MiB |
| 日志 | 退化状态（反复 retrying、撤回／重装）每节点 450–880 行／时、140–300 KiB／时，每行约 310 B；zstd 后 1/27 | 最坏 6 × 300 KiB × 24 × 7 ≈ 300 MiB 未压缩（当日一份），压缩后 ≈ 11 MiB |

30 天保留合计约 100 MiB（压缩后）；10 GiB 下限只用于防止宿主被意外填满。每次日志读取每来源最多 200,000 行（其余下一次继续）。

## docker 日志轮转

节点容器用 Docker `local` 驱动 10m×3，Server 容器用 `json-file` 50m×10。按实测日志量（退化状态下每节点每小时 ≤ 300 KiB，Server 更少），两次读取（每小时一次）之间写满一个 10 MiB 文件需要 30 小时以上，所以轮转不会在两次读取之间丢掉未读的行。`logs.py` 仍会检查：每次读取前数一下上一个有内容的时间窗在 docker 中还剩几行；一行都不剩（最早保留的行晚于上次时间窗末尾）就在 `state/log-gaps.jsonl` 记一条，日报的“可能缺行”节列出，覆盖门槛行标为未过。journal 的 cursor 被 vacuum 读不出时同样记一条，并从上次存下的最后时刻续读（重叠部分由报告去重）。

## 故障日程

T0 所在 UTC 日为 D1。`config.json` 的 `faults[]` 定义（`day`、`at`、`action` 与参数）。每次注入：`start` 标记（`planned_at`、`affects`、`grace_min`）→ 经 gate 的动作 → `end` 标记（`ok`、`detail`、`error`）；动作失败、超时或进程被打断都会留下 `end`（打断的在下一次 `due`／`recover` 时执行清理并写 `interrupted: true`）。

| id | 时刻 | `action` | gate 命令形与过程 | 影响范围 |
| --- | --- | --- | --- | --- |
| d2-server-upgrade | D2 10:00Z | `server_upgrade`（`version`） | `server-upgrade <version>`：planck 上把 Release 的 Server tar 与 `SHA256SUMS` 下载到暂存目录（已有则不下载），执行 PROD-1 的 `sudo sh server-upgrade.sh <tar> <SHA256SUMS>`（校验、预检、停、一次性容器 migrate、换程序目录、启动、健康；新进程未答 sync 时自动回退），之后等公网健康 | Server＋全部节点 |
| d3-server-stop | D3 10:00Z | `server_stop` | `server-stop` → 30 分钟 → `server-start`（`finally` 中一定执行），等健康 | Server＋全部节点 |
| d4-rotate-netmap-key | D4 10:00Z | `server_rotate_key` | `server-rotate-key`：`docker stop` → 一次性容器（`--network none`、同 uid、程序只读、同 data dir）`meshway-server rotate-netmap-key --data-dir …` → `docker start`（失败也执行），等健康 | Server＋全部节点 |
| d5-jp-restart | D5 10:00Z | `systemd_restart` | `restart jp`，等新 PID 的 running／ready（≤ 300 s） | jp |
| d5-kr-kill9 | D5 12:00Z | `kill9` | 先 `inspect kr` 核对 `HostConfig.RestartPolicy`（`unless-stopped`／`always`，否则不杀）；`kill9 kr`：gate 在宿主找到该容器内 `meshwayd` 的宿主 PID（父进程为容器 init）并 `sudo kill -9`，容器 init 随之退出，由 Docker 重启策略拉起；等新 PID 的 running／ready。systemd 节点则对 `MainPID` 发 SIGKILL，要求 `Restart=on-failure`／`always` | kr |
| d5-hk-docker-restart | D5 14:00Z | `docker_restart` | `docker-restart hk`，等 running／ready | hk |
| d5-it7-restart | D5 16:00Z | `systemd_restart` | 同 jp | it7 |
| d6-planck-udp-drop | D6 10:00Z | `nft_drop` `udp` | `nft-drop planck udp 300`：planck 容器 netns 内建表 `inet meshway_soak_fault`（input／output／forward 丢弃 UDP，回环除外），先 `docker exec -d` 放 `sleep 420; nft delete table …` 兜底；5 分钟后 `nft-clear planck` | planck |
| d6-hk-server-tcp-drop | D6 12:00Z | `nft_drop` `tcp443` | `nft-drop hk tcp443 600`：hk 容器 netns 内同一独立表的 output 链丢弃全部 `tcp dport 443`（`inet` 表，IPv4＋IPv6）。Server 经 Cloudflare 任播，按解析地址拦截不可靠，所以按端口；hk 的对端承载用 21820／21821，不受影响。兜底同上，10 分钟后清除 | hk |
| d6-us-exit | D6 14:00Z | `exit_profile`（`node`） | `exit-select <node> us-exit` → `curl-loop <node> 600` → `exit-default <node>`（`finally`）；记录 `appliedProfile` 与成功次数，零成功判失败。`node` 由配置给定：jp（若 X19 入站回包与 DNS-2 外来搜索域的修复已在 D6 前部署），否则 hk——jp 当前选全量出口会断入站 SSH，且 DNS 安装被 `foreign_domain` 挂起 | 出口节点 |

curl 循环：每 2 秒一次 `curl -m 10 --max-filesize 65536 --limit-rate 64k https://www.example.com/`（正文 577 B），10 分钟至多 300 次；每次连同 TLS 握手约 6 KB，总量约 2 MB，对 hk（1 核、448 MB 容器）可忽略。

物理机只有进程级动作；宿主防火墙、路由不改。到点 30 分钟内未执行（采集机停机等）的故障记为 `skipped`，需要时用 `fault.py run <id>` 手工补跑（标记记录实际时刻）。

## 门槛判定

`report.py` 对给定日期范围（日报：当日；`--final`：`[t0, t0+7d)`）计算，结果为 通过／未过／不完整（缺测不填零）。**计划内**＝事件时刻落在某个影响该节点的故障窗口 `[start, end + grace_min]`（默认 15 分钟；`*` 指全部节点，不含 Server）内。

| 门槛 | 判定 |
| --- | --- |
| 零人工重启 | 相邻两次 Status 样本的进程身份（`processId`＋启动时刻）变化即一次进程启动；Server 以容器 `StartedAt` 变化计。进程启动只有在点名该节点（或 Server）的故障窗口内才算计划内——`*` 不适用，Server 故障期间的 Node 重启算计划外。计划外启动必须为 0。systemd 的 `Started …`／`Main process exited` 行只作佐证 |
| 每次 `blocked` 有已证实原因 | 采样中连续 `phase: blocked` 为一段，每段所有样本都必须有 `lastError`（`lastErrorDetail` 同列）；日志中 `phase=Blocked` 的 Control 事件另列。计划外的段列为“待人工确认”——工具只能检查原因已给出，是否为真实的已证实不一致由人确认 |
| 空闲 CPU | 每次资源样本（故障窗口内的排除）：最忙线程 < 5 %；`relay_or_exit` 节点另要求进程合计 ≤ `fixed_pct`（默认 8 %）＋ pps × 29 µs、子进程 ≤ 5 %。缺测计数单列 |
| RSS／fd／socket 无持续增长 | 自 t0 起按进程段（同一 PID 与启动时刻）分别判定，段长 ≥ 24 h 才判定。**增长**＝最小二乘斜率外推 7 天 > 限值 **且** 末 24 h 中位 − 首 24 h 中位 > 限值 / 4。限值：RSS max(8 MiB, 首 24 h 中位 × 10 %)，fd 与 socket 各 8 |
| 计划外 sync 失败只属于可达性 | journal／容器日志中的 `Node sync failed; retrying`：计划外的 `error` 只允许 `Network`／`ServerBusy`（tunnel 或 Server 不可达／忙）；其他种类（`Contract`、`SignatureInvalid`、`ServerKeyMismatch`、`Credentials`…）任一出现即未过。采集机的 Server 健康失败同表列出作佐证 |
| 每日 ping 10/10 | 配置中的 10 条边每天 23:30Z 各 `meshway ping -c 10`，全部 `success == 10`；缺测为不完整 |

**趋势限值的来源**：稳态 RSS 不随时间增长是 [匿名驻留与分配器对照](../../docs/v2/node/performance.md#匿名驻留与分配器对照) 的结论（mimalloc＋TUN 复用后匿名页平台化）；该节两次独立复验的 RSS 相差约 3.3 %（32.74／33.82 MiB），10 % 约为其 3 倍，8 MiB 下限避免小 RSS 节点被分配器颗粒度误判。fd／socket 随按需通道（空闲 120 s 关闭）上下波动，8 约等于一个 peer 新增一组 dial／listener socket 与 TUN／netlink 句柄；7 天持续多出这么多即视为泄漏。两个条件同时满足才判增长，单个尖峰（如每日 ping 建立的通道）不会触发。数值可在 `thresholds` 中调整，报告逐段给出斜率、外推、限值与首末中位，便于人工复核。

日报另含：故障表、每节点样本覆盖（< 90 % 标出）、phase 分布、续租次数、最少剩余租约、serial／revision 变化、按类别（`level message [error|problem]`）的 WARN／ERROR 计数、撤回（`… suspending packet plane`、`Netmap withdrawn …`、`Netmap lease ended …`）与重装（`… netmap reinstalled`、`Netmap installed again …`、`Kernel ownership recovered …`）及其字段、CPU／RSS／fd 每日中位与最大、探测结果。

## 中断与恢复

- **采集机重启／cron 停摆**：日志按 cursor／时间窗从断点继续（单次最多 200,000 行，余下次续）；样本缺口在覆盖率中显示，不补零。
- **任务被杀**：追加与保存 cursor 之间被打断最多重复一批日志，报告按 `(t, pid, msg)` 去重；被截断的 JSONL 末行读取时跳过；压缩先写 `.zst.tmp` 再改名，残留的未压缩文件下次删除。
- **故障中途中断**：下一次 `fault.py due`（或手动 `fault.py recover`）执行该故障的清理（`server-start`、`nft-clear`、`exit-default`）并写 `end`（`interrupted: true`）；nft 表另有容器内的兜底删除。被打断的 Server 升级不自动清理（按 install.md 的回退边界由人处理）。中断的故障不自动重跑。
- **磁盘满**：清理空间后自动恢复；当天日报标红并列出停写时刻。
- **改配置**：soak 期间不改 `t0` 与 `faults[].id`；改了影响 gate 的部分要重新分发 spec（见安装）。
