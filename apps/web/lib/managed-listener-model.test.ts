import assert from "node:assert/strict"
import { test } from "node:test"

import type { ConnectionsSnapshot, ManagedListenerSettings } from "@meshway/control-client"
import { zConnectionsSnapshot, zManagedListenerSettings, zManagedListenersSnapshot } from "@meshway/control-client/schemas"

import { ManagedListenerDraftRegistry, type ManagedListenerDraft } from "./managed-listener-draft-registry.ts"
import {
  allowedPresets,
  bindAddressProblem,
  changeImpact,
  changePreset,
  followSuggestions,
  fromSettings,
  isManagedListener,
  manualListenerIds,
  newManagedForm,
  referencingCandidates,
  sameForm,
  takenListenerIds,
  toSettings,
  validateManagedForm,
  websocketHostProblem,
  websocketPathProblem,
} from "./managed-listener-model.ts"

const NODE = "a".repeat(64)
const DIALER = "b".repeat(64)

const ws: ManagedListenerSettings = zManagedListenerSettings.parse({
  id: "edge-ws",
  node_id: NODE,
  enabled: true,
  transport: "vless",
  bind_address: "0.0.0.0",
  bind_port: 443,
  advertised_endpoints: [{ host: "edge.example.com", port: 8443 }],
  vless: { stream: "websocket", server_name: "meshway.local", websocket_path: "/meshway", websocket_host: "edge.example.com" },
})

function connections(candidates: { id: string; listener: string; host: string; port: number }[]): ConnectionsSnapshot {
  return zConnectionsSnapshot.parse({
    revision: 7,
    valid: true,
    diagnostics: [],
    nodes: [],
    configuration: {
      listeners: [
        { id: "edge-ws", node_id: NODE, transport: "vless", bind: "0.0.0.0:443", advertised_endpoints: [{ host: "edge.example.com", port: 8443 }], credential_ref: "managed.edge-ws" },
        { id: "legacy-vless", node_id: NODE, transport: "vless", bind: "0.0.0.0:9443", advertised_endpoints: [], credential_ref: "local-profile" },
      ],
      candidates: candidates.map((candidate) => ({
        id: candidate.id,
        dialer_node_id: DIALER,
        listener_id: candidate.listener,
        endpoint: { host: candidate.host, port: candidate.port },
        priority: 0,
        credential_ref: null,
      })),
    },
  })
}

test("the generated DTO round-trips through the form and passes the generated schema", () => {
  const form = fromSettings(ws)
  const settings = toSettings(form, NODE)
  assert.deepEqual(settings, ws)
  assert.equal(zManagedListenerSettings.safeParse(settings).success, true)
  // Native presets send vless: null; VLESS TCP sends explicit null WebSocket fields.
  const native = toSettings(newManagedForm("native_udp", []), NODE)
  assert.equal(native.vless, null)
  assert.equal(native.bind_port, 8421)
  const tcp = toSettings(newManagedForm("vless_tcp", []), "")
  assert.deepEqual(tcp.vless, { stream: "tcp", server_name: "meshway.local", websocket_path: null, websocket_host: null })
  assert.equal(tcp.bind_port, 443)
  assert.deepEqual(tcp.advertised_endpoints, [], "no public address is guessed")
})

test("preset changes keep typed values and move only untouched suggestions", () => {
  const start = newManagedForm("native_udp", ["native-udp"])
  assert.equal(start.id, "native-udp-2")
  const vless = changePreset(start, "vless_tcp", { idFollows: true, taken: ["native-udp"] })
  assert.equal(vless.id, "vless")
  assert.equal(vless.bindPort, "443")
  assert.equal(vless.serverName, "meshway.local")
  const typed = { ...vless, bindPort: "9443", endpoints: [{ key: "e0", host: "edge.example.com", port: "9443" }] }
  const websocket = changePreset(typed, "vless_websocket", { idFollows: false, taken: [] })
  assert.equal(websocket.id, "vless")
  assert.equal(websocket.bindPort, "9443")
  assert.equal(websocket.websocketPath, "/meshway")
  assert.equal(websocket.websocketHost, "edge.example.com", "Host defaults to the advertised hostname")
  const back = changePreset(websocket, "vless_tcp", { idFollows: false, taken: [] })
  assert.equal(toSettings(back, NODE).vless?.websocket_path, null, "hidden WebSocket values are never sent")
  const fresh = changePreset(newManagedForm("vless_tcp", []), "vless_websocket", { idFollows: true, taken: [] })
  assert.equal(fresh.websocketHost, "meshway.local")
  const withEndpoint = followSuggestions(fresh, { ...fresh, endpoints: [{ key: "e0", host: "edge.example.test", port: "443" }] })
  assert.equal(withEndpoint.websocketHost, "edge.example.test", "an untouched Host follows the advertised hostname")
  const typedHost = { ...withEndpoint, websocketHost: "cdn.example.test" }
  assert.equal(followSuggestions(typedHost, { ...typedHost, endpoints: [] }).websocketHost, "cdn.example.test")
  assert.deepEqual(allowedPresets(ws), ["vless_tcp", "vless_websocket"], "transport is fixed after creation")
})

test("local validation mirrors the controller's field rules", () => {
  const form = { ...fromSettings(ws), id: "managed.x", bindAddress: "edge.example.com", bindPort: "0" }
  const fields = validateManagedForm(form, { creating: true, takenIds: [] }).map((issue) => issue.field)
  assert.deepEqual(fields, ["id", "bind_address", "bind_port"])
  assert.equal(validateManagedForm(fromSettings(ws), { creating: true, takenIds: ["edge-ws"] })[0]?.field, "id")
  assert.equal(validateManagedForm(fromSettings(ws), { creating: false, takenIds: ["edge-ws"] }).length, 0, "an edit keeps its own ID")
  assert.equal(bindAddressProblem("::"), null)
  assert.equal(bindAddressProblem("[::]") !== null, true)
  assert.equal(bindAddressProblem("239.1.1.1") !== null, true)
  assert.equal(websocketPathProblem("/a?b") !== null, true)
  assert.equal(websocketPathProblem("meshway") !== null, true)
  assert.equal(websocketHostProblem("edge.example.com:8443"), null)
  assert.equal(websocketHostProblem("edge.example.com/x") !== null, true)
  const duplicate = { ...fromSettings(ws), endpoints: [{ key: "e0", host: "Edge.example.com", port: "8443" }, { key: "e1", host: "edge.example.com.", port: "8443" }] }
  assert.deepEqual(validateManagedForm(duplicate, { creating: false, takenIds: [] }).map((issue) => issue.field), ["advertised_endpoints[1].host"])
  const badName = { ...fromSettings(ws), serverName: "-bad-.local", websocketHost: "" }
  assert.deepEqual(validateManagedForm(badName, { creating: false, takenIds: [] }).map((issue) => issue.field), ["server_name", "websocket_host"])
})

test("change impact is visible before saving: disable, moved endpoints, certificate rotation", () => {
  const snapshot = connections([
    { id: "dial-ws", listener: "edge-ws", host: "edge.example.com", port: 8443 },
    { id: "dial-old", listener: "edge-ws", host: "old.example.com", port: 8443 },
  ])
  const form = { ...fromSettings(ws), enabled: false }
  const candidates = referencingCandidates(snapshot, "edge-ws", form.endpoints)
  assert.equal(candidates.length, 2)
  const disabled = changeImpact(ws, form, candidates)
  assert.equal(disabled.disablesReferenced, 2)
  assert.equal(disabled.orphanedCandidates.length, 0, "a disabled listener has no endpoint to miss")
  const moved = changeImpact(ws, fromSettings(ws), candidates)
  assert.deepEqual(moved.orphanedCandidates.map((candidate) => candidate.id), ["dial-old"])
  assert.equal(moved.rotatesMaterial, false)
  const renamed = changeImpact(ws, { ...fromSettings(ws), serverName: "Other.local" }, candidates)
  assert.equal(renamed.rotatesMaterial, true)
  assert.equal(changeImpact(ws, { ...fromSettings(ws), serverName: "MESHWAY.local" }, []).rotatesMaterial, false, "the controller lowercases the name")
})

test("managed and manual listeners are told apart; IDs are network-wide", () => {
  const managed = zManagedListenersSnapshot.parse({
    revision: 7,
    listeners: [{ settings: ws, material_version: 2, certificate_sha256: "f".repeat(64), compiled_capability_available: false }],
  })
  const snapshot = connections([])
  assert.deepEqual(manualListenerIds(managed, snapshot, NODE), ["legacy-vless"])
  assert.deepEqual(takenListenerIds(managed, snapshot).sort(), ["edge-ws", "legacy-vless"])
  assert.equal(isManagedListener(null, "edge-ws", "managed.edge-ws"), true, "VLESS reference identifies a managed listener even when the managed read failed")
  assert.equal(isManagedListener(null, "native", null), false)
  assert.equal(isManagedListener(new Set(["native"]), "native", null), true)
})

test("drafts are kept per key and only an unchanged sent draft is settled", () => {
  const registry = new ManagedListenerDraftRegistry()
  const form = fromSettings(ws)
  const draft: ManagedListenerDraft = { key: "k", kind: "edit", nodeId: NODE, baseRevision: 7, original: ws, initial: form, form: { ...form, bindPort: "444" }, idFollows: false, open: true }
  let notified = 0
  registry.subscribe(() => (notified += 1))
  registry.put(draft)
  const later = { ...draft, form: { ...draft.form, bindPort: "445" } }
  registry.put(later)
  registry.settle("k", draft)
  assert.equal(registry.get("k"), later, "a save of an older form never drops newer edits")
  registry.settle("k", later)
  assert.equal(registry.hasDraft(), false)
  assert.equal(notified, 3)
  registry.put(draft)
  assert.deepEqual(registry.forNode(NODE), [draft])
  registry.reset()
  assert.equal(registry.hasDraft(), false)
  assert.equal(sameForm(form, fromSettings(ws)), true)
})
