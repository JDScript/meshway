import assert from "node:assert/strict";
import { test } from "node:test";
import { randomUUID } from "node:crypto";
import { controller } from "./support/controller.ts";
import {
  register, createNetwork, getConfigurationSource, getConfiguration,
  applyConfiguration, getNetworkState, watchNetwork, logout,
} from "../src/index.ts";

type NetworkStream = ReturnType<typeof watchNetwork>;
async function frame(stream: NetworkStream) {
  const value = await stream.next();
  assert.ok(!value.done && value.value.ok, "expected a validated event frame");
  return value.value.data;
}
async function snapshot(stream: NetworkStream, networkId: string, secret: string) {
  const config = await frame(stream);
  const entities = await frame(stream);
  const state = await frame(stream);
  const ready = await frame(stream);
  assert.equal(config.data.type, "config.snapshot");
  assert.equal(entities.data.type, "entities.snapshot");
  assert.ok(state.data.type === "state.snapshot");
  assert.equal(state.data.state.networkId, networkId);
  assert.deepEqual(state.data.state.nodes, {});
  assert.ok(ready.data.type === "sync.ready");
  assert.equal(ready.id, ready.data.cursor);
  for (const first of [config, entities, state]) {
    assert.equal(first.id, undefined);
    assert.equal(JSON.stringify(first.data).includes(secret), false);
  }
  return ready.data.cursor;
}

test("production SSE synchronizes real configuration, resumes applied cursors and closes revoked sessions", async t => {
  const client = await controller(t);
  const owner = await register(client, { email: "events-sdk@example.test", password: "valid event stream password" });
  assert.ok(owner.ok);
  const user = { token: owner.data.token };
  const created = await createNetwork(client, { requestId: randomUUID(), name: "SSE integration" }, user);
  assert.ok(created.ok);
  const path = { networkId: created.data.network.id };
  const source = await getConfigurationSource(client, path, {}, user);
  assert.ok(source.ok);
  const defaults = source.data.configuration.defaults;
  assert.ok(defaults?.dialers?.vless);
  const secret = "private-header-must-not-enter-event-stream";
  const seeded = await applyConfiguration(client, path, {
    type: "operations", requestId: randomUUID(), expectedRevision: source.data.revision,
    ops: [{ type: "replaceDefaults", defaults: { ...defaults, dialers: { ...defaults.dialers,
      vless: { ...defaults.dialers.vless, websocket: { headers: { Authorization: secret } } },
    } } }],
  }, user);
  assert.ok(seeded.ok);
  const options = { ...user, maxReconnects: 0, idleTimeoutMs: 2000 };
  const first = watchNetwork(client, path, options);
  t.after(() => first.return(undefined));
  let appliedCursor = await snapshot(first, path.networkId, secret);
  const initial = await getConfigurationSource(client, path, {}, user);
  assert.ok(initial.ok);
  // This local draft is deliberately not part of the submitted change.
  const draft = initial.data.sourceJsonc + "\n// unsaved local draft";
  const change = await applyConfiguration(client, path, {
    type: "document", requestId: randomUUID(), expectedRevision: initial.data.revision,
    sourceJsonc: initial.data.sourceJsonc + "\n// committed source annotation",
  }, user);
  assert.ok(change.ok);
  let seen = false;
  for (let index = 0; index < 12; index++) {
    const event = await frame(first);
    assert.equal(JSON.stringify(event.data).includes(secret), false);
    if (event.data.type !== "configuration.changed") continue;
    assert.equal(event.data.previousRevision, initial.data.revision);
    assert.equal(event.data.revision, change.data.revision);
    assert.equal(event.data.sourceChanged, true);
    const refreshed = await getConfiguration(client, path, user);
    assert.ok(refreshed.ok && refreshed.data.revision === change.data.revision);
    assert.ok(event.id);
    appliedCursor = event.id;
    seen = true;
    break;
  }
  assert.equal(seen, true);
  assert.ok(draft.endsWith("// unsaved local draft"));
  await first.return(undefined);
  const second = watchNetwork(client, path, { ...options, resumeCursor: () => appliedCursor });
  t.after(() => second.return(undefined));
  // Start the reconnect before the next commit; both orders are legal replay.
  const next = second.next();
  const changedAgain = await applyConfiguration(client, path, {
    type: "operations", requestId: randomUUID(), expectedRevision: change.data.revision,
    ops: [{ type: "replaceAccess", access: [] }],
  }, user);
  assert.ok(changedAgain.ok);
  let pending = await next;
  seen = false;
  for (let index = 0; index < 12; index++) {
    assert.ok(!pending.done && pending.value.ok);
    const event = pending.value.data;
    assert.equal(JSON.stringify(event.data).includes(secret), false);
    if (event.data.type === "configuration.changed") {
      assert.equal(event.data.revision, changedAgain.data.revision, "already applied notice must not replay");
      seen = true;
      break;
    }
    pending = await second.next();
  }
  assert.equal(seen, true);
  const state = await getNetworkState(client, path, user);
  assert.ok(state.ok);
  assert.equal(state.data.configurationRevision, changedAgain.data.revision);
  assert.deepEqual(state.data.nodes, {});
  await second.return(undefined);
  const final = watchNetwork(client, path, options);
  t.after(() => final.return(undefined));
  await snapshot(final, path.networkId, secret);
  const waiting = final.next();
  assert.ok((await logout(client, user)).ok);
  const closed = await waiting;
  assert.ok(closed.done || !closed.value.ok || closed.value.data.data.type === "stream.closed");
});
