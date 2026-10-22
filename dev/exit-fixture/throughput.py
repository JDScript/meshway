"""Exit throughput and queue pressure (phase 16) on the real A→R→E row.

- mesh: iperf3 TCP from A to E's Mesh address over the same A→R→E channels,
  forward and reverse (`-R`: E sends).
- exit: iperf3 TCP from A to the external service through exit E, forward
  and reverse (`-R`: the service sends, its packets return through E's NAT
  and exit TUN).
- pressure: one exit TCP flow for 20 s while A pings the service every 0.2 s
  (every echo request is its own exit flow, so E installs projections all
  along): the TCP flow never stalls (every 1 s interval carries data, as a
  dropped packet costs a retransmission, never the flow) and at least 95 %
  of the echoes return.

Throughout the phase (diagnostics only, never asserted):
- packet captures of E's exit TUN and A's Mesh TUN (A without the Mesh
  iperf3 runs), each truncated to `SNAP` bytes and at most `CAPTURE_PACKETS`
  packets: at most CAPTURE_PACKETS × (SNAP + 16) bytes ≈ 86 MB per file;
- once a second on A and E: TUN counters, the Status summary (phase, data
  plane, exit role and flows, peers), on E the conntrack count and the
  `Exit queue full` count, on A the iperf3 sockets (`ss -ti`);
- per second of the phase, the daemons' log events by message and cause
  (circuit retirements, projection installs, held local packets, ...).

Throughput and retransmissions are recorded beside E's interface counters,
not asserted: they belong to the host. The daemons own every decision; no
exit, NAT or admission code lives here."""
import collections
from datetime import datetime
import json
import subprocess
import time

PORT = "5201"
SECONDS = 20
PRESSURE_PING = 0.2
PING_FLOOR = 0.95
# A 1 s interval below this carried (next to) nothing: the flow stalled.
STALL_MBPS = 0.1
QUEUE_FULL = "Exit queue full; packet dropped, its flow kept"
SNAP = 128
CAPTURE_PACKETS = 600000
STOP = "/evidence/phase16.stop"

PING = ("import json,os,subprocess,sys\n"
        "target,interval,count,path=sys.argv[1:5]\n"
        "r=subprocess.run(['ping','-n','-D','-i',interval,'-c',count,'-W','2',target],capture_output=True,text=True)\n"
        "open(path+'.tmp','w').write(json.dumps({'exitCode':r.returncode,'stdout':r.stdout}))\n"
        "os.replace(path+'.tmp',path)\n")

# One line per sample of the Status: what the phase needs, not the document.
STATUS = ("import json,sys\n"
          "try: s=json.load(sys.stdin)\n"
          "except Exception as e: print(json.dumps({'error':str(e)})); sys.exit()\n"
          "x=s.get('exit') or {}\n"
          "print(json.dumps({'phase':s.get('phase'),'dataPlane':s.get('dataPlane'),"
          "'applied':(s.get('applied') or {}).get('serial'),'lastError':s.get('lastError'),"
          "'exitRole':bool(x.get('installedRole')),'exitFlows':x.get('activeFlows'),"
          "'peers':[[p.get('hostname'),p.get('state'),p.get('transport'),p.get('rtt')] for p in s.get('peers') or []]}))\n")


def sampler(role):
    """The in-container 1 s sampling loop; it ends once `STOP` exists."""
    extra = {
        "a": f"ss -tinH '( dport = :{PORT} or sport = :{PORT} )' 2>&1",
        "e": "echo conntrack $(conntrack -C 2>&1); echo queueFull $(grep -c 'Exit queue full' /evidence/daemon.stderr)",
    }[role]
    return ("echo $$ > /evidence/phase16-sampler.pid\n"
            f"while [ ! -e {STOP} ]; do\n"
            " echo \"=== $(date +%s.%N)\"\n"
            " for flags in /sys/class/net/*/tun_flags; do\n"
            "  d=${flags%/tun_flags}; s=$d/statistics\n"
            "  echo \"tun ${d##*/} rx=$(cat $s/rx_packets) tx=$(cat $s/tx_packets)"
            " rxDropped=$(cat $s/rx_dropped) txDropped=$(cat $s/tx_dropped)\"\n"
            " done\n"
            f" {extra}\n"
            " echo \"status $(/fixtures/meshway --socket /state/node/control.sock --json status 2>/dev/null"
            " | python3 -c \"$STATUS\")\"\n"
            " sleep 1\n"
            "done\n")


def summary(stdout):
    """(transmitted, received) of one iputils ping run."""
    for line in stdout.splitlines():
        if "packets transmitted" in line:
            fields = line.replace(",", "").split()
            return int(fields[0]), int(fields[3])
    raise RuntimeError("ping printed no summary")


def measured(record):
    """Receiver rate, sender retransmissions and per-second rates of one run."""
    value = json.loads(record["stdout"])
    if record["exitCode"] or "error" in value:
        raise RuntimeError("iperf3 failed: " + str(value.get("error")))
    end = value["end"]
    return {"receivedMbps": round(end["sum_received"]["bits_per_second"] / 1e6, 1),
            "retransmits": end["sum_sent"].get("retransmits"),
            "intervalsMbps": [round(item["sum"]["bits_per_second"] / 1e6, 1) for item in value["intervals"]],
            "maxSndCwnd": [stream.get("sender", {}).get("max_snd_cwnd") for stream in end.get("streams", [])]}


def timeline(text, start, end):
    """Log events of [start, end] (Unix seconds) per second, by message and cause."""
    seconds = collections.defaultdict(collections.Counter)
    for line in text.splitlines():
        try:
            event = json.loads(line)
            stamp = datetime.fromisoformat(event["timestamp"].replace("Z", "+00:00")).timestamp()
        except (ValueError, KeyError, TypeError):
            continue
        if start - 1 <= stamp <= end + 1:
            fields = event.get("fields") or {}
            key = " / ".join(str(part) for part in (fields.get("message"), fields.get("operation"),
                                                     fields.get("cause") or fields.get("reason")) if part)
            seconds[int(stamp)][key] += 1
    return {str(second): dict(counts) for second, counts in sorted(seconds.items())}


def run(ctx):
    command, write, report = ctx.command, ctx.write, ctx.report
    result = {"seconds": SECONDS, "port": int(PORT)}
    started = time.time()

    def counters(stage):
        return {role: {"links": command(role, "ip", "-s", "-s", "link", required=False)["stdout"],
                       "snmp": command(role, "cat", "/proc/net/snmp", required=False)["stdout"],
                       "qdisc": command(role, "tc", "-s", "qdisc", required=False)["stdout"]}
                for role in ("a", "e")} | {"stage": stage}

    def tuns(role):
        """(name, IPv4 addresses) of each TUN of `role`."""
        links = json.loads(command(role, "ip", "-j", "address")["stdout"])
        flags = command(role, "/bin/sh", "-c", "ls /sys/class/net/*/tun_flags")["stdout"].split()
        names = {path.split("/")[4] for path in flags}
        return {link["ifname"]: [a["local"] for a in link.get("addr_info", []) if a.get("family") == "inet"]
                for link in links if link["ifname"] in names}

    a_tuns, e_tuns = tuns("a"), tuns("e")
    exit_tun = [name for name, addresses in e_tuns.items() if ctx.e_mesh not in addresses]
    if len(a_tuns) != 1 or len(exit_tun) != 1:
        raise RuntimeError(f"phase 16 capture interfaces not found: A {a_tuns}, E {e_tuns}")
    captures = [("e", exit_tun[0], []),
                ("a", next(iter(a_tuns)), ["not", "(", "host", ctx.e_mesh, "and", "tcp", "port", PORT, ")"])]
    result["diagnostics"] = {"captures": {role: "phase16-" + name + ".pcap" for role, name, _ in captures},
                             "snap": SNAP, "packetLimit": CAPTURE_PACKETS,
                             "samples": "phase16-samples.log"}
    for role in ("a", "e"):
        command(role, "rm", "-f", STOP)
    for role, name, expression in captures:
        ctx.detached(role, "/bin/sh", "-ec",
                     f"echo $$ > /evidence/phase16-{name}.pid; exec /usr/bin/tcpdump -Z root -U -n -i {name} "
                     f"-s {SNAP} -c {CAPTURE_PACKETS} -w /evidence/phase16-{name}.pcap " + " ".join(expression))
    for role, name, _ in captures:
        deadline = time.monotonic() + 10
        while command(role, "test", "-s", f"/evidence/phase16-{name}.pcap", required=False)["exitCode"]:
            if time.monotonic() >= deadline:
                raise RuntimeError("phase 16 capture readiness failed: " + role + "/" + name)
            time.sleep(.1)
    for role in ("a", "e"):
        ctx.detached(role, "env", "STATUS=" + STATUS, "/bin/sh", "-c",
                     sampler(role) + " > /evidence/phase16-samples.log 2>&1")

    def stop_diagnostics():
        try:
            join_diagnostics()
        except Exception as error:
            result["diagnostics"]["error"] = repr(error)
        write(report / "exit-throughput.json", result)

    def join_diagnostics():
        for role in ("a", "e"):
            command(role, "touch", STOP, required=False)
        for role, name, _ in captures:
            command(role, "/bin/sh", "-c", f"kill -INT $(cat /evidence/phase16-{name}.pid) 2>/dev/null; true",
                    required=False)
        deadline = time.monotonic() + 15
        for role, pid in [(role, f"phase16-{name}") for role, name, _ in captures] + \
                         [("a", "phase16-sampler"), ("e", "phase16-sampler")]:
            while not command(role, "/bin/sh", "-c", f"kill -0 $(cat /evidence/{pid}.pid) 2>/dev/null",
                              required=False)["exitCode"]:
                if time.monotonic() >= deadline:
                    result["diagnostics"].setdefault("unjoined", []).append(role + "/" + pid)
                    break
                time.sleep(.2)
        ended = time.time()
        result["diagnostics"]["window"] = [started, ended]
        result["diagnostics"]["logTimeline"] = {role: timeline(ctx.logs(role), started, ended)
                                                for role in ("a", "r", "e")}

    def iperf(server, target, reverse, seconds=SECONDS):
        ctx.detached(server, "iperf3", "-s", "-1", "-p", PORT)
        deadline = time.monotonic() + 10
        while not command(server, "ss", "-Hltn", "sport = :" + PORT, required=False)["stdout"].strip():
            if time.monotonic() >= deadline:
                raise RuntimeError("iperf3 server did not listen on " + server)
            time.sleep(.2)
        try:
            return command("a", "iperf3", "-c", target, "-p", PORT, "-t", str(seconds), "-J",
                           *(["-R"] if reverse else []), required=False, timeout=seconds + 30)
        except subprocess.TimeoutExpired:
            # What the stuck flow looks like on both ends before the phase fails.
            result["stuck"] = {"a": command("a", "ss", "-tinH", required=False),
                               "e": command("e", "conntrack", "-L", "-p", "tcp", "--dport", PORT, required=False),
                               "server": command(server, "ss", "-tinH", required=False),
                               "status": {role: ctx.status(role) for role in ("a", "e")}}
            raise RuntimeError(f"iperf3 -t {seconds} to {target} did not end within {seconds + 30} s")

    try:
        log_before = ctx.logs("e").count(QUEUE_FULL)
        runs = {}
        for name, server, target in (("mesh", "e", ctx.e_mesh), ("exit", "external", ctx.target)):
            for direction, reverse in (("forward", False), ("reverse", True)):
                before = counters(name + "-" + direction)
                record = iperf(server, target, reverse)
                runs[name + "-" + direction] = {"record": record, "before": before,
                                                "after": counters(name + "-" + direction), **measured(record)}
                write(report / "exit-throughput.json", result | {"runs": runs})
        result["runs"] = runs
        mesh, exit_ = runs["mesh-reverse"]["receivedMbps"], runs["exit-reverse"]["receivedMbps"]
        result["exitReverseOfMeshReverse"] = round(exit_ / mesh, 2) if mesh else None

        # Pressure: echo flows install beside a running exit TCP flow.
        path = "/evidence/exit-pressure-ping.json"
        command("a", "rm", "-f", path)
        ctx.start(PING, ctx.target, str(PRESSURE_PING), str(int(SECONDS / PRESSURE_PING)), path)
        result["pressureStarted"] = time.time()
        record = iperf("external", ctx.target, False)
        deadline = time.monotonic() + 30
        while (pinged := command("a", "cat", path, required=False))["exitCode"]:
            if time.monotonic() >= deadline:
                raise RuntimeError("pressure ping did not finish")
            time.sleep(.5)
        pinged = json.loads(pinged["stdout"])
        transmitted, received = summary(pinged["stdout"])
        pressure = {"record": record, "ping": pinged, "transmitted": transmitted, "received": received,
                    **measured(record)}
        result["pressure"] = pressure
        result["queueFullLogged"] = ctx.logs("e").count(QUEUE_FULL) - log_before
        result["eStatus"] = ctx.status("e")
    finally:
        stop_diagnostics()
    stalled = [index for index, rate in enumerate(pressure["intervalsMbps"]) if rate < STALL_MBPS]
    if stalled:
        raise RuntimeError(f"exit TCP flow stalled under echo installs at seconds {stalled}")
    if received < PING_FLOOR * transmitted:
        raise RuntimeError(f"echoes beside the exit TCP flow lost: {received}/{transmitted}")
    if not ctx.running(result["eStatus"]) or not (result["eStatus"].get("exit") or {}).get("installedRole"):
        raise RuntimeError("E's exit role did not stay installed under the pressure run")
    return result
