import assert from "node:assert/strict";
import { randomBytes, randomUUID } from "node:crypto";
import { fstatSync } from "node:fs";
import { chmod, lstat, mkdtemp, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { arch } from "node:os";
import { dirname, isAbsolute, join, relative, resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import {
  ControlApiError, createEnrollmentToken, getConnections, getConnectionRuntimes, getPolicy,
  getTags, listNodes, previewFlow, revokeNode, saveConnections, savePolicy, saveTags, setNodeCountry, setNodeTags,
} from "../../packages/control-client/src/index.ts";
import {
  getAgentConnectionsOperation, getAgentDataplaneOperation,
  getMeshAccessOperation as getMeshAccess, saveMeshAccessOperation as saveMeshAccess,
} from "../../packages/control-client/src/generated/operations.gen.ts";
import type {
  AgentConnectionsSnapshot, AgentDataplaneSnapshot, ConnectionPlan, ConnectionsSnapshot,
  ConnectionRuntimesSnapshot, MeshAccessPolicy, MeshAccessSnapshot, PolicySnapshot, RoutingPolicy,
} from "../../packages/control-client/src/index.ts";
import { applied, probed } from "../managed-connections/checks.ts";
import { array, boolean, command, parse, record, text } from "../managed-connections/local.ts";
import { delivered, denied, hasExactCapabilities, local, noFallback, probe, ROLES, tunReady } from "./checks.ts";
import type { ByRole, Local, Probe, Role } from "./checks.ts";

const HERE = dirname(fileURLToPath(import.meta.url));
const WORKSPACE = resolve(HERE, "../..");
const TEMP = "/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode";
const OWNER = "io.meshway.managed-dataplane.run";
const LEASE = 15;
type Evidence = {
  name: string; at: string; status: "PASS" | "OBSERVED";
  policy?: PolicySnapshot; access?: MeshAccessSnapshot; connections?: ConnectionsSnapshot;
  neighbors?: ConnectionRuntimesSnapshot; local?: ByRole<Local>;
  source_policy_rule_count?: number; compiled_path_count?: ByRole<number | null>;
  probes?: { source: Role; destination: Role; observed: Probe }[];
  duration_ms?: number; notes?: string[];
};
type Report = {
  schema_version: 1; run_id: string; status: "RUNNING" | "PASS" | "PREFLIGHT_PASS" | "FAIL";
  mode: "preflight" | "acceptance";
  started_at: string; finished_at?: string; phase: string; check?: string;
  error_category?: "assertion" | "controller-http" | "operation";
  image?: { reference: string; id: string; architecture: string };
  nodes?: ByRole<{ id: string; ipv4: string; public_key: string }>;
  topology?: { networks: ByRole<string[]>; underlay: ByRole<string>; controller_network: string };
  evidence: Evidence[]; cleanup_errors: string[];
};

function options(args: string[]) {
  let image: string | undefined;
  let parent = TEMP;
  let preflightOnly = false;
  if (args.includes("--help")) {
    console.log("Usage: python3 experiments/managed-dataplane/run.py --image meshway-managedtest:TAG [--artifacts-dir EXISTING_PRIVATE_PARENT] [--preflight-only]\nUses an existing native Linux image; main owns image build and tmux. Fixed lease=15s, poll=2s, TUN MTU=1000. Preflight only verifies isolated Docker setup; it does not certify managed data traffic.");
    process.exit(0);
  }
  const seen = new Set<string>();
  for (let i = 0; i < args.length; i++) {
    const flag = args[i];
    assert(flag && !seen.has(flag), "invalid/repeated option"); seen.add(flag);
    if (flag === "--preflight-only") { preflightOnly = true; continue; }
    const value = args[++i];
    assert(value && !value.startsWith("--"), "option requires a value");
    if (flag === "--image") {
      assert(/^meshway-managedtest:[a-zA-Z0-9_.-]+$/.test(value), "dedicated acceptance image required"); image = value;
    } else if (flag === "--artifacts-dir") parent = resolve(value);
    else throw new Error("unknown option");
  }
  assert(image, "main must supply its built --image");
  return { image, parent, preflightOnly };
}
function architecture(value: string): string {
  const names: Record<string, string> = { aarch64: "arm64", arm64: "arm64", x86_64: "amd64", amd64: "amd64", x64: "amd64" };
  const result = names[value.trim()]; assert(result); return result;
}

async function main() {
  const opts = options(process.argv.slice(2));
  assert.equal(Number(process.versions.node.split(".")[0]), 24, "Node 24 required");
  process.umask(0o077);
  const fd = Number(process.env.MESHWAY_DATAPLANE_LOCK_FD);
  assert(Number.isSafeInteger(fd) && fd >= 3, "start via run.py");
  const lock = process.env.MESHWAY_PERF_LOCK; assert(lock);
  const held = fstatSync(fd); const disk = await lstat(lock);
  assert(held.isFile() && disk.isFile() && held.dev === disk.dev && held.ino === disk.ino
    && (held.mode & 0o077) === 0 && held.uid === process.getuid?.());
  const parent = await realpath(opts.parent);
  const rel = relative(WORKSPACE, parent);
  assert(rel.startsWith("..") || isAbsolute(rel), "artifacts must be outside the workspace");
  assert((await lstat(parent)).isDirectory());
  const runId = `${Date.now().toString(36)}-${randomBytes(8).toString("hex")}`;
  const prefix = `meshway-dataplane-${runId}`;
  const artifacts = await mkdtemp(join(parent, `${prefix}-artifacts-`)); await chmod(artifacts, 0o700);
  const report: Report = { schema_version: 1, run_id: runId, status: "RUNNING", mode: opts.preflightOnly ? "preflight" : "acceptance", started_at: new Date().toISOString(), phase: "preflight", evidence: [], cleanup_errors: [] };
  const names = { NYC: `${prefix}-nyc`, LAX: `${prefix}-lax`, SHA: `${prefix}-sha`, controller: `${prefix}-controller` };
  const containers: string[] = []; const networks: string[] = [];
  let secrets: string | undefined;
  let interrupted = false; let cleaning = false;
  let failureEvidence: (() => Promise<void>) | undefined;
  const onSignal = () => { interrupted = true; };
  process.on("SIGINT", onSignal); process.on("SIGTERM", onSignal);
  const save = () => writeFile(join(artifacts, "result.json"), `${JSON.stringify(report, null, 2)}\n`, { mode: 0o600 });
  const stage = (name: string) => { report.phase = name; delete report.check; console.log(`[managed-dataplane] ${name}`); };
  const docker = (args: string[], timeout = 30_000, input?: string) => {
    assert(!interrupted || cleaning, "interrupted");
    return command("docker", args, { timeout, ...(input === undefined ? {} : { input }), cwd: WORKSPACE });
  };
  const helper = async (role: Role | "controller", action: string, data: object = {}) =>
    parse(await docker(["exec", "-i", names[role], "python3", "/private/harness.py", action], 40_000, JSON.stringify(data)));
  const until = async <T>(description: string, test: () => Promise<T | false>, timeout = 80_000): Promise<T> => {
    report.check = description;
    const end = Date.now() + timeout;
    do {
      assert(!interrupted, "interrupted");
      const value = await test(); if (value !== false) return value;
      await delay(500);
    } while (Date.now() < end);
    throw new Error("readiness deadline exceeded");
  };
  const ips: ByRole<string> = { NYC: "", LAX: "", SHA: "" };
  const ids: ByRole<string> = { NYC: "", LAX: "", SHA: "" };
  const bearers: ByRole<string> = { NYC: "", LAX: "", SHA: "" };
  const bodies = { NYC: `meshway-tun:${runId}:NYC\n`, SHA: `meshway-tun:${runId}:SHA\n` };
  const locals = async (): Promise<ByRole<Local>> => {
    const get = async (role: Role) => local(await helper(role, "observe", { destinations: ROLES.filter((peer) => peer !== role).map((peer) => ips[peer]).filter(Boolean) }));
    const [NYC, LAX, SHA] = await Promise.all([get("NYC"), get("LAX"), get("SHA")]);
    for (const value of [NYC, LAX, SHA]) assert(!value.ipv4_forwarding && !value.ipv6_forwarding, "kernel forwarding must stay disabled");
    return { NYC, LAX, SHA };
  };
  try {
    stage("native Linux image and isolated networks");
    report.check = "Docker engine architecture matches host";
    const engine = architecture(await docker(["info", "--format", "{{.Architecture}}"]));
    assert.equal(engine, architecture(arch()));
    report.check = "Docker engine operating system is Linux";
    assert.equal((await docker(["info", "--format", "{{.OSType}}"])).trim(), "linux");
    report.check = "image operating system is Linux";
    assert.equal((await docker(["image", "inspect", opts.image, "--format", "{{.Os}}"])).trim(), "linux");
    report.check = "image architecture is native";
    assert.equal(architecture(await docker(["image", "inspect", opts.image, "--format", "{{.Architecture}}"])), engine);
    report.check = "resolve immutable image identity";
    const imageId = (await docker(["image", "inspect", opts.image, "--format", "{{.Id}}"])).trim();
    assert(/^sha256:[a-f0-9]{64}$/.test(imageId));
    report.image = { reference: opts.image, id: imageId, architecture: engine };
    const netA = `${prefix}-nyc-lax`; const netB = `${prefix}-lax-sha`; const control = `${prefix}-control`;
    for (const network of [netA, netB, control]) {
      report.check = `create owned ${network === netA ? "NYC-LAX" : network === netB ? "LAX-SHA" : "controller API"} bridge`;
      await docker(["network", "create", ...(network === control ? [] : ["--internal"]), "--label", `${OWNER}=${runId}`, network]);
      networks.push(network);
    }
    report.check = "read fixture helper source";
    const helperSource = await readFile(join(HERE, "container.py"), "utf8");
    for (const role of ["controller", ...ROLES] as const) {
      const isController = role === "controller";
      report.check = `${role}: create isolated container`;
      await docker(["create", "--init", "--name", names[role], "--hostname", role.toLowerCase(), "--label", `${OWNER}=${runId}`,
        "--label", `io.meshway.managed-dataplane.role=${role}`, "--user", "0:0",
        "--network", isController || role === "LAX" ? control : role === "NYC" ? netA : netB,
        ...(isController ? ["--network-alias", "controller.mesh.test", "--publish", "127.0.0.1::8420"] : []),
        "--cap-drop", "ALL", ...(!isController ? ["--cap-add", "NET_ADMIN", "--device", "/dev/net/tun:/dev/net/tun:rw"] : []),
        "--security-opt", "no-new-privileges", "--read-only", "--tmpfs", "/private:rw,nosuid,nodev,noexec,mode=0700,size=96m",
        "--sysctl", "net.ipv4.ip_forward=0", "--sysctl", "net.ipv6.conf.all.forwarding=0",
        ...(!isController ? ["--sysctl", "net.ipv6.conf.default.disable_ipv6=1"] : []),
        "--sysctl", "net.ipv4.ping_group_range=0 2147483647", "--env", "PYTHONDONTWRITEBYTECODE=1",
        "--cpus", "1", "--memory", "256m", "--memory-swap", "256m", "--pids-limit", "96", imageId]);
      containers.push(names[role]);
      report.check = `${role}: start isolated container`;
      await docker(["start", names[role]]);
      report.check = `${role}: install helper into private tmpfs`;
      await docker(["exec", "-i", names[role], "python3", "-c", "import os,sys; os.umask(0o077); open('/private/harness.py','x').write(sys.stdin.read())"], 10_000, helperSource);
      report.check = `${role}: inspect isolated container`;
      const inspected = parse(await docker(["inspect", names[role], "--format", "{{json .}}"]));
      const host = record(inspected.HostConfig); const config = record(inspected.Config);
      report.check = `${role}: root UID inside isolated namespace`;
      assert.equal(config.User, "0:0");
      report.check = `${role}: privileged mode disabled`;
      assert.equal(host.Privileged, false);
      report.check = `${role}: read-only root filesystem`;
      assert.equal(host.ReadonlyRootfs, true);
      report.check = `${role}: all default capabilities dropped`;
      assert(hasExactCapabilities(host.CapDrop, ["ALL"]));
      report.check = `${role}: exact capability allowlist`;
      assert(hasExactCapabilities(host.CapAdd, isController ? [] : ["NET_ADMIN"]));
      if (!isController) {
        report.check = `${role}: IPv6 disabled inheritance configured at container creation`;
        assert.equal(record(host.Sysctls)["net.ipv6.conf.default.disable_ipv6"], "1");
      }
      report.check = `${role}: no host state bind mounts`;
      assert(!array(inspected.Mounts).some((mount) => record(mount).Type === "bind"), "no host state mounts allowed");
    }
    report.check = "attach LAX to NYC data bridge";
    await docker(["network", "connect", "--alias", "lax.mesh.test", netA, names.LAX]);
    report.check = "attach LAX to SHA data bridge";
    await docker(["network", "connect", "--alias", "lax.mesh.test", netB, names.LAX]);
    const topology: ByRole<string[]> = { NYC: [], LAX: [], SHA: [] };
    const underlay: ByRole<string> = { NYC: "", LAX: "", SHA: "" };
    const laxAddresses: string[] = [];
    for (const role of ROLES) {
      report.check = `${role}: inspect bridge membership and underlay address`;
      const value = parse(await docker(["inspect", names[role], "--format", "{{json .NetworkSettings.Networks}}"]));
      topology[role] = Object.keys(value).sort();
      underlay[role] = text(record(value[role === "SHA" ? netB : netA]).IPAddress);
      if (role === "LAX") for (const network of [netA, netB]) laxAddresses.push(text(record(value[network]).IPAddress));
      report.check = `${role}: start private underlay TCP canary`;
      await helper(role, "underlay-start");
      await until(`${role}: underlay local TCP canary`, async () => boolean((await helper(role, "underlay-probe", { address: "127.0.0.1" })).reachable), 8_000);
      report.check = `${role}: unprivileged loopback ICMP without NET_RAW`;
      assert(boolean((await helper(role, "loopback-ping")).ok), "unprivileged ping must work without NET_RAW");
    }
    report.check = "NYC: only its dedicated data bridge"; assert.deepEqual(topology.NYC, [netA]);
    report.check = "SHA: only its dedicated data bridge"; assert.deepEqual(topology.SHA, [netB]);
    report.check = "LAX: both data bridges plus controller API bridge"; assert.deepEqual(topology.LAX, [netA, netB, control].sort());
    for (const [source, destination] of [["NYC", "SHA"], ["SHA", "NYC"]] as const) {
      report.check = `${source}: direct ${destination} underlay must be isolated`;
      assert(!boolean((await helper(source, "underlay-probe", { address: underlay[destination] })).reachable));
      report.check = `${source}: LAX DNS resolves only owned addresses`;
      const dns = array((await helper(source, "resolve")).addresses).map(text);
      assert(dns.length > 0 && dns.every((value) => laxAddresses.includes(value)));
      report.check = `${source}: LAX underlay canary is reachable`;
      assert(boolean((await helper(source, "underlay-probe", { address: dns[0] })).reachable));
    }
    report.check = "inspect initial local sockets and kernel state";
    const initialLocal = await locals();
    for (const role of ROLES) {
      report.check = `${role}: effective UID and NET_ADMIN-only capabilities`;
      const value = initialLocal[role];
      assert.equal(value.uid, 0); assert.equal(value.capabilities, 1 << 12, "only NET_ADMIN is permitted");
      report.check = `${role}: forwarding disabled and no pre-existing TUN`;
      assert(!value.ipv4_forwarding && !value.ipv6_forwarding && value.tun.length === 0);
      report.check = `${role}: kernel confirms IPv6 disabled inheritance`;
      assert(value.ipv6_default_disabled);
    }
    report.topology = { networks: topology, underlay, controller_network: control };
    report.evidence.push({ name: "isolated-underlay", status: "PASS", at: new Date().toISOString(), local: initialLocal,
      notes: ["Only LAX joins both data bridges. Controller is separate; LAX forwards TCP only to its fixed controller upstream. No host mounts, host networking, privileged mode, NET_RAW or kernel IP forwarding."] });
    report.check = "LAX: start fixed-upstream controller proxy";
    await helper("LAX", "controller-proxy-start");
    if (opts.preflightOnly) {
      report.status = "PREFLIGHT_PASS"; report.phase = "preflight-complete"; delete report.check;
      return;
    }

    stage("private TLS profiles");
    secrets = await mkdtemp(join(parent, `${prefix}-certificates-`)); await chmod(secrets, 0o700);
    const certificateDirectory = secrets;
    const openssl = (args: string[]) => command("openssl", args, { cwd: certificateDirectory, timeout: 30_000 });
    await writeFile(join(secrets, "ca.cnf"), "[req]\ndistinguished_name=dn\nx509_extensions=ca\nprompt=no\n[dn]\nCN=Meshway isolated managed dataplane CA\n[ca]\nbasicConstraints=critical,CA:TRUE\nkeyUsage=critical,keyCertSign,cRLSign\n", { mode: 0o600 });
    await writeFile(join(secrets, "server.ext"), "basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\nsubjectAltName=DNS:lax.mesh.test\n", { mode: 0o600 });
    await openssl(["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "2", "-config", "ca.cnf", "-keyout", "ca-key.pem", "-out", "ca.pem"]);
    await openssl(["req", "-new", "-newkey", "rsa:2048", "-nodes", "-subj", "/CN=lax.mesh.test", "-keyout", "lax-key.pem", "-out", "lax.csr"]);
    await openssl(["x509", "-req", "-in", "lax.csr", "-CA", "ca.pem", "-CAkey", "ca-key.pem", "-CAcreateserial", "-days", "2", "-extfile", "server.ext", "-out", "lax-cert.pem"]);
    const uuid = randomUUID();
    await helper("NYC", "install-files", { files: [{ name: "profiles.json", content: JSON.stringify({ version: 1, profiles: [] }) }] });
    await helper("LAX", "install-files", { files: [
      { name: "profiles.json", content: JSON.stringify({ version: 1, profiles: [{ id: "lax-vless-server", role: "server", uuid, certificate: "lax-cert.pem", private_key: "lax-key.pem" }] }) },
      { name: "lax-cert.pem", content: await readFile(join(secrets, "lax-cert.pem"), "utf8") },
      { name: "lax-key.pem", content: await readFile(join(secrets, "lax-key.pem"), "utf8") },
    ] });
    await helper("SHA", "install-files", { files: [
      { name: "profiles.json", content: JSON.stringify({ version: 1, profiles: [{ id: "lax-vless-client", role: "client", uuid, ca_certificate: "ca.pem", server_name: "lax.mesh.test" }] }) },
      { name: "ca.pem", content: await readFile(join(secrets, "ca.pem"), "utf8") },
    ] });
    await rm(secrets, { recursive: true, force: true }); secrets = undefined;

    stage("fresh controller and actual enrolled identities");
    await helper("controller", "controller-start");
    const port = (await docker(["port", names.controller, "8420/tcp"])).trim(); assert(/^127\.0\.0\.1:\d+$/.test(port));
    const baseUrl = `http://${port}`;
    const admin = await until("controller bootstrap", async () => { try { return text((await helper("controller", "admin-token")).token); } catch { return false; } }, 20_000);
    await until("generated controller API readiness", async () => { try { return await getConnections(baseUrl, admin); } catch { return false; } }, 20_000);
    assert.equal((await listNodes(baseUrl, admin)).nodes.length, 0, "fresh controller only");
    const initialAccess = await getMeshAccess(baseUrl, admin);
    assert.deepEqual(initialAccess.policy, { default_action: "reject", rules: [] });
    assert.equal(initialAccess.source_jsonc, null);
    const initialPolicy = await getPolicy(baseUrl, admin);
    assert.deepEqual(initialPolicy.policy, { default_action: "reject", rules: [] });
    assert.equal(initialPolicy.source_jsonc, null);
    failureEvidence = async () => {
      const evidence: Evidence = { name: "failure-observations", status: "OBSERVED", at: new Date().toISOString() };
      try { evidence.local = await locals(); } catch { /* Docker may be unavailable. */ }
      try { evidence.connections = await getConnections(baseUrl, admin); evidence.neighbors = await getConnectionRuntimes(baseUrl, admin); } catch { /* Controller may be deliberately offline. */ }
      report.evidence.push(evidence);
    };
    for (const role of ROLES) {
      const token = await createEnrollmentToken(baseUrl, admin, { expires_in_seconds: 300 });
      ids[role] = text((await helper(role, "enroll", { name: role, token: token.token })).node_id);
      assert(/^[0-9a-f]{64}$/.test(ids[role]));
      bearers[role] = text((await helper(role, "node-bearer")).token);
      await helper(role, "agent-start");
    }
    const inventory = await until("registered key/capability publication", async () => {
      const value = await getConnections(baseUrl, admin);
      return value.nodes.length === 3 && ROLES.every((role) => value.nodes.some((entry) => entry.node.id === ids[role]
        && entry.node.revoked_at === null && entry.public_key !== null
        && entry.capabilities.some((cap) => cap.transport === (role === "NYC" ? "native_udp" : "vless") && cap.can_dial && cap.can_accept))) ? value : false;
    });
    assert.equal(new Set(Object.values(ids)).size, 3);
    const nodes: ByRole<{ id: string; ipv4: string; public_key: string }> = { NYC: { id: "", ipv4: "", public_key: "" }, LAX: { id: "", ipv4: "", public_key: "" }, SHA: { id: "", ipv4: "", public_key: "" } };
    for (const role of ROLES) {
      const item = inventory.nodes.find((entry) => entry.node.id === ids[role]); assert(item?.public_key);
      assert(/^100\.96\.\d{1,3}\.\d{1,3}$/.test(item.node.mesh_ipv4), "fresh controller-assigned 100.96 addresses required");
      const localIdentity = await helper(role, "public-identity");
      assert.equal(localIdentity.node_id, ids[role]); assert.equal(localIdentity.public_key, item.public_key);
      assert(/^[0-9a-f]{64}$/.test(item.public_key));
      ips[role] = item.node.mesh_ipv4; nodes[role] = { id: ids[role], ipv4: ips[role], public_key: item.public_key };
    }
    report.nodes = nodes;
    for (const [role, country] of [["NYC", "US"], ["SHA", "CN"]] as const) {
      await setNodeCountry(baseUrl, admin, { expected_revision: (await getPolicy(baseUrl, admin)).revision, node_id: ids[role], country });
    }
    const plan: ConnectionPlan = {
      listeners: [
        { id: "tun-lax-udp", node_id: ids.LAX, transport: "native_udp", bind: "0.0.0.0:41000", advertised_endpoints: [{ host: "lax.mesh.test", port: 41000 }], credential_ref: null },
        { id: "tun-lax-vless", node_id: ids.LAX, transport: "vless", bind: "0.0.0.0:41001", advertised_endpoints: [{ host: "lax.mesh.test", port: 41001 }], credential_ref: "lax-vless-server" },
      ],
      candidates: [
        { id: "tun-nyc-udp", dialer_node_id: ids.NYC, listener_id: "tun-lax-udp", endpoint: { host: "lax.mesh.test", port: 41000 }, priority: 0, credential_ref: null },
        { id: "tun-sha-vless", dialer_node_id: ids.SHA, listener_id: "tun-lax-vless", endpoint: { host: "lax.mesh.test", port: 41001 }, priority: 0, credential_ref: "lax-vless-client" },
      ],
    };
    const savedPlan = await saveConnections(baseUrl, admin, { expected_revision: (await getConnections(baseUrl, admin)).revision, configuration: plan }); assert(savedPlan.valid);
    const routing: RoutingPolicy = { default_action: "reject", rules: [
      { id: "tun-nyc-sha-cidr", source: { node_ids: [ids.NYC], countries: ["US"] }, destination_scope: "mesh",
        destination_any: [{ kind: "cidr", value: `${ips.SHA}/32` }], action: { kind: "route", via: [ids.LAX], exit_node_id: null } },
      { id: "tun-sha-nyc-country", source: { node_ids: [], countries: ["CN"] }, destination_scope: "mesh",
        destination_any: [{ kind: "node", value: ids.NYC }], action: { kind: "route", via: [ids.LAX], exit_node_id: null } },
    ] };
    const emptyAccess: MeshAccessPolicy = { default_action: "reject", rules: [] };
    const forward: MeshAccessPolicy = { default_action: "reject", rules: [{ id: "tun-nyc-to-sha", source: [`node:${ids.NYC}`],
      destination: [`node:${ids.SHA}`], action: "allow", bidirectional: false }] };
    const both: MeshAccessPolicy = { default_action: "reject", rules: [{ ...forward.rules[0]!, id: "tun-nyc-sha-bidirectional", bidirectional: true }] };
    const putPolicy = async (policy: RoutingPolicy) => {
      const value = await savePolicy(baseUrl, admin, { expected_revision: (await getPolicy(baseUrl, admin)).revision, policy, source_jsonc: null });
      assert(value.valid); assert.deepEqual(value.diagnostics, []); assert.equal(value.source_jsonc, null); return value;
    };
    const putAccess = async (policy: MeshAccessPolicy) => {
      const value = await saveMeshAccess(baseUrl, admin, { expected_revision: (await getMeshAccess(baseUrl, admin)).revision, policy, source_jsonc: null });
      assert(value.valid); assert.deepEqual(value.diagnostics, []); assert.equal(value.source_jsonc, null); return value;
    };
    await putPolicy(routing);
    const scopedConnections = async (): Promise<ByRole<AgentConnectionsSnapshot>> => {
      const [NYC, LAX, SHA] = await Promise.all(ROLES.map((role) => getAgentConnectionsOperation(baseUrl, bearers[role]))); assert(NYC && LAX && SHA); return { NYC, LAX, SHA };
    };
    const graph = async (expected: ByRole<AgentConnectionsSnapshot>) => until("fresh bidirectional authenticated neighbor probes", async () => {
      const snapshot = await getConnectionRuntimes(baseUrl, admin);
      const NYC = snapshot.nodes.find((node) => node.node_id === ids.NYC);
      const LAX = snapshot.nodes.find((node) => node.node_id === ids.LAX);
      const SHA = snapshot.nodes.find((node) => node.node_id === ids.SHA);
      if (!NYC || !LAX || !SHA || !applied(NYC, expected.NYC) || !applied(LAX, expected.LAX) || !applied(SHA, expected.SHA)) return false;
      return NYC.peers.length === 1 && LAX.peers.length === 2 && SHA.peers.length === 1
        && probed(NYC, ids.LAX, ids.NYC, "native_udp", "tun-lax-udp", "tun-nyc-udp")
        && probed(LAX, ids.NYC, ids.NYC, "native_udp", "tun-lax-udp", "tun-nyc-udp")
        && probed(SHA, ids.LAX, ids.SHA, "vless", "tun-lax-vless", "tun-sha-vless")
        && probed(LAX, ids.SHA, ids.SHA, "vless", "tun-lax-vless", "tun-sha-vless") ? snapshot : false;
    });
    const lastReady = new Map<Role, { revision: number; pid: number | null; events: number }>();
    const ready = async () => {
      const policy = await getPolicy(baseUrl, admin); const access = await getMeshAccess(baseUrl, admin);
      assert.equal(policy.revision, access.revision);
      assert.equal(policy.source_jsonc, null); assert.equal(access.source_jsonc, null);
      const expected = await scopedConnections();
      const [NYC, LAX, SHA] = await Promise.all(ROLES.map((role) => getAgentDataplaneOperation(baseUrl, bearers[role])));
      assert(NYC && LAX && SHA);
      const coherent: ByRole<AgentDataplaneSnapshot> = { NYC, LAX, SHA };
      for (const role of ROLES) {
        const value = coherent[role];
        assert.equal(value.payload.node_id, ids[role]);
        assert.equal(value.payload.revision, policy.revision); assert(value.payload.access_valid);
        assert.equal(value.payload.configuration.revision, policy.revision);
        assert.equal(value.payload.connections.revision, policy.revision);
        for (const identity of value.payload.identities) {
          const exact = Object.values(nodes).find((node) => node.id === identity.node_id); assert(exact);
          assert.equal(identity.public_key, exact.public_key);
        }
      }
      const observations = await until("configuration ACK plus new independent local TUN generation proof", async () => {
        const inventory = await listNodes(baseUrl, admin); const observations = await locals();
        return ROLES.every((role) => inventory.nodes.some((node) => node.id === ids[role] && node.acknowledged_revision === policy.revision)
          && tunReady(observations[role], ids[role], ips[role], coherent[role].payload.revision, coherent[role].digest)
          && (() => {
            const before = lastReady.get(role); const now = observations[role];
            return before === undefined || before.revision === policy.revision || before.pid !== now.agent_pid || now.ready_events > before.events;
          })()) ? observations : false;
      });
      const neighbors = await graph(expected);
      for (const role of ROLES) lastReady.set(role, { revision: policy.revision, pid: observations[role].agent_pid, events: observations[role].ready_events });
      return { policy, access, neighbors };
    };
    const capture = async (name: string, extra: Partial<Pick<Evidence, "probes" | "duration_ms" | "notes">> = {}, revoked: Role | null = null) => {
      const access = await getMeshAccess(baseUrl, admin);
      const compiled = await Promise.all(ROLES.map(async (role) => {
        try { return (await getAgentDataplaneOperation(baseUrl, bearers[role])).payload.access_policy.grants.length; }
        catch (error) { if (role === revoked && error instanceof ControlApiError && error.status === 403) return null; throw error; }
      }));
      report.evidence.push({ name, status: "PASS", at: new Date().toISOString(), policy: await getPolicy(baseUrl, admin),
        access, source_policy_rule_count: access.policy.rules.length,
        compiled_path_count: { NYC: compiled[0]!, LAX: compiled[1]!, SHA: compiled[2]! },
        connections: await getConnections(baseUrl, admin),
        neighbors: await getConnectionRuntimes(baseUrl, admin), local: await locals(), ...extra }); await save();
    };
    const oneProbe = async (source: Role, destination: "NYC" | "SHA") => probe(await helper(source, "probe", { address: ips[destination], body: bodies[destination] }));
    const directions = [["NYC", "SHA"], ["SHA", "NYC"]] as const;
    const probes = async () => Promise.all(directions.map(async ([source, destination]) => ({ source, destination, observed: await oneProbe(source, destination) })));
    const localServers = async () => {
      for (const role of ["NYC", "SHA"] as const) assert(boolean((await helper(role, "self-http", { address: ips[role], body: bodies[role] })).ready), "mesh-only target server must remain healthy during negatives");
    };
    const success = async (name: string) => {
      await ready();
      const observed = await until("both kernel ICMP and distinctive mesh HTTP body", async () => { const value = await probes(); return value.every((item) => delivered(item.observed)) ? value : false; });
      await capture(name, { probes: observed });
    };
    const negative = async (name: string, configReady = true, duration = 6_000) => {
      if (configReady) await ready();
      await localServers();
      const started = Date.now(); const observed: NonNullable<Evidence["probes"]> = [];
      do {
        const batch = await probes(); assert(batch.every((item) => denied(item.observed)), "denial must fail ping/HTTP and retain kernel TUN/guard routing");
        observed.push(...batch);
      } while (Date.now() - started < duration);
      if (configReady) await graph(await scopedConnections());
      await capture(name, { probes: observed, duration_ms: Date.now() - started });
    };

    stage("empty access denies saved routes while adjacencies remain authenticated");
    await ready();
    const initial = await locals();
    assert(!initial.SHA.listeners.some((socket) => socket.port === 41001));
    assert(initial.SHA.established.some((socket) => socket.peer_port === 41001 && laxAddresses.includes(socket.peer_host) && socket.local_port !== 41001));
    assert(initial.LAX.established.some((socket) => socket.local_port === 41001 && socket.peer_host === underlay.SHA));
    for (const role of ["NYC", "SHA"] as const) {
      await helper(role, "http-start", { address: ips[role], body: bodies[role] });
      await until("mesh-IP-only target HTTP listener", async () => boolean((await helper(role, "self-http", { address: ips[role], body: bodies[role] })).ready), 8_000);
      const value = (await locals())[role];
      assert(value.listeners.some((socket) => socket.port === 43000 && socket.host === ips[role]));
      assert(!value.listeners.some((socket) => socket.port === 43000 && socket.host !== ips[role]));
    }
    await negative("empty-access-default-deny");

    stage("forward-only endpoint rule delivers ICMP but denies return traffic");
    await putAccess(forward); await ready();
    const beforeEcho = (await locals()).SHA.icmp.InEchos;
    await negative("forward-only-no-implicit-reverse");
    const afterEcho = (await locals()).SHA.icmp.InEchos;
    assert(afterEcho > beforeEcho, "one-way rule must deliver actual ICMP to SHA, not just fail before transmission");
    await capture("forward-delivery-independent-kernel-counter", { notes: [`SHA Icmp.InEchos increased ${afterEcho - beforeEcho}; ping/HTTP round trips remained denied. No raw socket/capture capability.`] });

    stage("one bidirectional endpoint rule enables real kernel TUN ping and HTTP");
    await putAccess(both); await success("bidirectional-rule-kernel-traffic");

    stage("administrator tags drive one bidirectional endpoint rule");
    const tagCatalog = [{ tag: "tag:client", description: "managed dataplane client" }, { tag: "tag:server", description: "managed dataplane server" }];
    await saveTags(baseUrl, admin, { expected_revision: (await getTags(baseUrl, admin)).revision, tags: tagCatalog });
    await setNodeTags(baseUrl, admin, { expected_revision: (await getTags(baseUrl, admin)).revision, node_id: ids.NYC, tags: ["tag:client"] });
    await setNodeTags(baseUrl, admin, { expected_revision: (await getTags(baseUrl, admin)).revision, node_id: ids.SHA, tags: ["tag:server"] });
    const tagged: MeshAccessPolicy = { default_action: "reject", rules: [{ id: "tagged-client-server", source: ["tag:client"],
      destination: ["tag:server"], action: "allow", bidirectional: true }] };
    await putAccess(tagged);
    const taggedPreview = await previewFlow(baseUrl, admin, { flow: { source_node_id: ids.NYC, destination_node_id: ids.SHA,
      destination_ip: ips.SHA, domain: null, destination_country: null } });
    assert(taggedPreview.valid && taggedPreview.execution_supported);
    assert.equal(taggedPreview.authorization.action, "allow"); assert.equal(taggedPreview.authorization.rule_id, "tagged-client-server");
    assert.deepEqual(taggedPreview.routing.path, [ids.NYC, ids.LAX, ids.SHA]);
    await success("tag-selectors-bidirectional-kernel-traffic");

    stage("earlier wildcard-to-server reject wins before bidirectional tag allow");
    const taggedFirstReject: MeshAccessPolicy = { ...tagged, rules: [{ id: "reject-to-tagged-server", source: ["*"], destination: ["tag:server"],
      action: "reject", bidirectional: false }, ...tagged.rules] };
    await putAccess(taggedFirstReject); await negative("access-first-match-forward-reject");
    await putAccess(tagged); await success("access-first-match-forward-reject-removed");

    stage("tag removal and move withdraw authorization until membership is restored");
    await setNodeTags(baseUrl, admin, { expected_revision: (await getTags(baseUrl, admin)).revision, node_id: ids.SHA, tags: [] });
    await negative("server-tag-removed");
    await setNodeTags(baseUrl, admin, { expected_revision: (await getTags(baseUrl, admin)).revision, node_id: ids.LAX, tags: ["tag:server"] });
    await negative("server-tag-moved-away-from-endpoint");
    await setNodeTags(baseUrl, admin, { expected_revision: (await getTags(baseUrl, admin)).revision, node_id: ids.SHA, tags: ["tag:server"] });
    await setNodeTags(baseUrl, admin, { expected_revision: (await getTags(baseUrl, admin)).revision, node_id: ids.LAX, tags: [] });
    await success("server-tag-membership-restored");

    stage("tag catalog edits preserve existing assignments");
    const editedTags = await saveTags(baseUrl, admin, { expected_revision: (await getTags(baseUrl, admin)).revision, tags: [
      { tag: "tag:client", description: "updated managed dataplane client" }, tagCatalog[1]!, { tag: "tag:west", description: "western region" },
    ] });
    assert.deepEqual(editedTags.assignments.find((entry) => entry.node_id === ids.NYC)?.tags, ["tag:client"]);
    assert.deepEqual(editedTags.assignments.find((entry) => entry.node_id === ids.SHA)?.tags, ["tag:server"]);
    await success("tag-catalog-edit-preserved-memberships");

    stage("default allow still intersects mandatory routing and route rejection");
    await putAccess({ default_action: "allow", rules: [] }); await success("access-default-allow-keeps-via-LAX");
    const first = routing.rules[0]; assert(first);
    await putPolicy({ ...routing, rules: [{ ...first, id: "tun-own-first-reject", action: { kind: "reject", via: [], exit_node_id: null } }, ...routing.rules] });
    await negative("route-reject-overrides-access-default-allow");
    await putPolicy(routing); await putAccess(tagged); await success("default-reject-tag-policy-restored");

    stage("reverse-direction first match rejects at its ordered position");
    await putAccess({ ...tagged, rules: [{ id: "reject-server-to-client", source: ["tag:server"], destination: ["tag:client"],
      action: "reject", bidirectional: false }, ...tagged.rules] });
    await negative("access-first-match-reverse-reject");
    await putAccess(tagged); await success("access-first-match-reverse-reject-removed");

    stage("ordered routing first-match reject overrides access allow");
    await putPolicy({ ...routing, rules: [{ ...first, id: "tun-own-first-reject", action: { kind: "reject", via: [], exit_node_id: null } }, ...routing.rules] });
    await negative("first-match-reject");
    const rejected = await getPolicy(baseUrl, admin);
    await putPolicy({ ...rejected.policy, rules: rejected.policy.rules.filter((rule) => rule.id !== "tun-own-first-reject") });
    await success("first-match-reject-removed");

    stage("a newly selected direct route requires its own authenticated adjacency");
    await putPolicy({ default_action: "direct", rules: [] }); await negative("direct-route-without-direct-adjacency");
    await putPolicy(routing); await success("via-LAX-restored");

    stage("country selector changes take effect independently of endpoint access");
    await setNodeCountry(baseUrl, admin, { expected_revision: (await getPolicy(baseUrl, admin)).revision, node_id: ids.SHA, country: "US" });
    await negative("source-country-mismatch");
    await setNodeCountry(baseUrl, admin, { expected_revision: (await getPolicy(baseUrl, admin)).revision, node_id: ids.SHA, country: "CN" });
    await success("source-country-restored");

    stage("endpoint access withdrawal with saved routing intact");
    await putAccess(emptyAccess); await negative("access-rule-withdrawal");
    await putAccess(tagged); await success("access-rules-restored");

    stage("unavailable bootstrap DNS retries without activating cached grants");
    await helper("NYC", "agent-stop");
    assert(!(await locals()).NYC.agent_running);
    const delayedHost = `delayed-${runId}.mesh.test`;
    const delayedPlan: ConnectionPlan = {
      listeners: plan.listeners.map((listener) => listener.id === "tun-lax-udp"
        ? { ...listener, advertised_endpoints: [{ host: delayedHost, port: 41000 }] } : listener),
      candidates: plan.candidates.map((candidate) => candidate.id === "tun-nyc-udp"
        ? { ...candidate, endpoint: { host: delayedHost, port: 41000 } } : candidate),
    };
    const delayed = await saveConnections(baseUrl, admin, {
      expected_revision: (await getConnections(baseUrl, admin)).revision, configuration: delayedPlan,
    });
    assert(delayed.valid);
    await helper("NYC", "agent-start");
    await until("fresh bundle persisted while transport DNS is unavailable", async () => {
      const value = (await locals()).NYC;
      assert(value.agent_running, "transient bootstrap DNS must not terminate the Agent");
      return value.saved_revision === delayed.revision ? value : false;
    });
    const dnsWaitAt = Date.now();
    do {
      const value = (await locals()).NYC;
      assert(value.agent_running && value.tun.length === 0 && value.routes.every((route) => route.kind === "guard" && noFallback(route)),
        "DNS retries must keep the owned unreachable route and never install cached packet grants");
      await delay(500);
    } while (Date.now() - dnsWaitAt < 6_000);
    await capture("bootstrap-DNS-unavailable-retained-guard", { duration_ms: Date.now() - dnsWaitAt });
    // Docker registers aliases on network attachment; this changes DNS without
    // restarting the waiting Agent or widening the two-bridge topology.
    await docker(["network", "disconnect", netA, names.LAX]);
    await docker(["network", "connect", "--alias", "lax.mesh.test", "--alias", delayedHost, netA, names.LAX]);
    await success("bootstrap-DNS-recovery-fresh-packet-generation");
    await saveConnections(baseUrl, admin, {
      expected_revision: (await getConnections(baseUrl, admin)).revision, configuration: plan,
    });
    await success("original-endpoint-restored-after-DNS-recovery");

    stage("mandatory relay failure while controller remains reachable");
    const relayFenceAt = Date.now();
    const preRelay = await until("endpoint reports freshly renewed before relay-only failure", async () => {
      const value = await getConnectionRuntimes(baseUrl, admin);
      return [ids.NYC, ids.SHA].every((id) => value.nodes.some((node) => node.node_id === id
        && node.current && !node.stale && node.reported_at !== null && Date.parse(node.reported_at) >= relayFenceAt)) ? value : false;
    }, 12_000);
    const endpointLeaseFloor = Math.min(...preRelay.nodes.filter((node) => node.node_id === ids.NYC || node.node_id === ids.SHA)
      .map((node) => { assert(node.reported_at); return Date.parse(node.reported_at); }));
    await helper("LAX", "agent-stop");
    assert(!(await locals()).LAX.agent_running);
    await negative("mandatory-LAX-stopped-before-lease", false, 3_500);
    assert(Date.now() - endpointLeaseFloor < (LEASE - 2) * 1_000, "relay failure must be observed before unrelated freshness leases can expire");
    assert.equal((await listNodes(baseUrl, admin)).nodes.length, 3, "controller must remain independently reachable");
    await helper("LAX", "agent-start"); await success("relay-restarted");
    const nextRelay = await getConnectionRuntimes(baseUrl, admin);
    const oldLax = preRelay.nodes.find((node) => node.node_id === ids.LAX); const newLax = nextRelay.nodes.find((node) => node.node_id === ids.LAX);
    assert(oldLax && newLax && newLax.epoch > oldLax.epoch, "relay restart requires a replacement process epoch");

    stage("controller outage withdraws TUN packet paths after local lease");
    const preOutage = await getConnectionRuntimes(baseUrl, admin);
    await helper("controller", "controller-stop");
    const outageAt = Date.now();
    let offline = false; try { await getConnections(baseUrl, admin); } catch { offline = true; } assert(offline);
    const expired = await until("all independent freshness leases close sockets and keep route guards", async () => {
      if (Date.now() - outageAt <= (LEASE + 1) * 1_000) return false;
      const value = await locals();
      return Object.values(value).every((node) => node.agent_running && node.agent_udp_ports.length === 0
        && node.dataplane_proof?.tun_ready === false && node.dataplane_proof.allowed_paths === 0 && node.dataplane_proof.reason === "lease_expired"
        && !node.listeners.some((socket) => socket.port === 41000 || socket.port === 41001)
        && !node.established.some((socket) => socket.local_port === 41001 || socket.peer_port === 41001)
        && node.routes.every(noFallback)) ? value : false;
    }, 40_000);
    const outageProbes = await probes(); assert(outageProbes.every((value) => denied(value.observed)));
    const repeatOutage = await probes(); assert(repeatOutage.every((value) => denied(value.observed)));
    report.evidence.push({ name: "controller-outage-independent-lease", status: "PASS", at: new Date().toISOString(), local: expired,
      probes: [...outageProbes, ...repeatOutage], duration_ms: Date.now() - outageAt,
      notes: ["Controller stopped; all three Agent processes remain alive. Evidence is local sockets/routes and actual kernel probes, not stale controller runtime claims."] }); await save();
    await helper("controller", "controller-start");
    await until("restarted controller", async () => { try { return await getConnections(baseUrl, admin); } catch { return false; } }, 20_000);
    await success("controller-recovery-fresh-data-traffic");
    const postOutage = await getConnectionRuntimes(baseUrl, admin);
    for (const role of ROLES) {
      const before = preOutage.nodes.find((node) => node.node_id === ids[role]); const after = postOutage.nodes.find((node) => node.node_id === ids[role]); assert(before && after);
      assert.equal(after.epoch, before.epoch); assert(after.sequence > before.sequence);
      assert.equal((await helper(role, "identity")).node_id, ids[role]);
    }

    stage("final endpoint revocation denies API and packet delivery");
    await revokeNode(baseUrl, admin, { node_id: ids.SHA });
    const tagsAfterRevoke = await getTags(baseUrl, admin);
    assert(!tagsAfterRevoke.assignments.some((entry) => entry.node_id === ids.SHA), "revocation must clean SHA tag membership");
    assert(tagsAfterRevoke.tags.some((entry) => entry.tag === "tag:server"), "policy-referenced tag definition must remain");
    let referencedTagRejected = false;
    try {
      await saveTags(baseUrl, admin, { expected_revision: tagsAfterRevoke.revision, tags: tagsAfterRevoke.tags.filter((entry) => entry.tag !== "tag:server") });
    } catch (error) { referencedTagRejected = error instanceof ControlApiError && error.status === 422; }
    assert(referencedTagRejected, "deleting a policy-referenced tag must return 422 without silently changing policy");
    assert.deepEqual(await getTags(baseUrl, admin), tagsAfterRevoke, "failed referenced-tag deletion must be atomic");
    let forbidden = false;
    try { await getAgentDataplaneOperation(baseUrl, bearers.SHA); } catch (error) { forbidden = error instanceof ControlApiError && error.status === 403; }
    assert(forbidden, "revoked node must lose its generated dataplane API");
    await until("revoked SHA exits and withdraws carrier", async () => {
      const value = await locals(); return !value.SHA.agent_running && !value.SHA.established.some((socket) => socket.peer_port === 41001) ? value : false;
    });
    const revokedRevision = (await getPolicy(baseUrl, admin)).revision;
    await until("surviving Agents save and ACK revocation withdrawal", async () => {
      const inventory = await listNodes(baseUrl, admin); const value = await locals();
      return (["NYC", "LAX"] as const).every((role) => value[role].agent_running && value[role].saved_revision === revokedRevision
        && value[role].dataplane_proof?.tun_ready === false && inventory.nodes.some((node) => node.id === ids[role] && node.acknowledged_revision === revokedRevision));
    });
    const revokedProbes: NonNullable<Evidence["probes"]> = [];
    const revokeCheckAt = Date.now();
    do {
      const value = await probes(); assert(value.every((item) => denied(item.observed))); revokedProbes.push(...value);
    } while (Date.now() - revokeCheckAt < 6_000);
    const runtime = await getConnectionRuntimes(baseUrl, admin); const sha = runtime.nodes.find((node) => node.node_id === ids.SHA);
    assert(sha && sha.state === "withdrawn" && !sha.current && sha.stale && sha.peers.length === 0 && sha.reported_at === null);
    await capture("revoked-endpoint", { probes: revokedProbes, notes: ["Generated getAgentDataplane returned 403; revoked process exited; runtime retained no fresh adjacency claims."] }, "SHA");
    report.status = "PASS"; report.phase = "complete"; delete report.check;
  } catch (error) {
    report.status = "FAIL"; process.exitCode = 1;
    report.error_category = error instanceof assert.AssertionError ? "assertion" : error instanceof ControlApiError ? "controller-http" : "operation";
    console.error(`[managed-dataplane] failed: ${report.phase}${report.check ? ` / ${report.check}` : ""} (${report.error_category}); private exception details suppressed`);
    if (!interrupted && failureEvidence) await failureEvidence();
  } finally {
    cleaning = true;
    const cleanup = async (resource: string, action: () => Promise<unknown>) => {
      try { await action(); } catch { report.cleanup_errors.push(resource); process.exitCode = 1; }
    };
    for (const name of containers.reverse()) await cleanup(`container:${name}`, async () => {
      assert.equal((await docker(["inspect", name, "--format", `{{index .Config.Labels "${OWNER}"}}`])).trim(), runId);
      await docker(["rm", "--force", name]);
    });
    for (const name of networks.reverse()) await cleanup(`network:${name}`, async () => {
      assert.equal((await docker(["network", "inspect", name, "--format", `{{index .Labels "${OWNER}"}}`])).trim(), runId);
      await docker(["network", "rm", name]);
    });
    const remaining = secrets;
    if (remaining !== undefined) await cleanup("private-certificate-directory", () => rm(remaining, { recursive: true, force: true }));
    if (report.cleanup_errors.length) report.status = "FAIL";
    report.finished_at = new Date().toISOString(); await save();
    process.off("SIGINT", onSignal); process.off("SIGTERM", onSignal);
    console.log(`[managed-dataplane] ${report.status}; sanitized report: ${join(artifacts, "result.json")}`);
  }
}

await main().catch(() => {
  console.error("[managed-dataplane] preflight failed: require regenerated SDK, Node 24, run.py lock, built native Linux image and external artifact directory");
  process.exitCode = 1;
});
