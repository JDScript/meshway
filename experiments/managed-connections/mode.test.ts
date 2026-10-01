import assert from "node:assert/strict";
import test from "node:test";
import { assertDemoContainer, IMMUTABLE_FILES, metadata } from "./demo.ts";
import { agentArguments, bootstrapArguments, BOOTSTRAP_CAPS, changeMode, demoMode, modeJournal } from "./mode.ts";
import type { DemoMode, ModeActions, ModeJournal, ModeSnapshot } from "./mode.ts";
import { packetInterface } from "./packet-readiness.ts";
import { shellMode } from "./upgrade.ts";

const image = `sha256:${"1".repeat(64)}`;
const original: ModeSnapshot = { image_id: image, shell: "readonly", mode: "neighbors", binding: "2".repeat(64), plan_revision: 7, plan_digest: "3".repeat(64),
  files: Object.fromEntries(IMMUTABLE_FILES.map((file) => [file, "4".repeat(64)])) };

function fixture(initial: DemoMode = "neighbors") {
  const current = { ...structuredClone(original), mode: initial };
  const disk: { journal: ModeJournal | null; mode: DemoMode; running: boolean } = { journal: null, mode: initial, running: true };
  const events: string[] = [];
  let epoch = 7; let crash: string | null = null; let packetReady = true;
  const event = (name: string) => { events.push(name); if (name === crash) { crash = null; throw Error("simulated crash"); } };
  const actions: ModeActions = {
    snapshot: async () => structuredClone(current),
    preflight: async (_mode, allowed) => { assert(allowed.includes(disk.mode)); event("preflight"); },
    saveJournal: async (journal) => { disk.journal = modeJournal(JSON.parse(JSON.stringify(journal))); event(`journal:${journal.phase}`); },
    stop: async () => { disk.running = false; event("stop"); },
    epochs: async () => { assert(!disk.running); event("epochs"); return { nyc: epoch, lax: epoch, sha: epoch }; },
    replacementsRunning: async (mode) => disk.mode === mode && disk.running,
    pin: async (mode) => { current.mode = mode; event("pin"); },
    replaceContainers: async (mode) => { assert(!disk.running); disk.mode = mode; event("replace"); },
    start: async () => { disk.running = true; epoch++; event("start"); },
    healthy: async (floor) => { assert(disk.running); if (floor) assert(Object.values(floor).every((value) => epoch > value));
      assert(disk.mode !== "linux-ipv4-tun" || packetReady, "adjacency is healthy but TUN is not ready"); event("healthy"); },
    finish: async () => { disk.journal = null; event("finish"); },
  };
  return { current, disk, events, actions, crash: (point: string) => { crash = point; }, packetReady: (value: boolean) => { packetReady = value; } };
}

test("opt-in mode pins intent before stop and keeps every immutable byte and plan; repeated enable is read-only", async () => {
  const f = fixture();
  await changeMode("linux-ipv4-tun", null, f.actions);
  assert(f.events.indexOf("journal:prepared") < f.events.indexOf("stop"));
  assert.deepEqual(f.current, { ...original, mode: "linux-ipv4-tun" });
  assert.equal(f.disk.journal, null);
  f.events.length = 0;
  await changeMode("linux-ipv4-tun", null, f.actions);
  assert.deepEqual(f.events, ["preflight", "healthy"]);
  await changeMode("neighbors", null, f.actions);
  assert.deepEqual(f.current, original);
});

test("mode recovery succeeds after every durable phase, pin and container boundary", async () => {
  for (const target of ["neighbors", "linux-ipv4-tun"] as const) for (const point of ["journal:prepared", "stop", "epochs", "journal:restarting", "pin", "replace", "journal:verifying", "start", "healthy", "journal:complete", "finish"]) {
    const initial = target === "neighbors" ? "linux-ipv4-tun" : "neighbors";
    const f = fixture(initial); f.crash(point);
    await assert.rejects(changeMode(target, null, f.actions), /simulated crash/);
    assert.deepEqual({ ...f.current, mode: "neighbors" }, original, point);
    await changeMode(target, f.disk.journal, f.actions);
    assert.equal(f.current.mode, target, point); assert.equal(f.disk.mode, target, point);
    assert(f.disk.running); assert.equal(f.disk.journal, null);
    assert.deepEqual({ ...f.current, mode: "neighbors" }, original, point);
  }
});

test("enable never mistakes adjacency for TUN readiness and retry keeps a running cohort's epoch fence", async () => {
  const f = fixture(); f.packetReady(false);
  await assert.rejects(changeMode("linux-ipv4-tun", null, f.actions), /TUN/);
  assert.equal(f.disk.journal?.phase, "verifying");
  assert.deepEqual(f.disk.journal?.epoch_floor, { nyc: 7, lax: 7, sha: 7 });
  await assert.rejects(changeMode("linux-ipv4-tun", f.disk.journal, f.actions), /TUN/);
  assert.equal(f.events.filter((event) => event === "epochs").length, 1);
  f.packetReady(true);
  await changeMode("linux-ipv4-tun", f.disk.journal, f.actions);
  assert.equal(f.events.filter((event) => event === "start").length, 1);
});

test("dead or partially started mode cohorts restart together under a NEW post-stop floor", async () => {
  const f = fixture(); f.crash("start");
  await assert.rejects(changeMode("linux-ipv4-tun", null, f.actions));
  f.disk.running = false; f.packetReady(false);
  await assert.rejects(changeMode("linux-ipv4-tun", f.disk.journal, f.actions), /TUN/);
  assert.deepEqual(f.disk.journal?.epoch_floor, { nyc: 8, lax: 8, sha: 8 });
  assert.equal(f.events.filter((event) => event === "stop").length, 2);
  f.packetReady(true);
  await changeMode("linux-ipv4-tun", f.disk.journal, f.actions);
});

test("disable is explicit recovery of a pending enable, retaining the previous container mode for removal", async () => {
  const f = fixture(); f.packetReady(false);
  await assert.rejects(changeMode("linux-ipv4-tun", null, f.actions), /TUN/);
  f.crash("pin");
  await assert.rejects(changeMode("neighbors", f.disk.journal, f.actions), /simulated/);
  assert.equal(f.disk.journal?.source_mode, "linux-ipv4-tun");
  assert.equal(f.disk.journal?.target_mode, "neighbors");
  assert.equal(f.disk.mode, "linux-ipv4-tun");
  await changeMode("neighbors", f.disk.journal, f.actions);
  assert.deepEqual(f.current, original);
});

test("ordinary start explicitly restarts the persisted mode, including TUN, without enrollment actions", async () => {
  const f = fixture("linux-ipv4-tun");
  await changeMode(f.current.mode, null, f.actions, true);
  assert.equal(f.disk.mode, "linux-ipv4-tun");
  assert(f.events.includes("start")); assert(f.events.includes("epochs"));
  assert.deepEqual({ ...f.current, mode: "neighbors" }, original);
});

test("original credential/key/profile/image/binding and concurrent plan changes reject before further mutation", async () => {
  for (const change of [
    (snapshot: ModeSnapshot) => { snapshot.image_id = `sha256:${"9".repeat(64)}`; },
    (snapshot: ModeSnapshot) => { snapshot.binding = "9".repeat(64); },
    (snapshot: ModeSnapshot) => { snapshot.plan_revision++; },
    (snapshot: ModeSnapshot) => { snapshot.plan_digest = "9".repeat(64); },
    ...IMMUTABLE_FILES.map((file) => (snapshot: ModeSnapshot) => { snapshot.files[file] = "9".repeat(64); }),
  ]) {
    const f = fixture(); f.crash("journal:prepared");
    await assert.rejects(changeMode("linux-ipv4-tun", null, f.actions));
    change(f.current); f.events.length = 0;
    await assert.rejects(changeMode("linux-ipv4-tun", f.disk.journal, f.actions), /changed/);
    assert.deepEqual(f.events, []);
  }
  const f = fixture();
  f.actions.preflight = async () => { f.current.plan_revision++; };
  await assert.rejects(changeMode("linux-ipv4-tun", null, f.actions), /changed/);
  assert.equal(f.disk.journal, null); assert(f.disk.running);
  assert.deepEqual(f.current.files, original.files);
});

test("mode journal is closed, requires explicit supported modes, exact epoch roles and immutable snapshots", async () => {
  const f = fixture(); f.crash("journal:prepared");
  await assert.rejects(changeMode("linux-ipv4-tun", null, f.actions));
  const journal = f.disk.journal; assert(journal);
  for (const changed of [{ ...journal, version: 2 }, { ...journal, target_mode: "auto" }, { ...journal, source_mode: null },
    { ...journal, kind: "image" }, { ...journal, privileged: true }, { ...journal, phase: "verifying" },
    { ...journal, phase: "verifying", epoch_floor: { nyc: 1, lax: 1, sha: -1 } }, { ...journal, original: { ...journal.original, mode: null } }])
    assert.throws(() => modeJournal(changed));
  assert.equal(IMMUTABLE_FILES.length, 12);
  assert(IMMUTABLE_FILES.every((file) => !/cache|control\.sock|config\.json|connections\.json/.test(file)));
});

test("strict metadata keeps legacy mode absent until explicit persistence and rejects unknown opt-in values", () => {
  const value = { version: 1, run_id: "1".repeat(32), fingerprint: "2".repeat(64), controller: "http://localhost:8420",
    agent_controller: "http://host.docker.internal:8420", uid: 501, gid: 20, image_id: image, host_gateway: false, phase: "preparing",
    nodes: Object.fromEntries(["nyc", "lax", "sha"].map((role) => [role, { phase: "empty", id: null, public_key: null }])), files: {} };
  assert.deepEqual(metadata(value), value);
  assert.equal(demoMode(metadata(value).mode), "neighbors");
  assert.equal(metadata({ ...value, mode: "linux-ipv4-tun" }).mode, "linux-ipv4-tun");
  assert.equal(shellMode(metadata(value).shell), "readonly");
  assert.equal(metadata({ ...value, shell: "development" }).shell, "development");
  for (const shell of [null, true, "writable", "auto", {}]) assert.throws(() => metadata({ ...value, shell }));
  for (const mode of [null, true, "tun", "auto", {}, undefined]) assert.throws(() => metadata({ ...value, mode }));
  assert.throws(() => metadata({ ...value, tun_mtu: 1400 }));
});

test("development containers are explicitly writable with bounded Docker defaults while PID 1 drops to the recorded UID", () => {
  const f = container("neighbors");
  const value: Record<string, unknown> = structuredClone(f.value);
  const config = value.Config as Record<string, unknown>; const host = value.HostConfig as Record<string, unknown>;
  config.User = "0:0"; config.Entrypoint = ["setpriv"];
  config.Cmd = [...bootstrapArguments("501:20", false), "meshway-agent", ...agentArguments("neighbors")];
  config.Env = ["PATH=/usr/local/bin:/usr/bin", "MESHWAY_AGENT_UID=501", "MESHWAY_AGENT_GID=20"];
  host.ReadonlyRootfs = false; host.CapDrop = null; host.CapAdd = ["NET_ADMIN"]; host.Init = false;
  const check = (candidate: Record<string, unknown>) => assertDemoContainer(candidate,
    { run_id: "a".repeat(32), fingerprint: "b".repeat(64) }, "nyc", [image], "/private/demo/nyc", "501:20", ["neighbors"], ["development"]);
  check(value);
  config.Env = ["MESHWAY_AGENT_GID=20", "PATH=/usr/local/bin:/usr/bin", "MESHWAY_AGENT_UID=501"];
  check(value);
  for (const mutate of [
    (item: Record<string, unknown>) => { (item.HostConfig as Record<string, unknown>).Privileged = true; },
    (item: Record<string, unknown>) => { (item.HostConfig as Record<string, unknown>).CapAdd = ["NET_ADMIN", "SYS_ADMIN"]; },
    (item: Record<string, unknown>) => { (item.HostConfig as Record<string, unknown>).CapDrop = ["ALL"]; },
    (item: Record<string, unknown>) => { (item.HostConfig as Record<string, unknown>).ReadonlyRootfs = true; },
    (item: Record<string, unknown>) => { (item.Config as Record<string, unknown>).User = "501:20"; },
    (item: Record<string, unknown>) => { (item.Config as Record<string, unknown>).Env = ["MESHWAY_AGENT_UID=0", "MESHWAY_AGENT_GID=0"]; },
    (item: Record<string, unknown>) => { (item.Config as Record<string, unknown>).Env = ["MESHWAY_AGENT_UID=501", "MESHWAY_AGENT_UID=501", "MESHWAY_AGENT_GID=20"]; },
  ]) { const changed = structuredClone(value); mutate(changed); assert.throws(() => check(changed)); }
});

function container(mode: DemoMode) {
  const packet = mode === "linux-ipv4-tun";
  const owner = { run_id: "a".repeat(32), fingerprint: "b".repeat(64) };
  const value = { Image: image, Config: { User: packet ? "0:0" : "501:20", Entrypoint: [packet ? "setpriv" : "meshway-agent"],
    Cmd: [...(packet ? [...bootstrapArguments("501:20"), "meshway-agent"] : []), ...agentArguments(mode)],
    Labels: { "io.meshway.demo.project": "managed-neighbors", "io.meshway.demo.run": owner.run_id, "io.meshway.demo.data": owner.fingerprint, "io.meshway.demo.resource": "nyc" } },
  HostConfig: { ReadonlyRootfs: true, Privileged: false, CapDrop: ["ALL"], CapAdd: packet ? [...BOOTSTRAP_CAPS] : [], Init: !packet,
    SecurityOpt: ["no-new-privileges"], NetworkMode: "meshway-demo-mgmt", Devices: packet ? [{ PathOnHost: "/dev/net/tun", PathInContainer: "/dev/net/tun", CgroupPermissions: "rw" }] : [],
    Sysctls: { "net.ipv4.ip_forward": "0", "net.ipv6.conf.all.forwarding": "0", "net.ipv4.ping_group_range": "0 2147483647", ...(packet ? { "net.ipv6.conf.default.disable_ipv6": "1" } : {}) },
    LogConfig: packet ? { Type: "local", Config: { "max-size": "1m", "max-file": "1", compress: "false" } } : { Type: "none", Config: {} } },
  Mounts: [{ Type: "bind", Source: "/private/demo/nyc", Destination: "/state", RW: true }] };
  return { value, check: (value: Record<string, unknown>, modes: DemoMode[] = [mode]) => assertDemoContainer(value, owner, "nyc", [image], "/private/demo/nyc", "501:20", modes) };
}

test("ordinary neighbors retain zero capabilities/devices; packet containers require EXACT non-root bootstrap and device", () => {
  for (const mode of ["neighbors", "linux-ipv4-tun"] as const) {
    const f = container(mode); f.check(f.value);
    for (const change of [
      (value: typeof f.value) => { value.HostConfig.CapAdd.push("NET_RAW"); },
      (value: typeof f.value) => { value.Config.Cmd = ["sleep", "infinity"]; },
      (value: typeof f.value) => { value.HostConfig.SecurityOpt = []; },
      (value: typeof f.value) => { value.HostConfig.SecurityOpt.push("seccomp=unconfined"); },
      (value: typeof f.value) => { value.HostConfig.NetworkMode = "host"; },
      (value: typeof f.value) => { value.HostConfig.ReadonlyRootfs = false; },
      (value: typeof f.value) => { value.HostConfig.Devices.push({ PathOnHost: "/dev/kvm", PathInContainer: "/dev/kvm", CgroupPermissions: "rwm" }); },
      (value: typeof f.value) => { value.Mounts.push({ Type: "bind", Source: "/", Destination: "/host", RW: true }); },
    ]) { const value = structuredClone(f.value); change(value); assert.throws(() => f.check(value)); }
  }
  const f = container("linux-ipv4-tun");
  assert.throws(() => f.check(f.value, ["neighbors"]));
  for (const change of [
    (value: typeof f.value) => { value.Config.Cmd[value.Config.Cmd.indexOf("501")] = "0"; },
    (value: typeof f.value) => { value.HostConfig.Init = true; },
    (value: typeof f.value) => { value.HostConfig.Devices = []; },
    (value: typeof f.value) => { value.HostConfig.CapAdd = ["NET_ADMIN"]; },
  ]) { const value = structuredClone(f.value); change(value); assert.throws(() => f.check(value)); }
});

const identity = { node_id: "a".repeat(64), ipv4: "100.96.0.2", ipv4_prefix: "100.96.0.0/16" };

test("real IPv4 kernel evidence requires own /32, MTU/up TUN, owned prefix route and unreachable guard", () => {
  const addresses = [{ ifname: "meshway0", mtu: 1000, flags: ["POINTOPOINT", "UP"], addr_info: [{ family: "inet", local: identity.ipv4, prefixlen: 32 }] }];
  const routes = [{ dst: identity.ipv4_prefix, dev: "meshway0", prefsrc: identity.ipv4, metric: 42759, protocol: "242" },
    { dst: identity.ipv4_prefix, type: "unreachable", metric: 42760, protocol: "242" }];
  assert(packetInterface(addresses, routes, identity));
  assert(packetInterface(addresses, [{ ...routes[0], protocol: "242", scope: "253" }, { ...routes[1], type: "7" }], identity), "ip -N uses numeric RTN_UNREACHABLE as well as protocol IDs");
  assert.equal(packetInterface([], routes, identity), false);
  assert.equal(packetInterface(addresses, routes.slice(0, 1), identity), false);
  assert.equal(packetInterface(addresses, [...routes, { dst: "100.96.0.4/32", dev: "eth0" }], identity), false);
  for (const addr of [{ ...addresses[0], mtu: 1400 }, { ...addresses[0], flags: [] }, { ...addresses[0], ifname: "eth0" },
    { ...addresses[0], addr_info: [{ family: "inet", local: identity.ipv4, prefixlen: 16 }] }])
    assert.equal(packetInterface([addr], routes, identity), false);
  assert.equal(packetInterface(addresses, [{ ...routes[0], protocol: "kernel" }, routes[1]], identity), false);
});
