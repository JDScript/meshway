import assert from "node:assert/strict";
import { randomBytes, randomUUID } from "node:crypto";
import { fstatSync } from "node:fs";
import { chmod, lstat, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { arch } from "node:os";
import { dirname, isAbsolute, join, relative, resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import {
  ControlApiError, createEnrollmentToken, getConnections, getConnectionRuntimes,
  getNetworkPolicy, listNetworks, revokeNode, saveConnections,
} from "../../packages/control-client/src/index.ts";
import type {
  AgentConnectionsSnapshot, ConnectionNode, ConnectionPlan, ConnectionRuntimesSnapshot, ConnectionsSnapshot,
} from "../../packages/control-client/src/index.ts";
import { getAgentConnectionsOperation } from "../../packages/control-client/src/generated/operations.gen.ts";
import { applied, probed } from "./checks.ts";
import { fixtureNetworkId } from "./network-scope.ts";
import { array, boolean, command, parse, record, sockets, text } from "./local.ts";
import type { SocketObservation } from "./local.ts";

const HERE = dirname(fileURLToPath(import.meta.url));
const WORKSPACE = resolve(HERE, "../..");
const DEFAULT_TEMP = "/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode";
const ROLES = ["NYC", "LAX", "SHA"] as const;
type Role = typeof ROLES[number];
type ByRole<T> = Record<Role, T>;
type Evidence = {
  name: string; at: string; notes?: string[];
  connections?: ConnectionsSnapshot; runtime?: ConnectionRuntimesSnapshot;
  sockets?: ByRole<SocketObservation>;
};
type Artifact = {
  schema_version: 1; run_id: string; started_at: string; finished_at?: string;
  status: "running" | "passed" | "failed"; phase: string;
  check?: string;
  node_ids?: ByRole<string>; architecture?: { host: string; engine: string; image: string };
  image?: { reference: string; id: string };
  topology?: { networks: ByRole<string[]>; addresses: ByRole<string>; dns: ByRole<string[]>; controller_api_network: string };
  evidence: Evidence[]; cleanup_errors: string[];
};

function options(args: string[]) {
  let image: string | undefined;
  let parent = DEFAULT_TEMP;
  let lease = 10;
  for (let i = 0; i < args.length; i++) {
    const flag = args[i];
    if (flag === "--help") {
      console.log("Usage: python3 experiments/managed-connections/run.py --image meshway-managedtest:TAG [--artifacts-dir EXISTING_PARENT] [--lease-seconds 10..45]\nUse build.py to package source-verified native binaries first; this runner retains the supplied image and cleans its owned containers/networks.");
      process.exit(0);
    }
    const value = args[++i];
    assert(value !== undefined, "every option requires a value");
    if (flag === "--image") {
      assert(/^meshway-managedtest:[a-zA-Z0-9_.-]+$/.test(value), "--image must use the dedicated meshway-managedtest repository");
      image = value;
    } else if (flag === "--artifacts-dir") parent = resolve(value);
    else if (flag === "--lease-seconds") {
      lease = Number(value);
      assert(Number.isInteger(lease) && lease >= 10 && lease <= 45, "lease must be 10..45 seconds");
    } else throw new Error("unknown option");
  }
  assert(image, "--image is required; package source-verified native binaries with build.py first");
  return { image, parent, lease };
}

function normalizeArchitecture(value: string): string {
  const aliases: Record<string, string> = { aarch64: "arm64", arm64: "arm64", x86_64: "amd64", amd64: "amd64", x64: "amd64" };
  const result = aliases[value.trim()];
  assert(result, "unsupported native architecture");
  return result;
}

async function main() {
  const opts = options(process.argv.slice(2));
  assert(Number(process.versions.node.split(".")[0]) === 24, "Node 24 required");
  process.umask(0o077);
  const descriptor = Number(process.env.MESHWAY_MANAGED_LOCK_FD);
  assert(Number.isInteger(descriptor) && descriptor >= 3, "use run.py to acquire the shared performance lock");
  const lock = process.env.MESHWAY_PERF_LOCK;
  assert(lock, "performance lock path missing");
  const inherited = fstatSync(descriptor);
  const onDisk = await lstat(lock);
  assert(inherited.isFile() && onDisk.isFile() && inherited.dev === onDisk.dev && inherited.ino === onDisk.ino
    && (inherited.mode & 0o077) === 0 && inherited.uid === process.getuid?.(), "invalid inherited performance lock");

  const insideWorkspace = relative(WORKSPACE, opts.parent);
  assert(insideWorkspace.startsWith("..") || isAbsolute(insideWorkspace), "artifacts must be outside the workspace");
  const parentStat = await lstat(opts.parent);
  assert(parentStat.isDirectory() && !parentStat.isSymbolicLink(), "artifact parent must be an existing directory");
  const runId = `${Date.now().toString(36)}-${randomBytes(4).toString("hex")}`;
  const prefix = `meshway-managedtest-${runId}`;
  const artifactDir = await mkdtemp(join(opts.parent, `${prefix}-artifacts-`));
  await chmod(artifactDir, 0o700);
  const artifact: Artifact = { schema_version: 1, run_id: runId, started_at: new Date().toISOString(), status: "running", phase: "preflight", evidence: [], cleanup_errors: [] };
  const image = opts.image;
  const label = `io.meshway.managed-test.run=${runId}`;
  const ownedContainers: string[] = [];
  const ownedNetworks: string[] = [];
  let secretDir: string | undefined;
  let interrupted = false;
  let cleaning = false;
  let collectFailure: (() => Promise<void>) | undefined;
  const names: ByRole<string> = { NYC: `${prefix}-nyc`, LAX: `${prefix}-lax`, SHA: `${prefix}-sha` };
  const onSignal = () => { interrupted = true; };
  process.on("SIGINT", onSignal);
  process.on("SIGTERM", onSignal);
  const docker = (args: string[], timeout = 30_000, input?: string) => {
    if (interrupted && !cleaning) throw new Error("interrupted");
    return command("docker", args, { timeout, ...(input === undefined ? {} : { input }), cwd: WORKSPACE });
  };
  const helper = async (role: Role, action: string, data: object = {}) =>
    parse(await docker(["exec", "-i", names[role], "python3", "/opt/meshway-managed/container.py", action], 40_000, JSON.stringify(data)));
  const socketSnapshot = async (): Promise<ByRole<SocketObservation>> => {
    const [NYC, LAX, SHA] = await Promise.all(ROLES.map(async (role) => sockets(await helper(role, "sockets"))));
    assert(NYC && LAX && SHA);
    return { NYC, LAX, SHA };
  };
  const stage = (name: string) => {
    artifact.phase = name;
    delete artifact.check;
    console.log(`[managed-test] ${name}`);
  };
  const saveArtifact = () => writeFile(join(artifactDir, "result.json"), `${JSON.stringify(artifact, null, 2)}\n`, { mode: 0o600 });
  const until = async <T>(description: string, test: () => Promise<T | false>, timeout = 70_000): Promise<T> => {
    artifact.check = description;
    const deadline = Date.now() + timeout;
    do {
      assert(!interrupted, "interrupted");
      const result = await test();
      if (result !== false) return result;
      await delay(1_000);
    } while (Date.now() < deadline);
    throw new Error(`deadline exceeded: ${description}`);
  };
  try {
    stage("native image");
    const engineArch = normalizeArchitecture(await docker(["info", "--format", "{{.Architecture}}"]));
    const hostArch = normalizeArchitecture(arch());
    assert.equal(engineArch, hostArch, "Docker must use the host's native architecture");
    const imageArch = normalizeArchitecture(await docker(["image", "inspect", image, "--format", "{{.Architecture}}"]));
    assert.equal(imageArch, engineArch, "emulated images are not accepted");
    const imageId = (await docker(["image", "inspect", image, "--format", "{{.Id}}"])).trim();
    assert(/^sha256:[0-9a-f]{64}$/.test(imageId), "expected resolved image identity");
    artifact.image = { reference: image, id: imageId };
    artifact.architecture = { host: hostArch, engine: engineArch, image: imageArch };

    stage("isolated underlay");
    const netA = `${prefix}-a`;
    const netB = `${prefix}-b`;
    const controlNetwork = `${prefix}-controller-api`;
    for (const network of [netA, netB, controlNetwork]) {
      ownedNetworks.push(network);
      await docker(["network", "create", ...(network === controlNetwork ? [] : ["--internal"]), "--label", label, network]);
    }
    for (const role of ROLES) {
      ownedContainers.push(names[role]);
      await docker([
        "run", "--detach", "--init", "--name", names[role], "--hostname", role.toLowerCase(),
        "--label", label, "--label", `io.meshway.managed-test.role=${role}`,
        "--network", role === "LAX" ? controlNetwork : role === "SHA" ? netB : netA,
        ...(role === "LAX" ? ["--publish", "127.0.0.1::8420"] : []),
        "--cap-drop", "ALL", "--cap-add", "NET_ADMIN", "--security-opt", "no-new-privileges", "--read-only",
        "--tmpfs", "/private:rw,nosuid,nodev,noexec,mode=0700,size=64m",
        "--tmpfs", "/tmp:rw,nosuid,nodev,noexec,mode=0700,size=8m",
        "--sysctl", "net.ipv4.ip_forward=0", "--sysctl", "net.ipv6.conf.all.forwarding=0",
        "--cpus", "1", "--memory", "256m", "--memory-swap", "256m", "--pids-limit", "64", imageId,
      ]);
    }
    await docker(["network", "connect", "--alias", "lax.mesh.test", netA, names.LAX]);
    await docker(["network", "connect", "--alias", "lax.mesh.test", netB, names.LAX]);
    const networks: ByRole<string[]> = { NYC: [], LAX: [], SHA: [] };
    const addresses: ByRole<string> = { NYC: "", LAX: "", SHA: "" };
    const dns: ByRole<string[]> = { NYC: [], LAX: [], SHA: [] };
    const laxAddresses: string[] = [];
    for (const role of ROLES) {
      const inspected = parse(await docker(["inspect", names[role], "--format", "{{json .NetworkSettings.Networks}}"]));
      networks[role] = Object.keys(inspected).sort();
      const entry = record(inspected[role === "SHA" ? netB : netA]);
      addresses[role] = text(entry.IPAddress);
      if (role === "LAX") laxAddresses.push(...[netA, netB].map((network) => text(record(inspected[network]).IPAddress)));
      dns[role] = array((await helper(role, "resolve")).addresses).map(text);
      await helper(role, "underlay-start");
    }
    assert.deepEqual(networks.NYC, [netA]);
    assert.deepEqual(networks.SHA, [netB]);
    assert.deepEqual(networks.LAX, [netA, netB, controlNetwork].sort());
    for (const role of ROLES) {
      assert(dns[role].length > 0 && dns[role].every((address) => laxAddresses.includes(address)), "DNS must resolve to this fixture's LAX");
      await until("local underlay canary", async () => boolean((await helper(role, "underlay-probe", { address: "127.0.0.1" })).reachable), 8_000);
    }
    assert(!boolean((await helper("NYC", "underlay-probe", { address: addresses.SHA })).reachable), "NYC must not reach SHA directly");
    assert(!boolean((await helper("SHA", "underlay-probe", { address: addresses.NYC })).reachable), "SHA must not reach NYC directly");
    for (const role of ["NYC", "SHA"] as const) {
      assert(boolean((await helper(role, "underlay-probe", { address: dns[role][0] })).reachable), "each side must reach LAX");
    }
    const underlaySockets = await socketSnapshot();
    for (const local of Object.values(underlaySockets)) {
      assert(!local.ipv4_forwarding && !local.ipv6_forwarding, "kernel forwarding must remain disabled");
      assert(!local.interfaces.some((name) => /^(tun|tap|mw)/.test(name)), "unexpected tunnel interface");
    }
    artifact.topology = { networks, addresses, dns, controller_api_network: controlNetwork };
    artifact.evidence.push({ name: "underlay-isolation", at: new Date().toISOString(), sockets: underlaySockets, notes: ["Both direct cross-network TCP canary attempts failed; both sides reached LAX. No NET_ADMIN, TUN, or IP forwarding."] });

    stage("ephemeral verified TLS profiles");
    secretDir = await mkdtemp(join(opts.parent, `${prefix}-secrets-`));
    await chmod(secretDir, 0o700);
    const certificateDir = secretDir;
    const openssl = (args: string[]) => command("openssl", args, { cwd: certificateDir, timeout: 30_000 });
    await writeFile(join(secretDir, "ca.cnf"), "[req]\ndistinguished_name=dn\nx509_extensions=ca\nprompt=no\n[dn]\nCN=Meshway managed isolated test CA\n[ca]\nbasicConstraints=critical,CA:TRUE\nkeyUsage=critical,keyCertSign,cRLSign\n", { mode: 0o600 });
    await writeFile(join(secretDir, "server.ext"), "basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\nsubjectAltName=DNS:lax.mesh.test\n", { mode: 0o600 });
    await openssl(["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "2", "-config", "ca.cnf", "-keyout", "ca-key.pem", "-out", "ca.pem"]);
    await openssl(["req", "-new", "-newkey", "rsa:2048", "-nodes", "-subj", "/CN=lax.mesh.test", "-keyout", "lax-key.pem", "-out", "lax.csr"]);
    await openssl(["x509", "-req", "-in", "lax.csr", "-CA", "ca.pem", "-CAkey", "ca-key.pem", "-CAcreateserial", "-days", "2", "-extfile", "server.ext", "-out", "lax-cert.pem"]);
    const uuid = randomUUID();
    const emptyProfiles = JSON.stringify({ version: 1, profiles: [] });
    const clientProfiles = JSON.stringify({ version: 1, profiles: [{ id: "lax-vless-client", role: "client", uuid, ca_certificate: "ca.pem", server_name: "lax.mesh.test" }] });
    const serverProfiles = JSON.stringify({ version: 1, profiles: [{ id: "lax-vless-server", role: "server", uuid, certificate: "lax-cert.pem", private_key: "lax-key.pem" }] });
    await helper("NYC", "install-files", { files: [{ name: "profiles.json", content: emptyProfiles }] });
    await helper("LAX", "install-files", { files: [
      { name: "profiles.json", content: serverProfiles },
      { name: "lax-cert.pem", content: await readFile(join(secretDir, "lax-cert.pem"), "utf8") },
      { name: "lax-key.pem", content: await readFile(join(secretDir, "lax-key.pem"), "utf8") },
    ] });
    await helper("SHA", "install-files", { files: [
      { name: "profiles.json", content: clientProfiles }, { name: "ca.pem", content: await readFile(join(secretDir, "ca.pem"), "utf8") },
    ] });
    await rm(secretDir, { recursive: true, force: true });
    secretDir = undefined;

    stage("real controller and enrollment");
    artifact.check = "controller process start";
    await helper("LAX", "controller-start");
    artifact.check = "loopback API publication";
    const port = (await docker(["port", names.LAX, "8420/tcp"])).trim();
    assert(/^127\.0\.0\.1:\d+$/.test(port), "only one ephemeral loopback controller publish is allowed");
    const baseUrl = `http://${port}`;
    const adminToken = await until("controller private initialization", async () => {
      try { return text((await helper("LAX", "admin-token")).token); } catch { return false; }
    }, 15_000);
    const controllerNetworks = await until("controller HTTP readiness", async () => {
      try { return await listNetworks(baseUrl, adminToken); } catch { return false; }
    }, 20_000);
    const networkId = fixtureNetworkId(controllerNetworks);
    collectFailure = async () => {
      const evidence: Evidence = { name: "failure-observations", at: new Date().toISOString() };
      try { evidence.connections = await getConnections(baseUrl, adminToken, networkId); } catch { /* API may be deliberately offline. */ }
      try { evidence.runtime = await getConnectionRuntimes(baseUrl, adminToken, networkId); } catch { /* Do not invent an observation. */ }
      try { evidence.sockets = await socketSnapshot(); } catch { /* An interrupted Docker engine may be unavailable. */ }
      artifact.evidence.push(evidence);
    };
    const ids: ByRole<string> = { NYC: "", LAX: "", SHA: "" };
    const bearers: ByRole<string> = { NYC: "", LAX: "", SHA: "" };
    for (const role of ROLES) {
      const grant = await createEnrollmentToken(baseUrl, adminToken, networkId, { expires_in_seconds: 300 });
      ids[role] = text((await helper(role, "enroll", { name: role, token: grant.token })).node_id);
      assert(/^[0-9a-f]{64}$/.test(ids[role]), "expected a real full-length registered node ID");
      bearers[role] = text((await helper(role, "node-bearer")).token);
      await helper(role, "agent-start", { lease_seconds: opts.lease });
    }
    assert.equal(new Set(Object.values(ids)).size, 3);
    artifact.node_ids = ids;
    const inventory = await until("all real identities and capabilities published", async () => {
      const current = await getConnections(baseUrl, adminToken, networkId);
      return ROLES.every((role) => current.nodes.some((node) => node.node.id === ids[role]
        && node.node.revoked_at === null && node.public_key !== null
        && node.capabilities.some((cap) => cap.transport === (role === "NYC" ? "native_udp" : "vless")
          && (role === "LAX" ? cap.can_accept : cap.can_dial)))) ? current : false;
    });
    assert.equal(inventory.nodes.length, 3);
    assert.deepEqual(inventory.configuration, { listeners: [], candidates: [] });
    const policy = await getNetworkPolicy(baseUrl, adminToken, networkId);
    assert.equal(policy.policy.connections[0]?.action, "allow");
    assert.equal(policy.policy.mesh.length, 0);

    const plan: ConnectionPlan = {
      listeners: [
        { id: "managed-lax-udp", node_id: ids.LAX, transport: "native_udp", bind: "0.0.0.0:41000", advertised_endpoints: [{ host: "lax.mesh.test", port: 41000 }], credential_ref: null },
        { id: "managed-lax-vless", node_id: ids.LAX, transport: "vless", bind: "0.0.0.0:41001", advertised_endpoints: [{ host: "lax.mesh.test", port: 41001 }], credential_ref: "lax-vless-server" },
      ],
      candidates: [
        { id: "managed-nyc-udp", dialer_node_id: ids.NYC, listener_id: "managed-lax-udp", endpoint: { host: "lax.mesh.test", port: 41000 }, priority: 0, credential_ref: null },
        { id: "managed-sha-vless", dialer_node_id: ids.SHA, listener_id: "managed-lax-vless", endpoint: { host: "lax.mesh.test", port: 41001 }, priority: 0, credential_ref: "lax-vless-client" },
      ],
    };
    const savePlan = async (configuration: ConnectionPlan) => {
      const before = await getConnections(baseUrl, adminToken, networkId);
      const saved = await saveConnections(baseUrl, adminToken, networkId, { expected_revision: before.revision, configuration });
      assert(saved.valid, "controller rejected the test's connection plan");
      assert.deepEqual(saved.diagnostics, []);
      return saved;
    };
    const scoped = async (): Promise<ByRole<AgentConnectionsSnapshot>> => {
      const [NYC, LAX, SHA] = await Promise.all(ROLES.map((role) => getAgentConnectionsOperation(baseUrl, bearers[role])));
      assert(NYC && LAX && SHA);
      return { NYC, LAX, SHA };
    };
    const capture = async (name: string, notes?: string[]) => {
      const evidence: Evidence = { name, at: new Date().toISOString(), connections: await getConnections(baseUrl, adminToken, networkId), runtime: await getConnectionRuntimes(baseUrl, adminToken, networkId), sockets: await socketSnapshot(), ...(notes === undefined ? {} : { notes }) };
      artifact.evidence.push(evidence);
      await saveArtifact();
      return evidence;
    };
    const graph = async (expected: ByRole<AgentConnectionsSnapshot>) => until("both authenticated neighbor probe directions", async () => {
      const observations = await getConnectionRuntimes(baseUrl, adminToken, networkId);
      const NYC = observations.nodes.find((node) => node.node_id === ids.NYC);
      const LAX = observations.nodes.find((node) => node.node_id === ids.LAX);
      const SHA = observations.nodes.find((node) => node.node_id === ids.SHA);
      if (!NYC || !LAX || !SHA) return false;
      if (!applied(NYC, expected.NYC) || !applied(LAX, expected.LAX) || !applied(SHA, expected.SHA)) return false;
      if (NYC.peers.length !== 1 || LAX.peers.length !== 2 || SHA.peers.length !== 1) return false;
      if (!probed(NYC, ids.LAX, ids.NYC, "native_udp", "managed-lax-udp", "managed-nyc-udp")
        || !probed(LAX, ids.NYC, ids.NYC, "native_udp", "managed-lax-udp", "managed-nyc-udp")
        || !probed(SHA, ids.LAX, ids.SHA, "vless", "managed-lax-vless", "managed-sha-vless")
        || !probed(LAX, ids.SHA, ids.SHA, "vless", "managed-lax-vless", "managed-sha-vless")) return false;
      return observations;
    });

    stage("SDK CAS plan and DNS reverse dial");
    const saved = await savePlan(plan);
    const expected = await scoped();
    assert.equal(expected.SHA.payload.listeners.length, 0);
    assert.equal(expected.SHA.payload.outgoing[0]?.endpoint.host, "lax.mesh.test");
    const firstGraph = await graph(expected);
    const initial = await capture("authenticated-dns-reverse-dial", ["LAX and SHA each report their own fresh authenticated nonce-probe RTT; SHA is the initiator. These are Agent self-observations, not independent controller attestation."]);
    assert(initial.sockets && initial.connections);
    assert(!initial.sockets.SHA.listeners.some((entry) => entry.port === 41001), "SHA must not host a VLESS static listener");
    assert(initial.sockets.SHA.established.some((entry) => entry.peer_port === 41001 && laxAddresses.includes(entry.peer_host) && entry.local_port !== 41001), "SHA must own an established outgoing TCP connection to LAX");
    assert(initial.sockets.LAX.established.some((entry) => entry.local_port === 41001 && entry.peer_host === addresses.SHA), "LAX must own the accepted TCP endpoint");
    assert.equal(initial.connections.revision, saved.revision, "runtime reports must not bump configuration revision");
    for (const role of ROLES) {
      const receipt: ConnectionNode | undefined = initial.connections.nodes.find((node) => node.node.id === ids[role]);
      assert.equal(receipt?.acknowledged_revision, expected[role].payload.revision);
      assert.equal(receipt?.acknowledged_digest, expected[role].digest);
    }

    stage("missing private profile fails application");
    await helper("SHA", "install-files", { files: [{ name: "profiles.json", content: emptyProfiles }] });
    await until("missing profile must not claim authenticated resources", async () => {
      const snapshot = await getConnectionRuntimes(baseUrl, adminToken, networkId);
      const sha = snapshot.nodes.find((node) => node.node_id === ids.SHA);
      return sha !== undefined && sha.current && !sha.stale && sha.state === "error" && sha.reason === "profile_unavailable"
        && sha.revision === expected.SHA.payload.revision && sha.digest === expected.SHA.digest
        && sha.listeners.length === 0 && sha.peers.length === 0;
    });
    await until("profile failure closes outgoing VLESS", async () => !(await socketSnapshot()).SHA.established.some((entry) => entry.peer_port === 41001));
    await capture("missing-profile-error");
    await helper("SHA", "install-files", { files: [{ name: "profiles.json", content: clientProfiles }] });
    await graph(expected);
    await capture("profile-restored");

    stage("configuration withdrawal");
    await savePlan({ ...plan, candidates: plan.candidates.filter((candidate) => candidate.dialer_node_id !== ids.SHA) });
    const withdrawn = await scoped();
    assert.equal(withdrawn.SHA.payload.outgoing.length, 0);
    assert.equal(withdrawn.SHA.payload.listeners.length, 0);
    assert(!withdrawn.LAX.payload.incoming.some((approval) => approval.peer_node_id === ids.SHA));
    await until("withdrawn SHA adjacency on both nodes", async () => {
      const observations = await getConnectionRuntimes(baseUrl, adminToken, networkId);
      const sha = observations.nodes.find((node) => node.node_id === ids.SHA);
      const lax = observations.nodes.find((node) => node.node_id === ids.LAX);
      if (!sha || !lax) return false;
      return applied(sha, withdrawn.SHA) && applied(lax, withdrawn.LAX) && sha.peers.length === 0
        && !lax.peers.some((peer) => peer.peer_node_id === ids.SHA);
    });
    await until("withdrawn outgoing TCP closed", async () => !(await socketSnapshot()).SHA.established.some((entry) => entry.peer_port === 41001));
    await capture("configuration-withdrawn", ["SHA has a valid applied empty plan and zero neighbors; LAX no longer has an incoming approval for SHA. Applied does not mean the whole network is connected."]);
    await savePlan(plan);
    const restored = await scoped();
    const preOutage = await graph(restored);
    await capture("configuration-restored");

    stage("controller outage and local freshness lease");
    await helper("LAX", "controller-stop");
    let unavailable = false;
    try { await getConnections(baseUrl, adminToken, networkId); } catch { unavailable = true; }
    assert(unavailable, "controller outage was not effective");
    await delay((opts.lease + 2) * 1_000);
    const expired = await until("fresh-config lease closes all managed sockets", async () => {
      const observations = await socketSnapshot();
      return Object.values(observations).every((local) => local.agent_running
        && local.agent_udp_ports.length === 0
        && !local.listeners.some((entry) => entry.port === 41000 || entry.port === 41001)
        && !local.established.some((entry) => entry.local_port === 41001 || entry.peer_port === 41001)) ? observations : false;
    }, 12_000);
    artifact.evidence.push({ name: "outage-lease-expired", at: new Date().toISOString(), sockets: expired, notes: ["Controller is stopped. Evidence is container-local socket closure, not a fabricated fresh controller report. Agent processes remain running; old admin self-reports cannot certify offline health."] });
    await saveArtifact();

    stage("controller restart and fresh recovery");
    await helper("LAX", "controller-start");
    await until("restarted controller HTTP", async () => { try { return await getConnections(baseUrl, adminToken, networkId); } catch { return false; } }, 20_000);
    const recovered = await graph(restored);
    for (const role of ROLES) {
      assert.equal(text((await helper(role, "identity")).node_id), ids[role], "recovery must retain the enrolled identity");
      const before = preOutage.nodes.find((node) => node.node_id === ids[role]);
      const after = recovered.nodes.find((node) => node.node_id === ids[role]);
      assert(before && after);
      assert.equal(after.epoch, before.epoch, "controller restart must preserve the running process epoch");
      assert(after.sequence > before.sequence, "recovery needs fresh reports");
    }
    await capture("controller-restart-recovered");

    stage("revocation closes resources and denies node API");
    await revokeNode(baseUrl, adminToken, networkId, { node_id: ids.SHA });
    let denied = false;
    try { await getAgentConnectionsOperation(baseUrl, bearers.SHA); }
    catch (error) { denied = error instanceof ControlApiError && error.status === 403; }
    assert(denied, "revoked node bearer must receive HTTP 403 through the generated operation");
    await until("revoked agent process and connection exit", async () => {
      const sha = sockets(await helper("SHA", "sockets"));
      return !sha.agent_running && !sha.established.some((entry) => entry.peer_port === 41001);
    });
    const revoked = await getConnectionRuntimes(baseUrl, adminToken, networkId);
    const sha = revoked.nodes.find((node) => node.node_id === ids.SHA);
    assert(sha && sha.state === "withdrawn" && !sha.current && sha.stale && sha.listeners.length === 0
      && sha.peers.length === 0 && sha.reported_at === null, "revoked runtime must not retain fresh claims");
    await capture("revoked", ["Generated node configuration operation returned HTTP 403. No harness-originated begin/report calls were sent."]);
    assert(firstGraph.nodes.length === 3);
    artifact.status = "passed";
    artifact.phase = "complete";
    delete artifact.check;
  } catch {
    // Stage names and structured public evidence suffice to identify the failed
    // check; arbitrary thrown values may contain private process/protocol input.
    artifact.status = "failed";
    console.error(`[managed-test] failed during ${artifact.phase}${artifact.check ? ` / ${artifact.check}` : ""}; private exception details suppressed`);
    process.exitCode = 1;
    if (!interrupted && collectFailure) await collectFailure();
  } finally {
    cleaning = true;
    const cleanup = async (name: string, action: () => Promise<unknown>) => {
      try { await action(); } catch { artifact.cleanup_errors.push(name); process.exitCode = 1; }
    };
    // Names include an unguessable run suffix; also prove the ownership label
    // before removing anything. Never prune or match unrelated resources.
    for (const name of ownedContainers.reverse()) await cleanup(`container:${name}`, async () => {
      const owner = (await docker(["inspect", name, "--format", '{{index .Config.Labels "io.meshway.managed-test.run"}}'])).trim();
      assert.equal(owner, runId);
      await docker(["rm", "--force", name]);
    });
    for (const name of ownedNetworks.reverse()) await cleanup(`network:${name}`, async () => {
      const owner = (await docker(["network", "inspect", name, "--format", '{{index .Labels "io.meshway.managed-test.run"}}'])).trim();
      assert.equal(owner, runId);
      await docker(["network", "rm", name]);
    });
    const remainingSecrets = secretDir;
    if (remainingSecrets !== undefined) await cleanup("private-certificate-directory", () => rm(remainingSecrets, { recursive: true, force: true }));
    if (artifact.cleanup_errors.length > 0) artifact.status = "failed";
    artifact.finished_at = new Date().toISOString();
    await saveArtifact();
    process.off("SIGINT", onSignal);
    process.off("SIGTERM", onSignal);
    console.log(`[managed-test] ${artifact.status}; public artifact: ${join(artifactDir, "result.json")}`);
  }
}

await main().catch(() => {
  console.error("[managed-test] preflight failed: require Node 24, regenerated SDK, run.py lock wrapper, and an existing artifact directory outside the workspace");
  process.exitCode = 1;
});
