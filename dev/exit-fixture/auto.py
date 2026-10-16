"""`auto` by the source's declared region (phase 14) on the real A→R→E row.

The controller (tools/meshway-runtime-fixture controller/exit.rs) applies,
on signals, a configuration where `exit` and `exit-alt` send names under
the name zone to E via R with a direct fallback, `auto` maps US → `exit`,
JP → `exit-alt` and falls back to `mesh`, and A's `region` is absent, US,
JP, FR, then the original configuration again. A has no direct route to the
external service, so a connection reaches it only through E. Validation
reads A's real CLI, its installed capture, the service's access log and the
Server's `routing.autoResolve` activities; the daemons own every decision."""
import ipaddress
import json
import time

FLOW = (
    "import json,os,socket,sys,time\n"
    "name,duration=sys.argv[1],float(sys.argv[2])\n"
    "address=socket.getaddrinfo(name,8080,socket.AF_INET,socket.SOCK_STREAM)[0][4]\n"
    "t=socket.create_connection(address,3); t.settimeout(3)\n"
    "u=socket.socket(socket.AF_INET,socket.SOCK_DGRAM); u.connect(address); u.settimeout(3)\n"
    "result={'address':address[0],'tcp':[],'udp':[],'done':False}\n"
    "def save():\n"
    " open('/evidence/auto-flow.tmp','w').write(json.dumps(result)); os.replace('/evidence/auto-flow.tmp','/evidence/auto-flow.json')\n"
    "def tcp(m):\n"
    " t.sendall(m); return t.recv(64)==m\n"
    "def udp(m):\n"
    " u.send(m); return u.recv(64)==m\n"
    "start=time.monotonic(); i=0\n"
    "while time.monotonic()-start<duration:\n"
    " m=b'auto-%d'%i\n"
    " for kind,probe in (('tcp',tcp),('udp',udp)):\n"
    "  at=time.time()\n"
    "  try: ok=probe(m)\n"
    "  except OSError: ok=False\n"
    "  result[kind].append([at,ok])\n"
    " save(); i+=1; time.sleep(1)\n"
    "result['done']=True; save()\n")


def resolution(selection):
    """`auto`'s (profile, basis, region) in a RoutingSelectionState; None
    when `auto` is not the applied profile."""
    if selection.get("appliedProfile") != "auto":
        return None
    auto = selection.get("auto") or {}
    return (auto.get("profile"), auto.get("basis"), auto.get("region"))


def ended_at_switch(flow, requested, switched):
    """Every probe before the switch succeeded and every probe after it
    failed, for TCP and UDP; one probe in flight while it commits may go
    either way."""
    for kind in ("tcp", "udp"):
        before = [ok for at, ok in flow[kind] if at < requested - 1]
        after = [ok for at, ok in flow[kind] if at > switched + 1]
        if not before or not all(before) or not after or any(after):
            return False
    return True


def run(c):
    """`c` carries the row's helpers: command, stage_config, exit_cli,
    wait_for, applied, lookup, connect, by_name, visits, capture, managed,
    start, write, report, target, zone, tag."""
    result = {"steps": {}}
    target = ipaddress.ip_address(c.target)

    def activities():
        record = c.command("controller", "cat", "/evidence/auto-activities.json", required=False)
        return json.loads(record["stdout"]) if record["exitCode"] == 0 else []

    def recorded(count, step):
        """The Server's `routing.autoResolve` activities, newest first, once
        `count` exist: one per change of the resolution."""
        c.wait_for(lambda: len(activities()) >= count, "routing.autoResolve at " + step, 15)
        found = activities()
        result.setdefault("activities", {})[step] = found
        if len(found) != count or any((event["actor"] or {}).get("type") != "system" for event in found):
            raise RuntimeError("resolution changes were not recorded once each at " + step)
        return [event["target"].get("profile") for event in found]

    def expect(step, selection, profile, basis, region):
        found = resolution(selection)
        result["steps"].setdefault(step, {})["selection"] = selection
        if found != (profile, basis, region):
            raise RuntimeError("auto at " + step + " resolved to " + json.dumps(found))

    def unreachable(step):
        visits = c.visits()
        record = c.connect(c.target)
        result["steps"].setdefault(step, {})["unreachable"] = record
        if record["exitCode"] == 0 or c.visits() != visits:
            raise RuntimeError("an external flow left through E at " + step)

    def through_e(step, name):
        """A name answered by E's resolver: its association lets a
        connection by name, and to the bare address, leave through E."""
        visits = c.visits()
        found = c.lookup(name)
        if found["value"].get("addresses") != [c.target]:
            raise RuntimeError("the name did not resolve at " + step)
        named = c.by_name(name)
        bare = c.by_name(c.target)
        result["steps"].setdefault(step, {}).update({"lookup": found, "byName": named, "bare": bare})
        if named["exitCode"] or bare["exitCode"] or c.visits() != visits + 2:
            raise RuntimeError("a connection did not leave through E at " + step)
        if not c.managed(c.capture("auto-" + step), target):
            raise RuntimeError("the associated address is not in A's capture at " + step)

    def name(step):
        return "auto-" + step + "-" + c.tag + "." + c.zone

    # 1. No region: auto is its fallback `mesh`; nothing leaves through E.
    result["configured"] = c.stage_config("exit-auto-configure", "exit-auto-configured.json")
    selected = c.exit_cli("auto")
    if not c.applied(selected, "auto"):
        raise RuntimeError("meshway exit auto was not applied")
    expect("none", selected["local"], "mesh", "noTrustedRegion", None)
    unreachable("none")
    recorded(1, "none")

    # 2. US: `exit`; E recomputes A's selection from its own projection, or
    # the circuit's handshake would fail.
    result["us"] = c.stage_config("exit-auto-us", "exit-auto-us.json")
    expect("us", c.exit_cli(), "exit", "label", "US")
    through_e("us", name("us"))
    recorded(2, "us")

    # 3. JP: `exit-alt`. Flows of the US resolution end, its association is
    # gone (the bare address no longer follows the name rule), a new name
    # leaves through E under the new resolution, and the Server recorded
    # exactly one resolution change.
    c.start(FLOW, name("us-flow"), "40")

    def flow():
        record = c.command("a", "cat", "/evidence/auto-flow.json", required=False)
        return json.loads(record["stdout"]) if record["exitCode"] == 0 else None
    c.wait_for(lambda: sum(ok for _, ok in (flow() or {}).get("tcp", [])) >= 3, "auto flow before JP", 30)
    requested = time.time()
    result["jp"] = c.stage_config("exit-auto-jp", "exit-auto-jp.json")
    switched = time.time()
    expect("jp", c.exit_cli(), "exit-alt", "label", "JP")
    visits = c.visits()
    result["steps"]["jp"]["bareAfterSwitch"] = c.by_name(c.target)
    if result["steps"]["jp"]["bareAfterSwitch"]["exitCode"] == 0 or c.visits() != visits:
        raise RuntimeError("the US association outlived the resolution change")
    c.wait_for(lambda: (flow() or {}).get("done"), "auto flow across JP", 60)
    result["flow"] = flow()
    if not ended_at_switch(result["flow"], requested, switched):
        raise RuntimeError("the resolution change did not end the US flows")
    c.wait_for(lambda: not c.managed(c.capture("auto-jp-released"), target), "US association released", 120)
    through_e("jp", name("jp"))
    if recorded(3, "jp") != ["exit-alt", "exit", "mesh"]:
        raise RuntimeError("the JP resolution change was not recorded")

    # 4. A manual selection overrides auto; selecting auto again returns to
    # the JP resolution.
    manual = c.exit_cli("exit")
    if not c.applied(manual, "exit") or manual["local"].get("auto") is not None:
        raise RuntimeError("meshway exit exit did not override auto")
    result["steps"]["manual"] = {"selection": manual["local"]}
    back = c.exit_cli("auto")
    if not c.applied(back, "auto"):
        raise RuntimeError("meshway exit auto was not applied again")
    expect("back", back["local"], "exit-alt", "label", "JP")
    # The override is no resolution change; returning to auto is one.
    recorded(4, "back")

    # 5. FR: listed by no rule, so the fallback `mesh`.
    result["fr"] = c.stage_config("exit-auto-fr", "exit-auto-fr.json")
    expect("fr", c.exit_cli(), "mesh", "noRuleMatched", "FR")
    c.wait_for(lambda: not c.managed(c.capture("auto-fr"), target), "mesh captures nothing", 120)
    unreachable("fr")
    if recorded(5, "fr") != ["mesh", "exit-alt", "exit-alt", "exit", "mesh"]:
        raise RuntimeError("the FR resolution change was not recorded")

    # 6. Clearing the persistent selection follows the default `auto` again:
    # from a manual `exit`, `meshway exit --default` leaves no requested
    # profile (the clearing's version stays, the base of the next change) and
    # resolves FR as before; returning to auto is one change.
    if not c.applied(c.exit_cli("exit"), "exit"):
        raise RuntimeError("meshway exit exit was not applied before clearing")
    cleared = c.exit_cli("--default")
    result["steps"]["cleared"] = {"selection": cleared["local"]}
    if (not c.applied(cleared, "auto") or cleared.get("cleared") is not True
            or cleared["local"].get("requestedProfile") is not None
            or not isinstance(cleared["local"].get("version"), int)):
        raise RuntimeError("meshway exit --default did not clear the selection")
    expect("cleared", cleared["local"], "mesh", "noRuleMatched", "FR")
    if recorded(6, "cleared") != ["mesh", "mesh", "exit-alt", "exit-alt", "exit", "mesh"]:
        raise RuntimeError("returning to the default auto was not recorded")

    # The later phases run on the original configuration and `exit`.
    if not c.applied(c.exit_cli("exit"), "exit"):
        raise RuntimeError("the exit profile was not selected again")
    result["restored"] = c.stage_config("exit-auto-restore", "exit-auto-restored.json")
    c.write(c.report / "auto-profile.json", result)
    return result
