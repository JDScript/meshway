import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { fstatSync } from "node:fs";
import { chmod, lstat, mkdtemp, readFile, realpath, writeFile } from "node:fs/promises";
import { arch } from "node:os";
import { dirname, join, relative, resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import { ControlApiError, createEnrollmentToken, getConnections, getConnectionRuntimes, getEndpointDiscoveries,
  getNetwork, listNodes, revokeNode, saveConnections } from "../../packages/control-client/src/index.ts";
import { enrollAgentOperation, getAgentConnectionsOperation } from "../../packages/control-client/src/generated/operations.gen.ts";
import type { ConnectionPlan, NodeEndpointDiscovery, NodeSummary } from "../../packages/control-client/src/index.ts";
import { array, boolean, command, parse, record, text } from "../managed-connections/local.ts";
import { carrierProof, currentMapped, endpoint, ledger, liveStatus, local, noPhantomRenewal, ROLES, successfulPing } from "./checks.ts";
import type { Ledger, Local, Role } from "./checks.ts";

const HERE = dirname(fileURLToPath(import.meta.url));
const WORKSPACE = resolve(HERE, "../..");
const TEMP = "/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode";
const LOCK = join(TEMP, "meshway-performance.lock");
const OWNER = "io.meshway.managed-discovery.run";
const LEASE = 10;
type ContainerRole = Role | "controller" | "observer";
type Report = { schema_version: 1; run_id: string; status: "RUNNING" | "PASS" | "FAIL";
  started_at: string; finished_at?: string; stage: string; check?: string; error_category?: string;
  image?: { reference: string; id: string; architecture: string }; network_id?: string;
  runtime_ids: { role: ContainerRole; id: string; ip: string }[];
  nodes: { role: Role; id: string; name: string; ip: string }[];
  evidence: { stage: string; at: string; status: "RUNNING" | "PASS" | "FAIL"; checks: string[]; samples: object[] }[];
  cleanup: { status: "PENDING" | "PASS" | "FAIL"; errors: string[] } };

function options(args: string[]) {
  if (args.includes("--help")) {
    console.log("Usage: python3 experiments/managed-discovery/run.py --image meshway-manageddiscovery:TAG [--artifacts-dir EXISTING_OPENCODE_PARENT]\nMain builds/runs Docker. Four fresh isolated containers, no TUN or host networking. Caller holding the shared lock may pass MESHWAY_DISCOVERY_LOCK_FD.");
    process.exit(0);
  }
  let image: string | undefined; let parent = TEMP; const seen = new Set<string>();
  for (let index = 0; index < args.length; index++) {
    const flag = args[index]; assert(flag && !seen.has(flag)); seen.add(flag);
    const value = args[++index]; assert(value && !value.startsWith("--"));
    if (flag === "--image") { assert(/^meshway-manageddiscovery:[A-Za-z0-9_.-]+$/.test(value)); image = value; }
    else if (flag === "--artifacts-dir") parent = resolve(value);
    else throw new Error("unknown option");
  }
  assert(image, "dedicated image required"); return { image, parent };
}
function architecture(value: string) {
  const aliases: Record<string, string> = { arm64: "arm64", aarch64: "arm64", amd64: "amd64", x86_64: "amd64", x64: "amd64" };
  const result = aliases[value.trim()]; assert(result); return result;
}

async function main() {
  const opts = options(process.argv.slice(2)); process.umask(0o077);
  assert.equal(Number(process.versions.node.split(".")[0]), 24);
  const fd = Number(process.env.MESHWAY_DISCOVERY_LOCK_FD); assert(Number.isSafeInteger(fd) && fd >= 3);
  assert.equal(process.env.MESHWAY_PERF_LOCK, LOCK);
  const held = fstatSync(fd); const disk = await lstat(LOCK);
  assert(held.isFile() && disk.isFile() && held.ino === disk.ino && held.dev === disk.dev
    && held.uid === process.getuid?.() && !(held.mode & 0o077));
  const parent = await realpath(opts.parent); const approved = await realpath(TEMP);
  const relativeParent = relative(approved, parent); assert(!relativeParent.startsWith("..") && !relativeParent.startsWith("/"));
  assert((await lstat(parent)).isDirectory());
  const runId = `${Date.now().toString(36)}-${randomBytes(8).toString("hex")}`;
  const prefix = `meshway-discovery-${runId}`; const networkName = `${prefix}-bridge`;
  const artifacts = await mkdtemp(join(parent, `${prefix}-`)); await chmod(artifacts, 0o700);
  const names: Record<ContainerRole, string> = { controller: `${prefix}-controller`, observer: `${prefix}-observer`,
    one: `${prefix}-one`, two: `${prefix}-two` };
  const containers: string[] = []; const networks: string[] = [];
  const inventory = new Map<Role, NodeSummary>(); const credentials = new Map<Role, string>();
  const addresses = new Map<ContainerRole, string>();
  let interrupted = false; let cleaning = false;
  const signal = () => { interrupted = true; }; process.on("SIGINT", signal); process.on("SIGTERM", signal);
  const report: Report = { schema_version: 1, run_id: runId, status: "RUNNING", started_at: new Date().toISOString(),
    stage: "preflight", runtime_ids: [], nodes: [], evidence: [], cleanup: { status: "PENDING", errors: [] } };
  let current: Report["evidence"][number] | undefined;
  const save = () => writeFile(join(artifacts, "result.json"), `${JSON.stringify(report, null, 2)}\n`, { mode: 0o600 });
  const stage = (name: string) => {
    if (current) current.status = "PASS";
    report.stage = name; delete report.check;
    current = { stage: name, at: new Date().toISOString(), status: "RUNNING", checks: [], samples: [] };
    report.evidence.push(current); console.log(`[managed-discovery] ${name}`);
  };
  const check = (name: string) => { report.check = name; };
  const passed = (name: string) => { assert(current); current.checks.push(name); };
  const sample = (value: object) => { assert(current); current.samples.push(value); };
  const docker = (args: string[], input?: string, timeout = 30_000) => {
    assert(!interrupted || cleaning, "interrupted");
    return command("docker", args, { cwd: WORKSPACE, timeout, ...(input === undefined ? {} : { input }) });
  };
  const helper = async (role: ContainerRole, action: string, data: object = {}) =>
    parse(await docker(["exec", "-i", names[role], "python3", "/private/container.py", action], JSON.stringify(data)));
  const until = async <T>(name: string, test: () => Promise<T | false>, timeout = 45_000): Promise<T> => {
    check(name); const deadline = Date.now() + timeout;
    do { assert(!interrupted); const value = await test(); if (value !== false) return value; await delay(400); } while (Date.now() < deadline);
    throw new Error("readiness deadline");
  };
  const node = (role: Role) => { const value = inventory.get(role); assert(value); return value; };
  const credential = (role: Role) => { const value = credentials.get(role); assert(value); return value; };
  const ip = (role: ContainerRole) => { const value = addresses.get(role); assert(value); return value; };
  const processState = async (role: Role) => { const value = local(await helper(role, "observe"));
    assert.equal(value.identity, node(role).id); return value; };
  const observerState = async () => { const value = ledger(await helper("observer", "observer-state"));
    assert(value.peers.every((peer) => ROLES.some((role) => peer.ip === ip(role)))); return value; };
  const install = async (role: ContainerRole, filename: string, source: string) => {
    assert(/^[a-z_]+\.py$/.test(filename));
    await docker(["exec", "-i", names[role], "python3", "-c",
      `import os,sys; os.umask(0o077); open('/private/${filename}','x').write(sys.stdin.read())`], source);
  };
  try {
    stage("native-image-and-four-owned-containers");
    check("native engine and immutable full image ID");
    const engine = architecture(await docker(["info", "--format", "{{.Architecture}}"])); assert.equal(engine, architecture(arch()));
    assert.equal((await docker(["info", "--format", "{{.OSType}}"])).trim(), "linux");
    assert.equal((await docker(["image", "inspect", opts.image, "--format", "{{.Os}}"])).trim(), "linux");
    assert.equal(architecture(await docker(["image", "inspect", opts.image, "--format", "{{.Architecture}}"])), engine);
    const imageId = (await docker(["image", "inspect", opts.image, "--format", "{{.Id}}"])).trim();
    assert(/^sha256:[0-9a-f]{64}$/.test(imageId)); report.image = { reference: opts.image, id: imageId, architecture: engine };
    await docker(["network", "create", "--driver", "bridge", "--label", `${OWNER}=${runId}`, networkName]); networks.push(networkName);
    for (const role of ["controller", "observer", ...ROLES] as const) {
      check(`${role}: cap-drop ALL / readonly / private tmpfs / no mounts or host namespaces`);
      const id = (await docker(["create", "--init", "--name", names[role], "--hostname", role,
        "--label", `${OWNER}=${runId}`, "--label", `io.meshway.managed-discovery.role=${role}`,
        "--user", "0:0", "--network", networkName,
        ...(role === "controller" ? ["--publish", "127.0.0.1::8420"] : []),
        "--cap-drop", "ALL", "--security-opt", "no-new-privileges", "--read-only",
        "--tmpfs", "/private:rw,nosuid,nodev,noexec,mode=0700,size=96m",
        "--sysctl", "net.ipv4.ip_forward=0", "--sysctl", "net.ipv6.conf.all.forwarding=0",
        "--env", "PYTHONDONTWRITEBYTECODE=1", "--cpus", "1", "--memory", "256m", "--memory-swap", "256m", "--pids-limit", "96",
        "--entrypoint", "sleep", imageId, "infinity"])).trim();
      assert(/^[0-9a-f]{64}$/.test(id)); containers.push(names[role]); await docker(["start", names[role]]);
      // Read shared source on the host, install via stdin into owned tmpfs.
      for (const [filename, path] of [["base.py", "../managed-dns/container.py"], ["wire.py", "../managed-dns/wire.py"],
        ["stun_wire.py", "../netcheck/wire.py"], ["container.py", "container.py"], ["observer.py", "observer.py"]]) {
        assert(filename && path); await install(role, filename, await readFile(resolve(HERE, path), "utf8"));
      }
      const inspected = parse(await docker(["inspect", names[role], "--format", "{{json .}}"]));
      const host = record(inspected.HostConfig); assert.equal(host.Privileged, false); assert.equal(host.ReadonlyRootfs, true);
      assert.equal(host.NetworkMode, networkName); assert(host.PidMode !== "host" && host.IpcMode !== "host");
      assert.deepEqual(array(host.CapDrop), ["ALL"]); assert.deepEqual(array(host.CapAdd ?? []), []);
      assert(!array(inspected.Mounts).some((mount) => record(mount).Type === "bind"));
      const attached = record(record(inspected.NetworkSettings).Networks); assert.deepEqual(Object.keys(attached), [networkName]);
      const address = text(record(attached[networkName]).IPAddress); endpoint(`${address}:3478`); addresses.set(role, address);
      const ports = record(host.PortBindings); assert.deepEqual(Object.keys(ports), role === "controller" ? ["8420/tcp"] : []);
      if (role === "controller") assert.equal(record(array(ports["8420/tcp"])[0]).HostIp, "127.0.0.1");
      assert(boolean((await helper(role, "tools")).ready)); report.runtime_ids.push({ role, id, ip: address });
    }
    passed("exactly-four-isolated-capless-containers-no-host-mounts-no-agent-or-observer-published-ports");

    stage("fresh-identity-default-discovery-disabled");
    await helper("controller", "controller-start");
    const port = (await docker(["port", names.controller, "8420/tcp"])).trim(); assert(/^127\.0\.0\.1:\d+$/.test(port));
    const baseUrl = `http://${port}`;
    const admin = await until("fresh Controller bootstrap", async () => { try { return text((await helper("controller", "admin-token")).token); } catch { return false; } });
    const initial = await until("generated network readiness", async () => { try { return await getNetwork(baseUrl, admin); } catch { return false; } });
    assert.equal((await listNodes(baseUrl, admin)).nodes.length, 0); assert.equal(initial.dns.enabled, false);
    report.network_id = initial.network_id;
    for (const role of ROLES) {
      const name = role === "one" ? "Twin" : "Solo";
      const token = await createEnrollmentToken(baseUrl, admin, { expires_in_seconds: 300 });
      const prepared = await helper(role, "prepare-enrollment", { name, token: token.token, controller_address: ip("controller") });
      const secret = text(prepared.credential); credentials.set(role, secret);
      const enrolled = await enrollAgentOperation(baseUrl, token.token, { name, node_credential: secret, agent_version: text(prepared.version) });
      assert.equal((await helper(role, "complete-enrollment", { node_id: enrolled.node.id })).node_id, enrolled.node.id);
      inventory.set(role, enrolled.node); report.nodes.push({ role, name, id: enrolled.node.id, ip: ip(role) });
      await helper(role, "agent-start", { dns: true });
    }
    await until("both real Agent connection keys published", async () => {
      const value = await getConnections(baseUrl, admin);
      return ROLES.every((role) => value.nodes.some((entry) => entry.node.id === node(role).id && entry.public_key !== null)) ? value : false;
    });
    const put = async (configuration: ConnectionPlan) => until("generated ADMIN connections CAS", async () => {
      const baseline = await getConnections(baseUrl, admin);
      try { const saved = await saveConnections(baseUrl, admin, { expected_revision: baseline.revision, configuration }); assert(saved.valid); return saved; }
      catch (error) { if (error instanceof ControlApiError && error.status === 409) return false; throw error; }
    });
    const configuration: ConnectionPlan = {
      listeners: ROLES.map((role) => ({ id: `${role}-udp`, node_id: node(role).id, transport: "native_udp", bind: "0.0.0.0:41000",
        advertised_endpoints: [{ host: ip(role), port: 41000 }], credential_ref: null })),
      candidates: [{ id: "one-to-two", dialer_node_id: node("one").id, listener_id: "two-udp",
        endpoint: { host: ip("two"), port: 41000 }, priority: 0, credential_ref: null }],
    };
    const configured = await put(configuration); sample({ configuration_revision: configured.revision, configuration });
    const watermarks = new Map<string, { epoch: number; sequence: number }>();
    const discovery = async () => { const value = await getEndpointDiscoveries(baseUrl, admin);
      assert.equal(value.network_id, initial.network_id);
      assert.equal(new Set(value.nodes.map((item) => item.node_id)).size, value.nodes.length);
      for (const item of value.nodes) {
        assert(ROLES.some((role) => node(role).id === item.node_id));
        if (item.reported_at === null) continue; // Empty Controller startup cache is not a report.
        const previous = watermarks.get(item.node_id);
        if (previous) {
          assert(item.epoch >= previous.epoch);
          if (item.epoch === previous.epoch) assert(item.sequence >= previous.sequence);
        }
        watermarks.set(item.node_id, { epoch: item.epoch, sequence: item.sequence });
      }
      return value; };
    const entry = (values: Awaited<ReturnType<typeof discovery>>, role: Role) => {
      const value = values.nodes.find((item) => item.node_id === node(role).id); assert(value); return value;
    };
    const ping = async (role: Role) => {
      const peer = role === "one" ? "two" : "one";
      const proof = successfulPing(await helper(role, "ping", { node_id: node(peer).id }), node(role).id, node(peer).id);
      sample({ role, nonce_round_trip: proof }); return proof;
    };
    const adjacency = async () => until("both fresh authenticated NativeUDP runtime probes and actual CLI nonce", async () => {
      const values = await getConnectionRuntimes(baseUrl, admin);
      const ready = ROLES.every((role) => { const value = values.nodes.find((item) => item.node_id === node(role).id);
        const peer = node(role === "one" ? "two" : "one").id;
        return value?.current && !value.stale && value.state === "applied"
          && value.peers.some((item) => item.peer_node_id === peer && item.transport === "native_udp" && item.state === "authenticated" && item.probe_fresh); });
      if (!ready) return false;
      try { for (const role of ROLES) await ping(role); } catch { return false; }
      sample({ connection_runtimes: values }); return values;
    });
    await adjacency();
    const disabled = await until("default no STUN gives disabled empty discovery", async () => {
      const value = await discovery(); return ROLES.every((role) => { const item = entry(value, role);
        return item.current && !item.stale && item.state === "disabled" && item.sockets.length === 0 && item.epoch > 0; }) ? value : false;
    });
    assert.equal((await observerState()).count, 0); sample({ discovery: disabled });
    passed("default-disabled-no-fabricated-observations-while-noise-is-authenticated");
    const original = new Map<Role, Local>(); for (const role of ROLES) original.set(role, await processState(role));
    const sameIdentity = async (role: Role, samePid = true) => {
      const before = original.get(role); assert(before); const value = await processState(role);
      assert.equal(value.identity_digest, before.identity_digest); assert.equal(value.key_digest, before.key_digest);
      if (samePid) assert.equal(value.pid, before.pid);
      return value;
    };
    const observerConfig = { bind: ip("observer"), allowed: ROLES.map(ip) }; const observerEndpoint = `${ip("observer")}:3478`;
    await helper("observer", "observer-start", observerConfig);
    for (const role of ROLES) {
      await helper(role, "agent-stop"); await helper(role, "agent-start", { observer: ip("observer"), dns: true });
      const value = await until(`${role}: restart preserves credential/key bytes`, async () => {
        const current = await sameIdentity(role, false); return current.pid !== null ? current : false;
      }); original.set(role, value);
    }

    const aligned = async (requireCurrent = true) => {
      const values = await discovery(); const runtimes = await getConnectionRuntimes(baseUrl, admin);
      for (const role of ROLES) {
        const value = entry(values, role); const projection = await getAgentConnectionsOperation(baseUrl, credential(role));
        const runtime = runtimes.nodes.find((item) => item.node_id === node(role).id); assert(runtime);
        assert(runtime.current && !runtime.stale);
        if (requireCurrent) assert(value.current && !value.stale);
        assert.equal(value.epoch, runtime.epoch); assert.equal(value.revision, projection.payload.revision);
        assert.equal(value.digest, projection.digest); assert.equal(value.revision, runtime.revision); assert.equal(value.digest, runtime.digest);
      }
      return values;
    };
    const mapped = async (label: string) => until(label, async () => {
      try {
        const values = await aligned(); const observed = await observerState();
        for (const role of ROLES) {
          const value = entry(values, role); const process = await sameIdentity(role);
          assert(value.sockets.length > 0); carrierProof(value, process, observerEndpoint, ip(role), observed);
        }
        return values;
      } catch { return false; }
    });

    stage("same-carrier-stun-and-noise-coexistence");
    const active = await mapped("all real managed carriers have mapped recvfrom evidence");
    for (const role of ROLES) assert(entry(active, role).epoch > entry(disabled, role).epoch);
    assert.deepEqual(entry(active, "one").sockets.map((socket) => socket.resource_kind).sort(), ["listener", "outgoing_candidate"]);
    assert.deepEqual(entry(active, "two").sockets.map((socket) => socket.resource_kind), ["listener"]);
    assert.deepEqual(entry(active, "one").sockets.map((socket) => socket.resource_id).sort(), ["one-to-two", "one-udp"]);
    assert.equal(entry(active, "two").sockets[0]?.resource_id, "two-udp");
    await adjacency(); sample({ discovery: active, observer: await observerState() });
    for (const role of ROLES) { const state = await sameIdentity(role); sample({ role, pid: state.pid, sockets: state.udp,
      live: liveStatus(await helper(role, "live"), node(role).id, entry(active, role).revision ?? 0) }); }
    passed("listener-and-outgoing-distinct-sockets-exact-fd-inventory-real-observer-peer-no-extra-probe-socket");
    passed("same-daemon-dns-listener-and-noise-cli-authenticated-nonce-round-trips"); await save();

    stage("observer-loss-no-http-ttl-renewal-noise-remains");
    await helper("observer", "observer-stop");
    const lossAt = Date.now();
    const oldObserved = Math.max(...ROLES.flatMap((role) => currentMapped(entry(active, role)).map((item) => Date.parse(item.observed_at))));
    assert(Number.isFinite(oldObserved));
    // Flush an already-received STUN reply and its next HTTP publication before
    // comparing stable peer counters. An in-flight real reply is not renewal fraud.
    await delay(4000);
    const stoppedLedger = await observerState(); const lastMapped = new Map<string, { observation: NodeEndpointDiscovery["sockets"][number]["observations"][number]; peer: Ledger["peers"][number] }>();
    const remember = (values: Awaited<ReturnType<typeof discovery>>, observed: Ledger) => {
      for (const role of ROLES) for (const socket of entry(values, role).sockets) for (const observation of socket.observations) {
        if (observation.outcome !== "mapped" || observation.mapped_endpoint === null) continue;
        const target = endpoint(observation.mapped_endpoint); const peer = observed.peers.find((item) => item.ip === target.ip && item.port === target.port); assert(peer);
        const key = `${role}:${socket.resource_id}:${socket.generation}:${socket.socket_id}:${observation.observer}`;
        const before = lastMapped.get(key); if (before) noPhantomRenewal(before.observation, observation, before.peer, peer);
        lastMapped.set(key, { observation, peer });
      }
    };
    const stopped = await discovery(); remember(stopped, stoppedLedger);
    // Observe every report window for the full DTO TTL, with real nonce probes in parallel.
    let expired = false;
    while (Date.now() - lossAt < 34_000) {
      assert(!interrupted); const values = await aligned(false); const observed = await observerState(); assert.equal(observed.count, stoppedLedger.count);
      remember(values, observed); await ping("one"); await ping("two");
      for (const role of ROLES) await sameIdentity(role);
      if (ROLES.every((role) => currentMapped(entry(values, role)).length === 0)) expired = true;
      sample({ elapsed_ms: Date.now() - lossAt, discovery: values, observer_count: observed.count }); await delay(1000);
    }
    assert(expired); const noFresh = await discovery(); assert(ROLES.every((role) => currentMapped(entry(noFresh, role)).length === 0));
    const unchanged = await getConnections(baseUrl, admin); assert.equal(unchanged.revision, configured.revision); assert.deepEqual(unchanged.configuration, configuration);
    await adjacency(); passed("observer-loss-expires-mapped-without-report-based-renewal-revision-or-noise-loss"); await save();

    stage("observer-restoration-requires-new-real-observation");
    await helper("observer", "observer-start", observerConfig);
    const restored = await mapped("new real response restores every carrier");
    for (const role of ROLES) for (const observation of currentMapped(entry(restored, role))) assert(Date.parse(observation.observed_at) > oldObserved);
    assert((await observerState()).count > stoppedLedger.count);
    assert.deepEqual((await getConnections(baseUrl, admin)).configuration, configuration);
    assert.equal((await getConnections(baseUrl, admin)).revision, configured.revision);
    await adjacency(); sample({ discovery: restored, observer: await observerState() });
    passed("new-observed-at-same-pids-identities-observation-never-auto-installs-candidate");

    stage("cas-listener-withdrawal-replaces-generation");
    const reduced: ConnectionPlan = { listeners: configuration.listeners.filter((listener) => listener.id !== "one-udp"), candidates: configuration.candidates };
    const withdrawn = await put(reduced); assert(withdrawn.revision > configured.revision);
    const beforeAssociations = new Set(entry(restored, "one").sockets.map((socket) => `${socket.generation}:${socket.socket_id}`));
    const replacement = await until("removed listener cleared and only fresh outgoing carrier remains", async () => {
      const values = await discovery(); const value = entry(values, "one");
      assert(!value.sockets.some((socket) => socket.resource_id === "one-udp"), "GET retained removed listener after CAS");
      if (!value.current || value.revision !== withdrawn.revision || value.sockets.length !== 1) return false;
      try { const all = await mapped("reobserved current generation"); return all; } catch { return false; }
    });
    for (const socket of entry(replacement, "one").sockets) assert(!beforeAssociations.has(`${socket.generation}:${socket.socket_id}`));
    const withdrawnLocal = await sameIdentity("one"); assert(!withdrawnLocal.udp.some((socket) => socket.port === 41000));
    await adjacency(); sample({ configuration_revision: withdrawn.revision, discovery: replacement,
      live: liveStatus(await helper("one", "live"), node("one").id, withdrawn.revision) });
    passed("removed-listener-closed-old-associations-gone-current-source-revision-truthful-outgoing-noise-survives"); await save();

    stage("controller-outage-lease-stops-carriers-and-stun");
    const outageBefore = await mapped("fresh discovery immediately before controller outage");
    await helper("controller", "controller-stop"); await delay((LEASE + 3) * 1000);
    for (const role of ROLES) {
      const state = await sameIdentity(role); assert.deepEqual(state.udp.map((socket) => socket.port), [5353]);
      assert.notEqual((await helper(role, "ping", { node_id: node(role === "one" ? "two" : "one").id })).exit, 0);
      sample({ role, pid: state.pid, sockets: state.udp });
    }
    const expiredLedger = await observerState(); await delay(3000); assert.equal((await observerState()).count, expiredLedger.count);
    passed("lease-expiry-closes-all-real-carriers-no-old-stun-sends-no-authenticated-neighbor");
    // Freeze expired Agents to make the Controller's empty in-memory startup view deterministic.
    for (const role of ROLES) await helper(role, "pause", { paused: true });
    await helper("controller", "controller-start");
    const recoveredNetwork = await until("same isolated DB recovery", async () => { try { return await getNetwork(baseUrl, admin); } catch { return false; } });
    assert.equal(recoveredNetwork.network_id, initial.network_id);
    const persisted = await getConnections(baseUrl, admin); assert.equal(persisted.revision, withdrawn.revision); assert.deepEqual(persisted.configuration, reduced);
    for (const role of ROLES) {
      const before = configured.nodes.find((value) => value.node.id === node(role).id);
      const after = persisted.nodes.find((value) => value.node.id === node(role).id);
      assert(before && after && before.public_key !== null); assert.equal(after.public_key, before.public_key);
    }
    assert.deepEqual((await listNodes(baseUrl, admin)).nodes.map((value) => value.id).sort(), report.nodes.map((value) => value.id).sort());
    const empty = await discovery(); assert(ROLES.every((role) => entry(empty, role).sockets.length === 0 && currentMapped(entry(empty, role)).length === 0));
    sample({ discovery_before_outage: outageBefore, paused_startup_discovery: empty, configuration_revision: persisted.revision });
    for (const role of ROLES) await helper(role, "pause", { paused: false });
    const recovery = await mapped("fresh Agent reports alone restore discovery after restart");
    for (const role of ROLES) { const before = entry(outageBefore, role); const after = entry(recovery, role);
      assert(after.epoch >= before.epoch); assert(after.reported_at !== null && before.reported_at !== null && Date.parse(after.reported_at) > Date.parse(before.reported_at));
      for (const observation of currentMapped(after)) assert(Date.parse(observation.observed_at) > Date.parse(before.reported_at));
      await sameIdentity(role); }
    await adjacency(); sample({ fresh_discovery: recovery, observer: await observerState() });
    passed("same-db-network-keys-config-retained-no-disk-report-resurrection-fresh-carrier-generation-and-report-required"); await save();

    stage("node-revocation-closes-carriers-clears-reports-and-peer");
    await revokeNode(baseUrl, admin, { node_id: node("two").id });
    await until("revoked two self-exits with all UDP and DNS sockets closed", async () => {
      const state = await processState("two"); return state.pid === null && state.udp.length === 0 && array((await helper("two", "observe")).listeners).length === 0 ? state : false;
    });
    const revoked = await until("revoked discovery absent or fully cleared", async () => {
      const values = await discovery(); const value = values.nodes.find((item) => item.node_id === node("two").id);
      return !value || (!value.current && value.sockets.length === 0) ? values : false;
    });
    await until("one current config withdraws revoked peer and outgoing socket", async () => {
      const projection = await getAgentConnectionsOperation(baseUrl, credential("one")); const state = await sameIdentity("one");
      return projection.payload.outgoing.length === 0 && !projection.payload.peers.some((peer) => peer.node_id === node("two").id)
        && state.udp.every((socket) => socket.port === 5353) ? projection : false;
    });
    assert.notEqual((await helper("one", "ping", { node_id: node("two").id })).exit, 0);
    const final = await discovery(); const finalOne = entry(final, "one"); assert.equal(finalOne.sockets.length, 0);
    const finalLedger = await observerState(); await delay(3000); assert.equal((await observerState()).count, finalLedger.count);
    sample({ revoked_discovery: revoked, final_discovery: final, observer: finalLedger });
    passed("revoked-node-self-exits-report-cleared-no-old-candidate-socket-or-unauthorized-neighbor");
    assert(current); current.status = "PASS"; report.status = "PASS"; report.stage = "complete"; delete report.check;
  } catch (error) {
    report.status = "FAIL"; process.exitCode = 1; if (current) current.status = "FAIL";
    report.error_category = error instanceof assert.AssertionError ? "assertion" : error instanceof ControlApiError ? "controller-http" : "operation";
    console.error(`[managed-discovery] failed: ${report.stage} / ${report.check ?? "operation"} (${report.error_category}); private diagnostics suppressed`);
  } finally {
    cleaning = true;
    for (const name of containers.reverse()) {
      try { assert.equal((await docker(["inspect", name, "--format", `{{index .Config.Labels "${OWNER}"}}`])).trim(), runId); await docker(["rm", "--force", name]); }
      catch { report.cleanup.errors.push(`container:${name}`); }
    }
    for (const name of networks.reverse()) {
      try { assert.equal((await docker(["network", "inspect", name, "--format", `{{index .Labels "${OWNER}"}}`])).trim(), runId); await docker(["network", "rm", name]); }
      catch { report.cleanup.errors.push(`network:${name}`); }
    }
    report.cleanup.status = report.cleanup.errors.length ? "FAIL" : "PASS";
    if (report.cleanup.status === "FAIL") { report.status = "FAIL"; process.exitCode = 1; }
    report.finished_at = new Date().toISOString(); await save(); process.off("SIGINT", signal); process.off("SIGTERM", signal);
    console.log(`[managed-discovery] ${report.status}; public evidence: ${join(artifacts, "result.json")}`);
  }
}

await main().catch(() => { console.error("[managed-discovery] preflight failed: Node 24, inherited shared lock, generated SDK and approved artifact parent required"); process.exitCode = 1; });
