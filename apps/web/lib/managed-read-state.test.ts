import assert from "node:assert/strict"
import { test } from "node:test"

import type { ManagedListenerSettings, ManagedListenersSnapshot } from "@meshway/control-client"
import { zManagedListenerSettings, zManagedListenersSnapshot } from "@meshway/control-client/schemas"

import { draftBaseline, draftChanged, type ManagedListenerDraft } from "./managed-listener-draft-registry.ts"
import { fromSettings, toSettings } from "./managed-listener-model.ts"
import { initialManagedRead, managedFreshness, recordManagedFailure, recordManagedSuccess } from "./managed-read-state.ts"

const NODE = "a".repeat(64)

function settings(port: number): ManagedListenerSettings {
  return zManagedListenerSettings.parse({
    id: "edge", node_id: NODE, enabled: true, transport: "vless", bind_address: "0.0.0.0", bind_port: port,
    advertised_endpoints: [], vless: { stream: "tcp", server_name: "meshway.local", websocket_path: null, websocket_host: null },
  })
}

function snapshot(revision: number, port = 443): ManagedListenersSnapshot {
  return zManagedListenersSnapshot.parse({
    revision,
    listeners: [{ settings: settings(port), material_version: 1, certificate_sha256: "f".repeat(64), compiled_capability_available: true }],
  })
}

test("a failed managed read after a good one is a dated cache, and a retry restores current", () => {
  let state = initialManagedRead({ ok: true, data: snapshot(3), fetchedAt: 100 })
  assert.equal(managedFreshness(state), "current")
  state = recordManagedFailure(state, "HTTP 500", 200)
  assert.equal(managedFreshness(state), "cached")
  assert.equal(state.snapshot?.revision, 3, "the last snapshot stays on screen")
  assert.equal(state.snapshotAt, 100, "with its own read time, not the failure's")
  assert.equal(state.status.error, "HTTP 500")
  state = recordManagedSuccess(state, snapshot(3), 300)
  assert.equal(managedFreshness(state), "current")
  assert.equal(state.snapshotAt, 300)
})

test("out-of-order completions: an older success never clears a newer failure or rolls the snapshot back", () => {
  let state = initialManagedRead({ ok: true, data: snapshot(3), fetchedAt: 100 })
  state = recordManagedSuccess(state, snapshot(5), 250)
  state = recordManagedFailure(state, "disconnected", 300)
  state = recordManagedSuccess(state, snapshot(4), 200)
  assert.equal(managedFreshness(state), "cached", "the late success is older than the failure")
  assert.equal(state.snapshot?.revision, 5, "a lower revision never replaces a newer snapshot")
  // A late failure older than the last success does not flag current data.
  state = recordManagedSuccess(state, snapshot(5), 400)
  state = recordManagedFailure(state, "late", 350)
  assert.equal(managedFreshness(state), "current")
})

test("without any snapshot the read state is failed, not cached", () => {
  const state = initialManagedRead({ ok: false, error: "HTTP 500", fetchedAt: 100 })
  assert.equal(managedFreshness(state), "failed")
  assert.equal(state.snapshot, null)
})

test("after an explicit rebase the form is compared with the newly adopted saved version", () => {
  const opened = settings(443)
  const draft: ManagedListenerDraft = {
    key: "k", kind: "edit", nodeId: NODE, baseRevision: 3, original: opened,
    initial: fromSettings(opened), form: { ...fromSettings(opened), bindPort: "8443" }, idFollows: false, open: true,
  }
  assert.equal(draftChanged(draft), true)
  // A competing writer saved 9443; the administrator rebases explicitly.
  const rebased: ManagedListenerDraft = { ...draft, baseRevision: 4, original: settings(9443) }
  assert.equal(draftBaseline(rebased).bindPort, "9443")
  assert.equal(rebased.form.bindPort, "8443", "the typed form is kept")
  const backToOpened = { ...rebased, form: { ...rebased.form, bindPort: "443" } }
  assert.equal(draftChanged(backToOpened), true, "the first-opened value differs from the adopted base and can be saved")
  assert.equal(toSettings(backToOpened.form, NODE).bind_port, 443)
  const matchesBase = { ...rebased, form: { ...rebased.form, bindPort: "9443" } }
  assert.equal(draftChanged(matchesBase), false, "only the adopted saved value means no change")
  // Invalid text is a change and is kept, never coerced.
  const invalid = { ...rebased, form: { ...rebased.form, bindPort: "94x3" } }
  assert.equal(draftChanged(invalid), true)
  assert.equal(invalid.form.bindPort, "94x3")
  // Creation compares with its suggestion.
  const create: ManagedListenerDraft = { ...draft, kind: "create", original: null, form: draft.initial }
  assert.equal(draftChanged(create), false)
})
