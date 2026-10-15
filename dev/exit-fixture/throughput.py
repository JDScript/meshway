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

Throughput and retransmissions are recorded beside E's interface counters,
not asserted: they belong to the host. The daemons own every decision; no
exit, NAT or admission code lives here."""
import json
import time

PORT = "5201"
SECONDS = 20
PRESSURE_PING = 0.2
PING_FLOOR = 0.95
# A 1 s interval below this carried (next to) nothing: the flow stalled.
STALL_MBPS = 0.1
QUEUE_FULL = "Exit queue full; packet dropped, its flow kept"

PING = ("import json,os,subprocess,sys\n"
        "target,interval,count,path=sys.argv[1:5]\n"
        "r=subprocess.run(['ping','-n','-i',interval,'-c',count,'-W','2',target],capture_output=True,text=True)\n"
        "open(path+'.tmp','w').write(json.dumps({'exitCode':r.returncode,'stdout':r.stdout}))\n"
        "os.replace(path+'.tmp',path)\n")


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


def run(ctx):
    command, write, report = ctx.command, ctx.write, ctx.report
    result = {"seconds": SECONDS, "port": int(PORT)}

    def counters(stage):
        return {role: {"links": command(role, "ip", "-s", "-s", "link", required=False)["stdout"],
                       "snmp": command(role, "cat", "/proc/net/snmp", required=False)["stdout"],
                       "qdisc": command(role, "tc", "-s", "qdisc", required=False)["stdout"]}
                for role in ("a", "e")} | {"stage": stage}

    def iperf(server, target, reverse, seconds=SECONDS):
        ctx.detached(server, "iperf3", "-s", "-1", "-p", PORT)
        deadline = time.monotonic() + 10
        while not command(server, "ss", "-Hltn", "sport = :" + PORT, required=False)["stdout"].strip():
            if time.monotonic() >= deadline:
                raise RuntimeError("iperf3 server did not listen on " + server)
            time.sleep(.2)
        record = command("a", "iperf3", "-c", target, "-p", PORT, "-t", str(seconds), "-J",
                         *(["-R"] if reverse else []), required=False, timeout=seconds + 30)
        return record

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
    write(report / "exit-throughput.json", result)
    stalled = [index for index, rate in enumerate(pressure["intervalsMbps"]) if rate < STALL_MBPS]
    if stalled:
        raise RuntimeError(f"exit TCP flow stalled under echo installs at seconds {stalled}")
    if received < PING_FLOOR * transmitted:
        raise RuntimeError(f"echoes beside the exit TCP flow lost: {received}/{transmitted}")
    if not ctx.running(result["eStatus"]) or not (result["eStatus"].get("exit") or {}).get("installedRole"):
        raise RuntimeError("E's exit role did not stay installed under the pressure run")
    return result
