import assert from "node:assert/strict";
import { chmod, mkdir, mkdtemp, rm, symlink, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { assertDemoContainer, demoEpochFloor, demoHealthy, demoPlan, privateControlSocket } from "./demo.ts";
import type { ConnectionRuntimesSnapshot, ConnectionsSnapshot } from "../../packages/control-client/src/index.ts";
import { nativeArchitecture, upgradeImage, upgradeJournal } from "./upgrade.ts";
import type { UpgradeActions, UpgradeJournal, UpgradeSnapshot } from "./upgrade.ts";
import { agentArguments } from "./mode.ts";

const old = `sha256:${"1".repeat(64)}`;
const next = `sha256:${"2".repeat(64)}`;
const foreign = `sha256:${"3".repeat(64)}`;
const original: UpgradeSnapshot = {
  image_id: old, shell: "readonly", binding: "4".repeat(64), plan_revision: 7, plan_digest: "5".repeat(64),
  files: Object.fromEntries(["nyc", "lax", "sha"].flatMap((role) => ["agent.json", "agent.json.connection-key.json", "profiles.json", "cert.pem"].map((name) => [`${role}/${name}`, "6".repeat(64)]))),
};

function fixture() {
  const current = structuredClone(original);
  const disk: { journal: UpgradeJournal | null; containers: string; running: boolean } = { journal: null, containers: old, running: true };
  const events: string[] = [];
  let epoch = 7;
  let crash: string | null = null;
  const event = (name: string) => { events.push(name); if (crash === name) { crash = null; throw new Error("simulated process loss"); } };
  const actions: UpgradeActions = {
    snapshot: async () => structuredClone(current),
    preflight: async (_image, allowed) => { assert(allowed.includes(disk.containers), "foreign image"); event("preflight"); },
    saveJournal: async (journal) => { disk.journal = upgradeJournal(JSON.parse(JSON.stringify(journal))); event(`journal:${journal.phase}`); },
    stop: async () => { disk.running = false; event("stop"); },
    epochs: async () => { event("epochs"); return { nyc: epoch, lax: epoch, sha: epoch }; },
    replacementsRunning: async (image, shell) => disk.containers === image && current.shell === shell && disk.running,
    pin: async (image, shell) => { current.image_id = image; current.shell = shell; event("pin"); },
    replaceContainers: async (image) => { disk.containers = image; event("replace"); },
    start: async () => { disk.running = true; epoch++; event("start"); },
    healthy: async (floor) => { assert(disk.running); if (floor) assert(Object.values(floor).every((value) => epoch > value)); event("healthy"); },
    finish: async () => { disk.journal = null; event("finish"); },
  };
  return { current, disk, events, actions, failAfter: (name: string) => { crash = name; } };
}

test("image upgrade preserves all material/binding/plan bytes; repeating the pinned image is read-only", async () => {
  const f = fixture();
  assert.equal(await upgradeImage(next, null, f.actions), "upgraded");
  assert.deepEqual(f.current, { ...original, image_id: next });
  assert.equal(f.disk.journal, null);
  assert.equal(f.disk.containers, next);
  assert(f.disk.running);
  assert(f.events.indexOf("journal:prepared") < f.events.indexOf("stop"));
  f.events.length = 0;
  assert.equal(await upgradeImage(next, null, f.actions), "unchanged");
  assert.deepEqual(f.events, ["preflight", "healthy"]);
});

test("developer transition journals image and writable target before stop and survives retries", async () => {
  const f = fixture(); f.failAfter("stop");
  await assert.rejects(upgradeImage(next, null, f.actions, "development"), /simulated/);
  assert.equal(f.disk.journal?.old_shell, "readonly");
  assert.equal(f.disk.journal?.new_shell, "development");
  assert.equal(f.disk.journal?.target_shell, "development");
  assert(f.events.indexOf("journal:prepared") < f.events.indexOf("stop"));
  await upgradeImage(next, f.disk.journal, f.actions, "development");
  assert.equal(f.current.shell, "development");
  assert.equal(f.current.image_id, next);
});

test("pending developer transition rollback restores the old image and read-only descriptor", async () => {
  const f = fixture(); f.failAfter("start");
  await assert.rejects(upgradeImage(next, null, f.actions, "development"), /simulated/);
  assert.equal(f.current.shell, "development");
  assert.equal(await upgradeImage(old, f.disk.journal, f.actions, "readonly"), "rolled-back");
  assert.deepEqual(f.current, original);
});

test("every durable transition is retryable after process loss without restoring or replacing identity files", async () => {
  for (const boundary of ["journal:prepared", "stop", "epochs", "journal:restarting", "pin", "replace", "start", "journal:verifying", "healthy", "journal:complete", "finish"]) {
    const f = fixture(); f.failAfter(boundary);
    await assert.rejects(upgradeImage(next, null, f.actions), /simulated/);
    assert.deepEqual({ ...f.current, image_id: old }, original, boundary);
    await upgradeImage(next, f.disk.journal, f.actions);
    assert.equal(f.disk.journal, null, boundary);
    assert.equal(f.current.image_id, next, boundary);
    assert.equal(f.disk.containers, next, boundary);
    assert(f.disk.running, boundary);
    assert.deepEqual({ ...f.current, image_id: old }, original, boundary);
  }
});

test("pending-upgrade rollback changes only the image and is itself recoverable", async () => {
  const f = fixture(); f.failAfter("start");
  await assert.rejects(upgradeImage(next, null, f.actions));
  f.failAfter("pin");
  await assert.rejects(upgradeImage(old, f.disk.journal, f.actions));
  assert.equal(f.disk.journal?.target_image_id, old);
  assert.equal(await upgradeImage(old, f.disk.journal, f.actions), "rolled-back");
  assert.deepEqual(f.current, original);
  assert.equal(f.disk.containers, old);
  assert(f.disk.running);
});

test("ownership, credentials, key fingerprints, binding, image and concurrent plan changes reject before a stop", async () => {
  const foreignImage = fixture(); foreignImage.disk.containers = foreign;
  await assert.rejects(upgradeImage(next, null, foreignImage.actions), /foreign image/);
  assert.equal(foreignImage.disk.journal, null);
  assert(!foreignImage.events.includes("stop"));
  for (const mutate of [
    (value: UpgradeSnapshot) => { value.binding = "a".repeat(64); },
    (value: UpgradeSnapshot) => { value.files["nyc/agent.json"] = "a".repeat(64); },
    (value: UpgradeSnapshot) => { value.files["sha/agent.json.connection-key.json"] = "a".repeat(64); },
    (value: UpgradeSnapshot) => { value.files["lax/profiles.json"] = "a".repeat(64); },
    (value: UpgradeSnapshot) => { value.image_id = foreign; },
    (value: UpgradeSnapshot) => { value.plan_revision++; },
    (value: UpgradeSnapshot) => { value.plan_digest = "a".repeat(64); },
  ]) {
    const f = fixture(); f.failAfter("journal:prepared");
    await assert.rejects(upgradeImage(next, null, f.actions));
    mutate(f.current); f.events.length = 0;
    await assert.rejects(upgradeImage(next, f.disk.journal, f.actions));
    assert.deepEqual(f.events, []);
    assert(f.disk.running);
  }
  const f = fixture(); f.failAfter("journal:prepared");
  await assert.rejects(upgradeImage(next, null, f.actions));
  f.events.length = 0;
  await assert.rejects(upgradeImage(foreign, f.disk.journal, f.actions), /pending upgrade/);
  assert.deepEqual(f.events, []);
});

test("controller edits during preflight or restart are detected, never overwritten", async () => {
  for (const step of ["preflight", "replaceContainers"] as const) {
    const f = fixture();
    if (step === "preflight") {
      const operation = f.actions.preflight;
      f.actions.preflight = async (image, images) => { await operation(image, images); f.current.plan_revision++; };
    } else {
      const operation = f.actions.replaceContainers;
      f.actions.replaceContainers = async (image, shell) => { await operation(image, shell); f.current.plan_revision++; };
    }
    await assert.rejects(upgradeImage(next, null, f.actions), /controller plan changed/);
    assert.equal(f.current.plan_revision, original.plan_revision + 1);
    assert.deepEqual(f.current.files, original.files);
    if (step === "preflight") assert(!f.events.includes("stop"));
  }
});

test("upgrade journal rejects malformed or broadened snapshots", async () => {
  const f = fixture(); f.failAfter("journal:prepared");
  await assert.rejects(upgradeImage(next, null, f.actions));
  assert(f.disk.journal);
  assert.throws(() => upgradeJournal({ ...f.disk.journal, unexpected: true }));
  assert.throws(() => upgradeJournal({ ...f.disk.journal, target_image_id: foreign }));
  assert.throws(() => upgradeJournal({ ...f.disk.journal, original: { ...original, files: {} } }));
  assert.throws(() => upgradeJournal({ ...f.disk.journal, original: { ...original, files: { ...original.files, "../../foreign": "a".repeat(64) } } }));
  assert.throws(() => upgradeJournal({ ...f.disk.journal, phase: "verifying", epoch_floor: null }));
  for (const floor of [{ nyc: 7, lax: 7 }, { nyc: 7, lax: 7, sha: -1 }, { nyc: 7, lax: 7, sha: 0.5 }, { nyc: 7, lax: 7, sha: "7" }, { nyc: 7, lax: 7, sha: 7, foreign: 7 }])
    assert.throws(() => upgradeJournal({ ...f.disk.journal, phase: "verifying", epoch_floor: floor }));
});

/** Actual demo health predicate with otherwise fully healthy recent epoch-7 reports. */
function reportedFixture() {
  const f = fixture(); const now = Date.now();
  const ids = { nyc: "1".repeat(64), lax: "2".repeat(64), sha: "3".repeat(64) };
  const digest = "f".repeat(64);
  const config: ConnectionsSnapshot = {
    revision: 12, valid: true, diagnostics: [], configuration: demoPlan(ids),
    nodes: Object.entries(ids).map(([role, id]) => ({ public_key: "a".repeat(64), capabilities: [],
      acknowledged_at: new Date(now).toISOString(), acknowledged_digest: digest, acknowledged_revision: 12,
      node: { id, name: `demo-${role}`, revoked_at: null, registered_at: new Date(now).toISOString(), last_seen: new Date(now).toISOString(),
        acknowledged_revision: 12, configuration_revision: 12, agent_version: "test", country: null, mesh_ipv4: "100.64.0.1", mesh_ipv6: "fd00::1" } })),
  };
  const runtime: ConnectionRuntimesSnapshot = { nodes: Object.values(ids).map((id) => ({ node_id: id, epoch: 7, sequence: 2,
    revision: 12, digest, state: "applied", reason: "none", reported_at: new Date(now).toISOString(), stale_after_seconds: 60, current: true, stale: false,
    listeners: id === ids.lax ? ["demo-lax-udp", "demo-lax-vless"].map((listener_id) => ({ listener_id, state: "bound", reason: "none" })) : [], peers: [],
  })) };
  for (const [dialer, transport, listener, candidate] of [
    [ids.nyc, "native_udp", "demo-lax-udp", "demo-nyc-to-lax"],
    [ids.sha, "vless", "demo-lax-vless", "demo-sha-to-lax"],
  ] as const) for (const observer of [dialer, ids.lax]) {
    const node = runtime.nodes.find((entry) => entry.node_id === observer); assert(node);
    node.peers.push({ peer_node_id: observer === dialer ? ids.lax : dialer, initiator_node_id: dialer, transport,
      listener_id: listener, candidate_id: observer === dialer ? candidate : null,
      state: "authenticated", reason: "none", probe_fresh: true, probe_rtt_ms: 0 });
  }
  f.actions.epochs = async () => {
    assert.equal(f.disk.running, false, "floor must be captured after the original processes stop");
    f.events.push("epochs"); return demoEpochFloor(runtime, ids);
  };
  f.actions.healthy = async (floor) => {
    assert(f.disk.running);
    f.events.push("healthy");
    assert(demoHealthy(config, runtime, ids, now, floor), "fresh replacement graph deadline exceeded");
  };
  const reports = (nyc: number, lax = nyc, sha = nyc) => {
    for (const node of runtime.nodes) node.epoch = node.node_id === ids.nyc ? nyc : node.node_id === ids.lax ? lax : sha;
  };
  return { ...f, now, ids, config, runtime, reports };
}

test("recent healthy old reports cannot complete an upgrade: journal stays until EVERY replacement epoch advances", async () => {
  const f = reportedFixture();
  assert(demoHealthy(f.config, f.runtime, f.ids, f.now), "old graph is otherwise healthy");
  await assert.rejects(upgradeImage(next, null, f.actions), /replacement graph deadline/);
  assert.equal(f.disk.journal?.phase, "verifying");
  assert.deepEqual(f.disk.journal.epoch_floor, { nyc: 7, lax: 7, sha: 7 });
  assert(!f.events.includes("journal:complete"));
  assert(!f.events.includes("finish"));
  assert.deepEqual({ ...f.current, image_id: old }, original);
  f.reports(8, 8, 7);
  await assert.rejects(upgradeImage(next, f.disk.journal, f.actions), /replacement graph deadline/);
  f.reports(8);
  assert.equal(await upgradeImage(next, f.disk.journal, f.actions), "upgraded");
  assert.equal(f.events.filter((event) => event === "epochs").length, 1, "same running cohort must retain its original floor");
  assert.equal(f.events.filter((event) => event === "start").length, 1);
  assert.equal(f.disk.journal, null);
  assert.deepEqual({ ...f.current, image_id: old }, original);
});

test("crash after starting replacements resumes against persisted floors, even when new reports already arrived", async () => {
  const f = reportedFixture(); f.failAfter("start");
  await assert.rejects(upgradeImage(next, null, f.actions), /simulated process loss/);
  assert.equal(f.disk.journal?.phase, "verifying");
  f.reports(8);
  assert.equal(await upgradeImage(next, f.disk.journal, f.actions), "upgraded");
  assert.equal(f.events.filter((event) => event === "epochs").length, 1);
  assert.equal(f.events.filter((event) => event === "stop").length, 1);
  assert.deepEqual({ ...f.current, image_id: old }, original);
});

test("rollback captures its own post-stop floor and reuses it across verification retries", async () => {
  const f = reportedFixture();
  await assert.rejects(upgradeImage(next, null, f.actions), /replacement graph deadline/);
  f.reports(8);
  await assert.rejects(upgradeImage(old, f.disk.journal, f.actions), /replacement graph deadline/);
  assert.equal(f.disk.journal?.target_image_id, old);
  assert.deepEqual(f.disk.journal.epoch_floor, { nyc: 8, lax: 8, sha: 8 });
  f.reports(9);
  assert.equal(await upgradeImage(old, f.disk.journal, f.actions), "rolled-back");
  assert.equal(f.events.filter((event) => event === "epochs").length, 2);
  assert.equal(f.events.filter((event) => event === "start").length, 2);
  assert.deepEqual(f.current, original);
});

test("a dead replacement cohort requires a new durable floor, not acceptance of its last healthy reports", async () => {
  const f = reportedFixture();
  await assert.rejects(upgradeImage(next, null, f.actions), /replacement graph deadline/);
  f.reports(8); f.disk.running = false;
  await assert.rejects(upgradeImage(next, f.disk.journal, f.actions), /replacement graph deadline/);
  assert.deepEqual(f.disk.journal?.epoch_floor, { nyc: 8, lax: 8, sha: 8 });
  f.reports(9);
  assert.equal(await upgradeImage(next, f.disk.journal, f.actions), "upgraded");
  assert.equal(f.events.filter((event) => event === "epochs").length, 2);
  assert.deepEqual({ ...f.current, image_id: old }, original);
});

test("legacy V1 journal preserves pins/material but cannot reuse an unfenced complete phase", async () => {
  const f = reportedFixture(); f.failAfter("start");
  await assert.rejects(upgradeImage(next, null, f.actions));
  assert(f.disk.journal);
  const { epoch_floor: _floor, old_shell: _oldShell, new_shell: _newShell, target_shell: _targetShell, ...legacy } = f.disk.journal;
  const migrated = upgradeJournal({ ...legacy, version: 1, phase: "complete" });
  assert.equal(migrated.version, 3);
  assert.equal(migrated.phase, "prepared");
  assert.equal(migrated.epoch_floor, null);
  assert.deepEqual(migrated.original, original);
  f.reports(8);
  await assert.rejects(upgradeImage(next, migrated, f.actions), /replacement graph deadline/);
  assert.deepEqual(f.disk.journal?.epoch_floor, { nyc: 8, lax: 8, sha: 8 });
  assert.deepEqual({ ...f.current, image_id: old }, original);
});

test("only native Linux images and engines are accepted; emulation is rejected", () => {
  nativeArchitecture("arm64", "aarch64", "arm64", "linux");
  nativeArchitecture("x64", "x86_64", "amd64", "linux");
  for (const inputs of [["arm64", "arm64", "amd64", "linux"], ["arm64", "amd64", "amd64", "linux"], ["x64", "amd64", "amd64", "windows"], ["riscv64", "riscv64", "riscv64", "linux"]]) {
    assert.throws(() => nativeArchitecture(inputs[0] ?? "", inputs[1] ?? "", inputs[2] ?? "", inputs[3] ?? ""));
  }
});

test("matching Docker labels cannot adopt a foreign image, user or state mount", () => {
  const owner = { run_id: "a".repeat(32), fingerprint: "b".repeat(64) };
  const container = { Image: old, Config: { User: "501:20", Entrypoint: ["meshway-agent"], Cmd: agentArguments("neighbors"), Labels: { "io.meshway.demo.project": "managed-neighbors",
    "io.meshway.demo.run": owner.run_id, "io.meshway.demo.data": owner.fingerprint, "io.meshway.demo.resource": "nyc" } },
  HostConfig: { ReadonlyRootfs: true, Privileged: false, CapDrop: ["ALL"], CapAdd: null, SecurityOpt: ["no-new-privileges"], NetworkMode: "meshway-demo-mgmt",
    Sysctls: { "net.ipv4.ip_forward": "0", "net.ipv6.conf.all.forwarding": "0", "net.ipv4.ping_group_range": "0 2147483647" }, LogConfig: { Type: "none", Config: {} } },
  Mounts: [{ Type: "bind", Source: "/private/demo/nyc", Destination: "/state", RW: true }] };
  const check = (value: typeof container) => assertDemoContainer(value, owner, "nyc", [old, next], "/private/demo/nyc", "501:20");
  check(container);
  for (const mutate of [
    (value: typeof container) => { value.Image = foreign; },
    (value: typeof container) => { value.Config.User = "0:0"; },
    (value: typeof container) => { value.Config.Labels["io.meshway.demo.run"] = "c".repeat(32); },
    (value: typeof container) => { value.HostConfig.Privileged = true; },
    (value: typeof container) => { value.Mounts[0]!.Source = "/private/someone-else"; },
    (value: typeof container) => { value.Mounts.push({ Type: "bind", Source: "/", Destination: "/host", RW: true }); },
  ]) { const value = structuredClone(container); mutate(value); assert.throws(() => check(value)); }
});

test("IPC guard accepts an actual private socket, rejects files/symlinks/public sockets, and never unlinks it", async () => {
  const parent = join(tmpdir(), "opencode");
  await mkdir(parent, { recursive: true });
  const root = await mkdtemp(join(parent, "mwu-"));
  const socket = join(root, "s"); const server = createServer();
  try {
    await new Promise<void>((resolve, reject) => { server.once("error", reject); server.listen(socket, resolve); });
    await chmod(socket, 0o600);
    await privateControlSocket(socket);
    await privateControlSocket(join(root, "missing"));
    await chmod(socket, 0o644);
    await assert.rejects(privateControlSocket(socket), /private Agent-owned/);
    await chmod(socket, 0o600);
    await symlink(socket, join(root, "link"));
    await assert.rejects(privateControlSocket(join(root, "link")));
    await writeFile(join(root, "file"), "", { mode: 0o600 });
    await assert.rejects(privateControlSocket(join(root, "file")));
    await privateControlSocket(socket);
    assert(server.listening);
  } finally {
    if (server.listening) await new Promise<void>((resolve) => server.close(() => resolve()));
    await rm(root, { recursive: true, force: true });
  }
});
