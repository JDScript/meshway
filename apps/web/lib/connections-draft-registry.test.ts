import assert from "node:assert/strict"
import { test } from "node:test"

import type { ConnectionPlan, ConnectionsSnapshot } from "@meshway/control-client"

import { zConnectionsSnapshot } from "../../../packages/control-client/src/generated/zod.gen.ts"
import {
  ConnectionsDraftRegistry,
  type StoredConnectionsDraft,
} from "./connections-draft-registry.ts"
import {
  draftKey,
  fromPlan,
  newEndpointKey,
  type ConnectionsDraft,
  type ListenerDraft,
  type WorkingSheet,
} from "./connections-model.ts"

const plan: ConnectionPlan = {
  listeners: [
    {
      id: "lax-in",
      node_id: "lax",
      transport: "native_udp",
      bind: "0.0.0.0:51820",
      advertised_endpoints: [
        { host: "203.0.113.7", port: 51820 },
        { host: "lax.example.net", port: 51820 },
      ],
      credential_ref: null,
    },
  ],
  candidates: [],
}

function snapshot(revision: number, configuration: ConnectionPlan = plan): ConnectionsSnapshot {
  return zConnectionsSnapshot.parse({
    revision,
    configuration,
    valid: true,
    diagnostics: [],
    nodes: [],
  })
}

function entry(base: ConnectionsSnapshot, draft: ConnectionsDraft, sheet: WorkingSheet | null = null): StoredConnectionsDraft {
  return { base, draft, sheet, nextKey: 0, nextSession: 0 }
}

const savedListener = (): ListenerDraft => {
  const [listener] = fromPlan(plan).listeners
  assert.ok(listener)
  return listener
}

function listenerSheet(form: ListenerDraft): WorkingSheet {
  return { kind: "listener", session: { id: 1, mode: "edit", initial: savedListener() }, form }
}

test("a stale editor cannot clear or overwrite a newer editor's draft", () => {
  const registry = new ConnectionsDraftRegistry()
  const editorA = registry.claim()
  const draftA: ConnectionsDraft = { ...fromPlan(plan), candidates: [] }
  registry.write(editorA, entry(snapshot(5), { ...draftA, listeners: [] }))
  // A starts a reload, then unmounts before it resolves.
  registry.release(editorA)

  const editorB = registry.claim()
  const restored = registry.read()
  assert.ok(restored)
  const draftB: ConnectionsDraft = fromPlan(plan)
  registry.write(editorB, { ...restored, draft: draftB })

  // A's delayed reload completion runs only if it still owns the store.
  assert.equal(registry.owns(editorA), false)
  registry.clear(editorA)
  registry.write(editorA, entry(snapshot(9), { listeners: [], candidates: [] }))

  const kept = registry.read()
  assert.ok(kept)
  assert.equal(kept.base.revision, 5, "B keeps its CAS base")
  assert.equal(draftKey(kept.draft), draftKey(draftB))
  assert.equal(registry.owns(editorB), true)
})

test("unapplied sheet edits keep an entry, and the CAS base, while the plan is clean", () => {
  const registry = new ConnectionsDraftRegistry()
  const editor = registry.claim()
  const base = snapshot(7)
  const form = { ...savedListener(), bind: "0.0.0.0:443" }
  registry.write(editor, entry(base, fromPlan(base.configuration), listenerSheet(form)))
  registry.release(editor)

  assert.equal(registry.hasDraft(), true)
  const kept = registry.read()
  assert.equal(kept?.base.revision, 7)
  assert.equal(kept?.sheet?.kind === "listener" ? kept.sheet.form.bind : null, "0.0.0.0:443")
})

test("an orphaned save moves only matching kept work onto the saved snapshot", () => {
  const sent = fromPlan(plan)
  const sentKey = draftKey(sent)

  const pristine = new ConnectionsDraftRegistry()
  const saver = pristine.claim()
  pristine.write(saver, entry(snapshot(5, { listeners: [], candidates: [] }), sent))
  pristine.release(saver)
  pristine.adoptOrphanSave(5, sentKey, snapshot(6))
  assert.equal(pristine.read(), null, "a saved draft is no longer unsaved")

  const withSheet = new ConnectionsDraftRegistry()
  const owner = withSheet.claim()
  const form = { ...savedListener(), bind: "0.0.0.0:443" }
  withSheet.write(owner, entry(snapshot(5, { listeners: [], candidates: [] }), sent, listenerSheet(form)))
  withSheet.release(owner)
  withSheet.adoptOrphanSave(5, sentKey, snapshot(6))
  assert.equal(withSheet.read()?.base.revision, 6)
  assert.equal(withSheet.read()?.sheet?.form.key, form.key)

  const newer = new ConnectionsDraftRegistry()
  const first = newer.claim()
  newer.write(first, entry(snapshot(5, { listeners: [], candidates: [] }), sent))
  newer.release(first)
  newer.claim()
  newer.adoptOrphanSave(5, sentKey, snapshot(6))
  assert.equal(newer.read()?.base.revision, 5, "a mounted newer editor's work is untouched")
})

test("logout drops everything", () => {
  const registry = new ConnectionsDraftRegistry()
  registry.write(registry.claim(), entry(snapshot(3), fromPlan(plan)))
  registry.reset()
  assert.equal(registry.hasDraft(), false)
})

test("new endpoint rows never reuse a current row key", () => {
  let listener = savedListener()
  const add = () => {
    listener = { ...listener, endpoints: [...listener.endpoints, { key: newEndpointKey(listener), host: "", port: "" }] }
  }
  const unique = () => new Set(listener.endpoints.map((e) => e.key)).size === listener.endpoints.length

  add()
  assert.ok(unique())
  listener = { ...listener, endpoints: listener.endpoints.slice(1) }
  add()
  add()
  assert.ok(unique())
  assert.equal(listener.endpoints.length, 4)
  // Values stay with their rows after removal.
  assert.equal(listener.endpoints[0]?.host, "lax.example.net")
})
