import assert from "node:assert/strict";
import test from "node:test";
import { assertRunningOriginals, IMMUTABLE_FILES } from "./demo.ts";
import { currentImageSource, imageInventory, retainImage, retainedImageTag } from "./images.ts";
import { agentArguments, bootstrapArguments, BOOTSTRAP_CAPS } from "./mode.ts";
import { upgradeImage } from "./upgrade.ts";
import type { UpgradeActions, UpgradeJournal, UpgradeSnapshot } from "./upgrade.ts";

const old = `sha256:${"1".repeat(64)}`;
const next = `sha256:${"2".repeat(64)}`;
const foreign = `sha256:${"3".repeat(64)}`;
const owner = { run_id: "a".repeat(32), fingerprint: "b".repeat(64) };
const listed = (pins: string[]) => imageInventory(pins.map((ID) => JSON.stringify({ ID, Repository: "<none>", Tag: "<none>" })).join("\n"));

function originals() {
  return new Map((["nyc", "lax", "sha"] as const).map((role) => [role, {
    Id: `original-${role}`, Image: old, State: { Running: true, StartedAt: "2026-10-01T12:00:00Z" },
    Config: { User: "0:0", Entrypoint: ["setpriv"], Cmd: [...bootstrapArguments("501:20"), "meshway-agent", ...agentArguments("linux-ipv4-tun")],
      Labels: { "io.meshway.demo.project": "managed-neighbors", "io.meshway.demo.run": owner.run_id,
        "io.meshway.demo.data": owner.fingerprint, "io.meshway.demo.resource": role } },
    HostConfig: { ReadonlyRootfs: true, Privileged: false, CapDrop: ["ALL"], CapAdd: [...BOOTSTRAP_CAPS], Init: false,
      SecurityOpt: ["no-new-privileges"], NetworkMode: "meshway-demo-mgmt",
      Devices: [{ PathOnHost: "/dev/net/tun", PathInContainer: "/dev/net/tun", CgroupPermissions: "rw" }],
      Sysctls: { "net.ipv4.ip_forward": "0", "net.ipv6.conf.all.forwarding": "0", "net.ipv4.ping_group_range": "0 2147483647", "net.ipv6.conf.default.disable_ipv6": "1" },
      LogConfig: { Type: "local", Config: { "max-size": "1m", "max-file": "1", compress: "false" } } },
    Mounts: [{ Type: "bind", Source: `/private/demo/${role}`, Destination: "/state", RW: true }],
  }]));
}

test("only a successful full-ID listing establishes absence; target always needs an artifact", () => {
  const reference = retainedImageTag(owner.fingerprint, next);
  const repository = reference.split(":")[0]; const tag = reference.split(":")[1];
  const tagged = JSON.stringify({ ID: next, Repository: repository, Tag: tag });
  assert.equal(imageInventory(tagged).tags.get(reference), next);
  assert(imageInventory(`${tagged}\n${JSON.stringify({ ID: foreign, Repository: repository, Tag: tag })}`).ambiguousTags.has(reference));
  const currentMissing = listed([next]);
  assert.equal(currentImageSource(old, next, "upgrade-image", currentMissing), "running-cohort");
  assert.equal(currentImageSource(old, next, "upgrade-image", listed([old, next])), "artifact");
  for (const action of ["start", "rollback-image", "enable-dataplane", "disable-dataplane"])
    assert.throws(() => currentImageSource(old, next, action, currentMissing), /explicit forward/);
  assert.throws(() => currentImageSource(old, old, "upgrade-image", currentMissing), /target image artifact/);
  assert.throws(() => currentImageSource(old, next, "upgrade-image", listed([old])), /target image artifact/);
  assert.throws(() => currentImageSource(old, next, "upgrade-image", imageInventory("")), /target image artifact/);
  for (const bad of ["daemon connection error", '{"ID":"123abc","Repository":"x","Tag":"v1"}', '{"ID":null}', '{}'])
    assert.throws(() => imageInventory(bad));
});

test("missing-artifact fallback requires ALL three exact-owned running originals with unchanged mode security", () => {
  const check = (value: ReturnType<typeof originals>) => assertRunningOriginals(value, owner, old, "/private/demo", "501:20", "linux-ipv4-tun");
  check(originals());
  for (const role of ["nyc", "lax", "sha"] as const) {
    const absent = originals(); absent.delete(role); assert.throws(() => check(absent), /all three/);
    const stopped = originals(); const stoppedContainer = stopped.get(role); assert(stoppedContainer);
    stoppedContainer.State.Running = false; assert.throws(() => check(stopped));
    const different = originals(); const container = different.get(role); assert(container);
    container.Image = foreign; assert.throws(() => check(different), /unexpected image/);
    container.Image = old; container.Config.Labels["io.meshway.demo.run"] = "foreign"; assert.throws(() => check(different), /ownership/);
  }
  for (const mutate of [
    (value: ReturnType<typeof originals>) => { value.get("nyc")!.HostConfig.CapAdd.push("NET_RAW"); },
    (value: ReturnType<typeof originals>) => { value.get("lax")!.HostConfig.ReadonlyRootfs = false; },
    (value: ReturnType<typeof originals>) => { value.get("sha")!.Mounts[0]!.Source = "/private/foreign"; },
    (value: ReturnType<typeof originals>) => { value.get("nyc")!.Config.Cmd = ["sleep", "infinity"]; },
    (value: ReturnType<typeof originals>) => { value.get("lax")!.HostConfig.Devices = []; },
  ]) { const values = originals(); mutate(values); assert.throws(() => check(values)); }
});

function retention() {
  const inventory = listed([old, next, foreign]); const events: string[] = [];
  const actions = {
    inventory: async () => { events.push("list"); return inventory; },
    tag: async (pin: string, reference: string) => { assert(!inventory.tags.has(reference)); inventory.tags.set(reference, pin); events.push(`retain:${pin}`); },
    inspect: async (reference: string) => { events.push("inspect-retained"); return inventory.tags.get(reference) ?? ""; },
  };
  return { inventory, actions, events };
}

test("dedicated retention tags are deterministic, collision checked, idempotent and keep both pins", async () => {
  const f = retention();
  await retainImage(owner.fingerprint, next, f.actions);
  await retainImage(owner.fingerprint, old, f.actions);
  assert.equal(f.inventory.tags.get(retainedImageTag(owner.fingerprint, next)), next);
  assert.equal(f.inventory.tags.get(retainedImageTag(owner.fingerprint, old)), old);
  assert.equal(retainedImageTag(owner.fingerprint, next), `meshway-demo-image-${owner.fingerprint}:${"2".repeat(64)}`);
  f.events.length = 0;
  await retainImage(owner.fingerprint, next, f.actions);
  assert.deepEqual(f.events, ["list", "inspect-retained"]);
  f.inventory.tags.set(retainedImageTag(owner.fingerprint, next), foreign);
  f.events.length = 0;
  await assert.rejects(retainImage(owner.fingerprint, next, f.actions), /different image/);
  assert.deepEqual(f.events, ["list"]);
  assert.equal(f.inventory.tags.get(retainedImageTag(owner.fingerprint, next)), foreign);
});

test("listing, tagging and retention-inspect failures never become a missing-image fallback", async () => {
  const f = retention();
  await assert.rejects(retainImage(owner.fingerprint, next, { ...f.actions, inventory: async () => { throw Error("engine unavailable"); } }), /engine unavailable/);
  assert.deepEqual(f.events, []);
  await assert.rejects(retainImage(owner.fingerprint, next, { ...f.actions, tag: async () => { throw Error("tag failed"); } }), /tag failed/);
  assert.equal(f.inventory.tags.size, 0);
  await assert.rejects(retainImage(owner.fingerprint, next, { ...f.actions, inspect: async () => foreign }), /pin verification/);
});

test("unrelated multi-platform tags do not block an owned upgrade, but ambiguous retention tags do", async () => {
  const f = retention();
  f.inventory.ambiguousTags.add("unrelated:latest");
  await retainImage(owner.fingerprint, next, f.actions);
  f.inventory.ambiguousTags.add(retainedImageTag(owner.fingerprint, next));
  f.events.length = 0;
  await assert.rejects(retainImage(owner.fingerprint, next, f.actions), /ambiguous/);
  assert.deepEqual(f.events, ["list"]);
});

test("forward migration retains target before journal/stop, preserves original pin, and collisions abort before stop", async () => {
  const original: UpgradeSnapshot = { image_id: old, shell: "readonly", binding: "4".repeat(64), plan_revision: 19, plan_digest: "5".repeat(64),
    files: Object.fromEntries(IMMUTABLE_FILES.map((name) => [name, "6".repeat(64)])) };
  for (const collision of [false, true]) {
    const f = retention(); f.inventory.ids.delete(old);
    if (collision) f.inventory.tags.set(retainedImageTag(owner.fingerprint, next), foreign);
    const current = structuredClone(original); const journals: UpgradeJournal[] = [];
    const actions: UpgradeActions = {
      snapshot: async () => structuredClone(current),
      preflight: async (target) => {
        assert.equal(currentImageSource(old, target, "upgrade-image", await f.actions.inventory()), "running-cohort");
        assertRunningOriginals(originals(), owner, old, "/private/demo", "501:20", "linux-ipv4-tun");
        f.events.push("old-cli-private-copy", "target-native-isolated-cli-private-copy");
        await retainImage(owner.fingerprint, target, f.actions);
      },
      saveJournal: async (journal) => { journals.push(structuredClone(journal)); f.events.push("journal"); },
      stop: async () => { f.events.push("stop"); },
      epochs: async () => ({ nyc: 7, lax: 7, sha: 7 }),
      replacementsRunning: async () => false,
      pin: async (pin, shell) => { current.image_id = pin; current.shell = shell; },
      replaceContainers: async () => {}, start: async () => {}, healthy: async () => {}, finish: async () => {},
    };
    if (collision) {
      await assert.rejects(upgradeImage(next, null, actions), /different image/);
      assert(!f.events.includes("stop")); assert.equal(journals.length, 0); assert.deepEqual(current, original);
    } else {
      assert.equal(await upgradeImage(next, null, actions), "upgraded");
      assert(f.events.indexOf(`retain:${next}`) < f.events.indexOf("journal"));
      assert(f.events.indexOf("journal") < f.events.indexOf("stop"));
      assert(journals.every((journal) => journal.old_image_id === old && journal.original.image_id === old));
      assert.deepEqual({ ...current, image_id: old }, original);
      assert.throws(() => currentImageSource(next, old, "rollback-image", f.inventory), /target image artifact/);
    }
  }
});
