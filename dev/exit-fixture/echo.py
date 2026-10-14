"""Exit echo lifetime, capacity and recovery (phase 15) on the real A→R→E row.

Three steps, each on the real daemons:
- capacity: A pings an external address every 0.2 s for 60 s while it
  repeats `traceroute -I` to it. E's projections stay far below the exit
  owner's capacity (128), A loses no echo, E's role never fails.
- reconnect (OPS8): E's conntrack entry of A's echo id is deleted twice while
  A keeps pinging, as the ICMP conntrack timeout ends it while earlier
  sequences are still projected. The next sequence re-creates it; the role
  keeps serving.
- recovery: a return route of E's exit role is deleted under A's ping. Only
  the role stops; it reinstalls on the same content while E stays applied
  and connected, and A's system DNS (MagicDNS locally, ordinary names
  through E's resolver over the Mesh) answers throughout.

Validation reads the real CLI, E's exit journal and E's daemon log; the
daemons own every decision. No exit, NAT or admission code lives here."""
import json
import time

CAPACITY = 128
# Far below the capacity: a 5/s ping and repeated traceroutes, with echo
# projections ending at their reply, stay under a quarter of it.
PEAK_LIMIT = CAPACITY // 4
LOAD_SECONDS = 60
OWNER_STOPPED = "Exit owner stopping after a failed kernel operation"
ROLE_FAILED = "Exit role failed; withdrawing it alone"

PING = ("import json,os,subprocess,sys\n"
        "target,interval,count,path=sys.argv[1:5]\n"
        "r=subprocess.run(['ping','-n','-i',interval,'-c',count,'-W','2',target],capture_output=True,text=True)\n"
        "open(path+'.tmp','w').write(json.dumps({'exitCode':r.returncode,'stdout':r.stdout}))\n"
        "os.replace(path+'.tmp',path)\n")
TRACE = ("import json,os,subprocess,sys,time\n"
         "target,seconds,path=sys.argv[1],float(sys.argv[2]),sys.argv[3]\n"
         "runs=[]; start=time.monotonic()\n"
         "while time.monotonic()-start<seconds:\n"
         " r=subprocess.run(['traceroute','-I','-n','-q','1','-w','2','-m','5',target],capture_output=True,text=True)\n"
         " runs.append({'exitCode':r.returncode,'stdout':r.stdout}); time.sleep(2)\n"
         "open(path+'.tmp','w').write(json.dumps(runs)); os.replace(path+'.tmp',path)\n")


def summary(stdout):
    """(transmitted, received) of one iputils ping run."""
    for line in stdout.splitlines():
        if "packets transmitted" in line:
            fields = line.replace(",", "").split()
            return int(fields[0]), int(fields[3])
    raise RuntimeError("ping printed no summary")


def connected(sample):
    """E stayed applied, ready and connected to its Server."""
    return (sample["phase"] == "running" and sample["dataPlane"] == "ready" and sample["applied"] is not None
            and sample["sync"] == "connected" and not sample["syncError"])


def run(ctx):
    command, write, report = ctx.command, ctx.write, ctx.report
    target = ctx.target
    result = {"capacity": CAPACITY, "peakLimit": PEAK_LIMIT}

    def projections():
        record = command("e", "cat", "/state/node/underlay-exit.json", required=False)
        if record["exitCode"]:
            return None
        return len(json.loads(record["stdout"])["projections"])

    def e_log():
        return ctx.logs("e")

    def ping_on_a(name, interval, count):
        path = "/evidence/" + name + ".json"
        command("a", "rm", "-f", path)
        ctx.start(PING, target, interval, str(count), path)
        return path

    def finished(path, timeout):
        deadline = time.monotonic() + timeout
        while True:
            record = command("a", "cat", path, required=False)
            if record["exitCode"] == 0:
                return json.loads(record["stdout"])
            if time.monotonic() >= deadline:
                raise RuntimeError("A ping did not finish: " + path)
            time.sleep(.5)

    def role_installed(value):
        return ctx.running(value) and (value.get("exit") or {}).get("installedRole") is not None

    stopped_before = e_log().count(OWNER_STOPPED)

    # Capacity: a 0.2 s ping for 60 s and traceroutes beside it.
    load = ping_on_a("echo-load", "0.2", LOAD_SECONDS * 5)
    command("a", "rm", "-f", "/evidence/echo-traces.json")
    ctx.start(TRACE, target, str(LOAD_SECONDS - 10), "/evidence/echo-traces.json")
    samples = []
    started = time.monotonic()
    while time.monotonic() - started < LOAD_SECONDS + 5:
        e = ctx.status("e")
        samples.append({"at": time.time(), "projections": projections(),
                        "activeFlows": (e.get("exit") or {}).get("activeFlows"),
                        "roleInstalled": role_installed(e)})
        time.sleep(.5)
    traces = finished("/evidence/echo-traces.json", 30)
    pinged = finished(load, 30)
    transmitted, received = summary(pinged["stdout"])
    counts = [sample["projections"] for sample in samples if sample["projections"] is not None]
    result["load"] = {"ping": pinged, "transmitted": transmitted, "received": received,
                      "traceroutes": traces, "samples": samples, "peak": max(counts, default=None)}
    write(report / "echo-capacity.json", result)
    if not counts or max(counts) > PEAK_LIMIT:
        raise RuntimeError(f"echo projections approach the exit capacity: peak {max(counts, default=None)}")
    if received < transmitted or not all(sample["roleInstalled"] for sample in samples):
        raise RuntimeError(f"echo load lost replies ({received}/{transmitted}) or failed E's role")
    if any(trace["exitCode"] or target not in trace["stdout"] for trace in traces):
        raise RuntimeError("a traceroute through the exit did not reach the target")

    # Reconnect: E's conntrack entry of A's echo id ends under the ping.
    reconnect = ping_on_a("echo-reconnect", "0.5", 40)
    time.sleep(5)
    deletions = []
    for _ in range(2):
        deletions.append(command("e", "/usr/sbin/conntrack", "-D", "-f", "ipv4", "-p", "icmp",
                                 "-s", ctx.source, required=False))
        time.sleep(6)
    pinged = finished(reconnect, 40)
    transmitted, received = summary(pinged["stdout"])
    e = ctx.status("e")
    result["reconnect"] = {"ping": pinged, "transmitted": transmitted, "received": received,
                           "deletions": deletions, "status": e}
    write(report / "echo-capacity.json", result)
    # A reply in flight at a deletion may miss its NAT entry.
    if (any(item["exitCode"] for item in deletions) or received < transmitted - len(deletions)
            or not role_installed(e) or e_log().count(OWNER_STOPPED) != stopped_before):
        raise RuntimeError(f"a re-created echo conntrack entry stopped E's role ({received}/{transmitted})")

    # Recovery: a deleted return route stops only the role; it reinstalls.
    recovery = ping_on_a("echo-recovery", "0.5", 60)
    deadline = time.monotonic() + 10
    while True:
        rules = json.loads(command("e", "ip", "-j", "-4", "rule")["stdout"])
        pair = [rule for rule in rules if rule.get("dst") == ctx.source and rule.get("src") == target]
        if pair:
            break
        if time.monotonic() >= deadline:
            raise RuntimeError("E has no return rule for A's echo")
        time.sleep(.2)
    table = str(pair[0]["table"])
    failed_before = e_log().count(ROLE_FAILED)
    deleted = command("e", "ip", "route", "del", ctx.source + "/32", "table", table)
    window, lookups = [], []
    started = time.monotonic()
    reinstalled = None
    while time.monotonic() - started < 25:
        e = ctx.status("e")
        window.append({"at": time.time(), "phase": e.get("phase"), "dataPlane": e.get("dataPlane"),
                       "applied": e.get("applied"), "sync": e.get("sync"), "syncError": e.get("syncError"),
                       "exit": e.get("exit")})
        lookups.append({"magic": ctx.lookup(ctx.magic), "ordinary": ctx.lookup(
            "recovery-%d-%s.%s" % (len(lookups), ctx.tag, ctx.zone))})
        if reinstalled is None and role_installed(e) and e_log().count(ROLE_FAILED) > failed_before:
            reinstalled = time.monotonic() - started
        time.sleep(.5)
    pinged = finished(recovery, 40)
    transmitted, received = summary(pinged["stdout"])
    log = e_log()
    result["recovery"] = {"table": table, "deleted": deleted, "window": window, "lookups": lookups,
                          "reinstalledAfterSeconds": reinstalled, "ping": pinged,
                          "transmitted": transmitted, "received": received,
                          "roleFailedLogged": log.count(ROLE_FAILED) - failed_before}
    write(report / "echo-capacity.json", result)
    if reinstalled is None:
        raise RuntimeError("E's exit role did not stop alone and reinstall after its route was deleted")
    if not all(connected(sample) for sample in window):
        raise RuntimeError("E's netmap or control connection failed with its exit role")
    if not all(item["magic"]["value"].get("addresses") == [ctx.own] and item["ordinary"]["value"].get("addresses")
               == ["198.51.100.10"] for item in lookups):
        raise RuntimeError("A's system DNS failed while E's exit role recovered")
    # Echoes sent while the role was down are refused; the later ones return.
    tail = [line for line in pinged["stdout"].splitlines() if "icmp_seq=" in line]
    if not tail or received == 0 or not any("icmp_seq=" + str(transmitted - 1) in line or
                                            "icmp_seq=" + str(transmitted) in line for line in tail):
        raise RuntimeError("A's echo through E did not resume after the role reinstalled")
    return result
