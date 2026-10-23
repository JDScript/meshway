# 现场 soak 工具

v0.1.0 [里程碑 C](../../docs/v2/process.md#v0-1-gate) 的 7 天 prod soak：采集、日志、按日程注入故障、日报与最终判定。运行在采集机 **kr-icn-arm64**（`ubuntu@193.123.237.166`；planck 出站 ssh 被封，所以不在 planck）。Python 3.8+ 标准库，压缩用系统 `zstd`（Python 3.14 起用内置 `compression.zstd`）。全部只读，除了 `fault.py` 按日程执行的注入。

| 文件 | 作用 |
| --- | --- |
| `common.py` | 配置、远程执行（本机／ssh／前缀）、时间、JSONL 追加与按日压缩、30 天保留、磁盘下限、`flock` |
| `sample.py` | 每 5 分钟：各节点 `meshway --json status`（附 daemon PID 与启动时刻）、Server 健康与容器状态 |
| `resources.py` | 每小时：60 秒 `schedstat` 每线程 CPU、进程合计、已回收子进程（`nft`／`sh`）、RSS、fd、socket、netns 包计数 |
| `logs.py` | 每小时：journal（cursor 续读）／`docker logs`（不重叠时间窗）增量 |
| `report.py` | 每天：日报；`--final` 整段判定；`probe` 子命令做每日 ping 与吞吐 |
| `fault.py` | 每分钟 `due`：到点执行故障并写 `faults.jsonl` 标记；`run`／`recover`／`list` |
| `config.example.json` | prod 配置模板（`REPLACE` 占位全部填完才能加载） |

## 目录

```
~/meshway-soak/
  config.json             配置（含 t0、节点、边、故障日程、门槛）
  bin/                    本目录的脚本
  keys/collector_ed25519  采集专用 key（0600）
  known_hosts             钉住的各节点 host key
  faults.jsonl            故障开始／结束／跳过标记（只追加）
  state/                  logs.json（journal cursor、docker since）、*.lock、各任务 *.log、cron.log、disk-stop.jsonl
  samples/<node>/<日>.jsonl            5 分钟 Status 样本（node = it7 jp kr planck hk server）
  samples/<node>/resources/<日>.jsonl  每小时资源样本
  samples/probes/<日>.jsonl            每日 ping／吞吐
  logs/<node>/<日>.jsonl[.zst]         日志（当日未压缩追加，过日压缩；node 含 server）
  reports/<日>.md|.json, final.md|.json
```

日期都是 UTC。`samples/`、`logs/`、`reports/` 中带日期的文件保留 30 天；样本与日志过日后压缩。磁盘剩余 < 10 GiB（`min_free_gib`）时所有写入停止（`faults.jsonl` 标记与 `state/disk-stop.jsonl` 除外，均为几十字节），日报首行标红；空间恢复后自动继续写入。

## 安装（协调者在 PROD-1 合流后执行）

1. kr 上：`sudo apt-get install -y python3 zstd util-linux`；`timedatectl` 确认时区为 UTC（cron 按系统时区解释时刻）。
2. 目录与脚本（从协调者的 Mac）：
   ```sh
   ssh ubuntu@193.123.237.166 'mkdir -p ~/meshway-soak/{bin,keys,state,samples,logs,reports} && chmod 700 ~/meshway-soak ~/meshway-soak/keys'
   scp scripts/soak/*.py scripts/soak/config.example.json ubuntu@193.123.237.166:meshway-soak/bin/
   ```
3. 采集 key（kr 上）：`ssh-keygen -t ed25519 -N '' -C meshway-soak-collector -f ~/meshway-soak/keys/collector_ed25519`。
4. 各远端账户的 `authorized_keys` 追加一行（it7 `root`，jp `ubuntu`，planck `zj2631`，hk 的 Docker 宿主账户）：
   ```
   from="193.123.237.166",no-port-forwarding,no-agent-forwarding,no-X11-forwarding,no-pty ssh-ed25519 AAAA… meshway-soak-collector
   ```
   ssh 默认 `AddressFamily=inet`，使来源恒为该 IPv4。先在 kr 上 `curl -4 https://ifconfig.me` 核对出站地址就是 193.123.237.166。注意：planck 的 `zj2631` 属于 docker 组，等同宿主 root；该行只授予采集机。
5. host key：对每个目标 `ssh-keyscan -t ed25519 <host> >> ~/meshway-soak/known_hosts`，并与目标上 `ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub` 的指纹带外比对；配置中 `StrictHostKeyChecking=yes`。
6. sudo：jp（ssh）与 kr（本机）需要免密 `sudo -n`（读 owner-only 的 Node socket、journal、`/proc/<pid>/fd`，以及 D5 的 `systemctl restart`／`kill -9`）；Oracle Ubuntu 镜像的 `ubuntu` 默认即是。it7 以 root 登录。容器节点经宿主 `docker exec`（容器内为 root）。
7. 配置：`cp bin/config.example.json config.json`，按 PROD-1 README 填完所有 `REPLACE`（t0、jp／hk 地址、容器名、Mesh 主机名、kr 的 Mesh IPv4、D2 升级与 D4 轮换命令、D2 目标版本）。`us-exit` 必须是 jp 被允许选择的方案。
8. 冒烟（不注入）：
   ```sh
   cd ~/meshway-soak
   python3 bin/sample.py --config config.json && tail -1 state/sample.log
   python3 bin/resources.py --config config.json --window 5 && tail -1 state/resources.log
   python3 bin/logs.py --config config.json && tail -1 state/logs.log
   python3 bin/report.py --config config.json probe && tail -1 state/probe.log
   python3 bin/fault.py --config config.json list
   python3 bin/fault.py --config config.json --dry-run --force run d6-planck-udp-drop   # 只打印命令，不写标记
   ```
   冒烟样本落在 t0 之前，报告按 `[t0, t0+7d)` 截取，不计入。
9. `crontab -e` 写入下表。

## cron

```
SHELL=/bin/sh
PATH=/usr/local/bin:/usr/bin:/bin
S=/home/ubuntu/meshway-soak
*/5 * * * * flock -n $S/state/sample.lock    python3 $S/bin/sample.py    --config $S/config.json >>$S/state/cron.log 2>&1
1 * * * *   flock -n $S/state/resources.lock python3 $S/bin/resources.py --config $S/config.json >>$S/state/cron.log 2>&1
3 * * * *   flock -n $S/state/logs.lock      python3 $S/bin/logs.py      --config $S/config.json >>$S/state/cron.log 2>&1
30 23 * * * flock -n $S/state/probe.lock     python3 $S/bin/report.py    --config $S/config.json probe >>$S/state/cron.log 2>&1
10 0 * * *  flock -n $S/state/report.lock    python3 $S/bin/report.py    --config $S/config.json >>$S/state/cron.log 2>&1
20 0 * * *  flock -n $S/state/final.lock     python3 $S/bin/report.py    --config $S/config.json --final --no-collect >>$S/state/cron.log 2>&1
* * * * *   flock -n $S/state/fault.lock     python3 $S/bin/fault.py     --config $S/config.json due >>$S/state/cron.log 2>&1
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

| 数据 | 单位量 | 7 天（6 个来源） |
| --- | --- | --- |
| Status 样本 | 每条约 8–25 KiB（随 peer 数）× 288／天／节点 | ≈ 5 × 288 × 7 × 25 KiB ≈ 250 MiB 未压缩，zstd 后约 1/15 |
| 资源样本 | 每条约 1.5 KiB × 24／天／节点 | < 2 MiB |
| 日志 | 实测（dev 现场，见 `.meshway/validation/soak1/`）：健康时约 50–150 行／时／节点，退化（反复 retrying）时约 700 行／时；每行存储约 250–400 B | 最坏 700 × 24 × 7 × 6 × 400 B ≈ 280 MiB 未压缩，压缩后 < 30 MiB |

30 天保留合计在 1 GiB 以内；10 GiB 下限只用于防止宿主被意外填满。每次日志读取每来源最多 200,000 行（其余下一次继续）。

## 故障日程

T0 所在 UTC 日为 D1。`config.json` 的 `faults[]` 定义（`day`、`at`、`action` 与参数）。每次注入：`start` 标记（`planned_at`、`affects`、`grace_min`）→ 动作 → `end` 标记（`ok`、`detail`、`error`）；动作抛错、超时或进程被打断都会留下 `end`（打断的在下一次 `due`／`recover` 时执行清理并写 `interrupted: true`）。

| id | 时刻 | 动作 | 实现 | 影响范围 |
| --- | --- | --- | --- | --- |
| d2-server-upgrade | D2 10:00Z | `command` | 在 planck 执行 PROD-1 README 的容器升级命令（`{version}` 参数化），超时 900 s，之后等 Server 健康 | Server＋全部节点 |
| d3-server-stop | D3 10:00Z | `server_stop` | `docker stop -t 30` → 30 分钟 → `docker start`（`finally` 中一定执行），等健康 | Server＋全部节点 |
| d4-rotate-netmap-key | D4 10:00Z | `command` | PROD-1 README 的离线轮换（停容器 → `meshway-server rotate-netmap-key` → 启动），等健康 | Server＋全部节点 |
| d5-jp-restart | D5 10:00Z | `systemd_restart` | `systemctl restart meshwayd`，等新 PID 的 running／ready（≤ 300 s） | jp |
| d5-kr-kill9 | D5 12:00Z | `kill9` | `kill -9 <MainPID>`，由 unit 的 `Restart=on-failure` 拉起，等新 PID 的 running／ready | kr |
| d5-hk-docker-restart | D5 14:00Z | `docker_restart` | `docker restart -t 30`，等 running／ready | hk |
| d5-it7-restart | D5 16:00Z | `systemd_restart` | 同 jp | it7 |
| d6-planck-udp-drop | D6 10:00Z | `nft_drop` `udp` | planck 容器 netns 内建表 `inet meshway_soak_fault`（input／output／forward 丢弃 UDP，回环除外）5 分钟后删表；先 `docker exec -d` 放一个 `sleep 420; nft delete table …` 兜底 | planck |
| d6-hk-server-tcp-drop | D6 12:00Z | `nft_drop` `tcp-to-server` | hk 容器内解析 Server 主机名（`getent ahosts`，记录到 `detail.addresses`），output 丢弃到这些地址的 TCP 10 分钟，兜底同上 | hk |
| d6-jp-us-exit | D6 14:00Z | `exit_profile` | jp `meshway exit us-exit` → 10 分钟每 2 秒 `curl` → `meshway exit --default`（`finally`）；记录 `appliedProfile` 与成功次数，零成功判失败 | jp |

物理机只有进程级动作；宿主防火墙、路由不改。到点 30 分钟内未执行（采集机停机等）的故障记为 `skipped`，需要时用 `fault.py run <id>` 手工补跑（标记记录实际时刻）。

## 门槛判定

`report.py` 对给定日期范围（日报：当日；`--final`：`[t0, t0+7d)`）计算，结果为 通过／未过／不完整（缺测不填零）。**计划内**＝事件时刻落在某个影响该节点的故障窗口 `[start, end + grace_min]`（默认 15 分钟；`*` 指全部节点，不含 Server）内。

| 门槛 | 判定 |
| --- | --- |
| 零人工重启 | 相邻两次 Status 样本的进程身份（`processId`＋启动时刻）变化即一次进程启动；Server 以容器 `StartedAt` 变化计。计划外启动必须为 0。systemd 的 `Started …`／`Main process exited` 行只作佐证 |
| 每次 `blocked` 有已证实原因 | 采样中连续 `phase: blocked` 为一段，每段所有样本都必须有 `lastError`（`lastErrorDetail` 同列）；日志中 `phase=Blocked` 的 Control 事件另列。计划外的段列为“待人工确认”——工具只能检查原因已给出，是否为真实的已证实不一致由人确认 |
| 空闲 CPU | 每次资源样本（故障窗口内的排除）：最忙线程 < 5 %；`relay_or_exit` 节点另要求进程合计 ≤ `fixed_pct`（默认 8 %）＋ pps × 29 µs、子进程 ≤ 5 %。缺测计数单列 |
| RSS／fd／socket 无持续增长 | 自 t0 起按进程段（同一 PID 与启动时刻）分别判定，段长 ≥ 24 h 才判定。**增长**＝最小二乘斜率外推 7 天 > 限值 **且** 末 24 h 中位 − 首 24 h 中位 > 限值 / 4。限值：RSS max(8 MiB, 首 24 h 中位 × 10 %)，fd 与 socket 各 8 |
| 计划外 sync 失败只属于可达性 | journal／容器日志中的 `Node sync failed; retrying`：计划外的 `error` 只允许 `Network`／`ServerBusy`（tunnel 或 Server 不可达／忙）；其他种类（`Contract`、`SignatureInvalid`、`ServerKeyMismatch`、`Credentials`…）任一出现即未过。采集机的 Server 健康失败同表列出作佐证 |
| 每日 ping 10/10 | 配置中的 10 条边每天 23:30Z 各 `meshway ping -c 10`，全部 `success == 10`；缺测为不完整 |

**趋势限值的来源**：稳态 RSS 不随时间增长是 [匿名驻留与分配器对照](../../docs/v2/node/performance.md#匿名驻留与分配器对照) 的结论（mimalloc＋TUN 复用后匿名页平台化）；该节两次独立复验的 RSS 相差约 3.3 %（32.74／33.82 MiB），10 % 约为其 3 倍，8 MiB 下限避免小 RSS 节点被分配器颗粒度误判。fd／socket 随按需通道（空闲 120 s 关闭）上下波动，8 约等于一个 peer 新增一组 dial／listener socket 与 TUN／netlink 句柄；7 天持续多出这么多即视为泄漏。两个条件同时满足才判增长，单个尖峰（如每日 ping 建立的通道）不会触发。数值可在 `thresholds` 中调整，报告逐段给出斜率、外推、限值与首末中位，便于人工复核。

日报另含：故障表、每节点样本覆盖（< 90 % 标出）、phase 分布、续租次数、最少剩余租约、serial／revision 变化、按类别（`level message [error|problem]`）的 WARN／ERROR 计数、撤回（`… suspending packet plane`、`Netmap withdrawn …`、`Netmap lease ended …`）与重装（`… netmap reinstalled`、`Netmap installed again …`、`Kernel ownership recovered …`）及其字段、CPU／RSS／fd 每日中位与最大、探测结果。

## 中断与恢复

- **采集机重启／cron 停摆**：日志按 cursor／since 从断点继续（单次最多 200,000 行，余下次续）；样本缺口在覆盖率中显示，不补零。
- **任务被杀**：追加与保存 cursor 之间被打断最多重复一批日志，报告按 `(t, pid, msg)` 去重；被截断的 JSONL 末行读取时跳过；压缩先写 `.zst.tmp` 再改名，残留的未压缩文件下次删除。
- **故障中途中断**：下一次 `fault.py due`（或手动 `fault.py recover`）执行该故障的清理（`docker start` Server、删 `meshway_soak_fault` 表、`meshway exit --default`）并写 `end`（`interrupted: true`）；nft 表另有容器内的兜底删除。中断的故障不自动重跑。
- **磁盘满**：清理空间后自动恢复；当天日报标红并列出停写时刻。
- **改配置**：soak 期间不改 `t0` 与 `faults[].id`；追加节点或修正命令可以，改动记入 `state/` 下的人工记录。
