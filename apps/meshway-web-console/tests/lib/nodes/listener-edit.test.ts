import assert from "node:assert/strict"
import type { ListenerView } from "@meshway/control-client"
import { test } from "vitest"

import { listenerFromView } from "@/lib/configuration/listener-update"
import {
  draftOfView,
  draftProblems,
  editable,
  keepsCertificate,
  keepsUuid,
  newDraft,
  updateOf,
  type ListenerOrigin,
} from "@/lib/nodes/listener-edit"
import { newListener } from "@/lib/nodes/listener-model"

const VLESS: ListenerView = {
  protocol: "vless",
  listen: "0.0.0.0",
  listenPort: 443,
  bindInterface: "",
  enabled: true,
  remark: "edge",
  endpoints: [
    { host: "edge.example.com", port: 443 },
    { host: "203.0.113.9", port: 8443 },
  ],
  socket: {},
  tcp: { noDelay: true },
  credentialPresent: true,
  flow: "xtls-rprx-vision",
  stream: "websocket",
  tls: {
    serverName: "edge.example.com",
    mode: "managed",
    alpn: ["http/1.1"],
    minVersion: "1.2",
    maxVersion: "1.3",
    handshakeTimeout: "10s",
    certificatePresent: true,
    privateKeyPresent: true,
  },
  websocket: { path: "/mesh", host: null, responseHeaderNames: [] },
}

const origin = (view: ListenerView): ListenerOrigin => ({
  view,
  restated: listenerFromView(view),
})

test("an untouched draft restates the view exactly, secrets omitted", () => {
  const draft = draftOfView(VLESS)
  assert.ok(draft)
  assert.equal(draft.fields.preset, "vless_websocket")
  const update = updateOf(draft, origin(VLESS))
  assert.deepEqual(update, listenerFromView(VLESS))
  assert.ok(update?.type === "vless")
  assert.equal(update.uuid, undefined)
  assert.equal(update.tls.certificate, undefined)
  assert.deepEqual(update.endpoints, VLESS.endpoints)
})

test("edits keep fields the form does not show and the other endpoints", () => {
  const draft = draftOfView(VLESS)!
  const update = updateOf(
    {
      ...draft,
      fields: { ...draft.fields, port: "8443", host: "new.example.com" },
      wsPath: "/other",
      uuid: "6f1c2b9e-0d4a-4c3e-9a51-2f6e8b7d1c00",
    },
    origin(VLESS)
  )
  assert.ok(update?.type === "vless")
  assert.equal(update.listenPort, 8443)
  assert.deepEqual(update.endpoints, [
    // No public port of its own: it follows the bind port.
    { host: "new.example.com", port: 8443 },
    { host: "203.0.113.9", port: 8443 },
  ])
  assert.equal(update.flow, "xtls-rprx-vision")
  assert.deepEqual(update.tls.alpn, ["http/1.1"])
  assert.equal(update.websocket?.path, "/other")
  assert.equal(update.uuid, "6f1c2b9e-0d4a-4c3e-9a51-2f6e8b7d1c00")
})

test("a provided certificate is written as a pair, or kept when held", () => {
  const draft = { ...draftOfView(VLESS)!, tlsMode: "provided" as const }
  // The managed certificate is not one to keep as provided.
  assert.equal(keepsCertificate(draft, origin(VLESS)), false)
  assert.deepEqual(draftProblems(draft, origin(VLESS)), ["certificate_missing"])
  assert.deepEqual(
    draftProblems({ ...draft, certificate: "-----BEGIN" }, origin(VLESS)),
    ["certificate_pair"]
  )
  const provided: ListenerView = {
    ...VLESS,
    tls: { ...VLESS.tls!, mode: "provided" },
  }
  const held = draftOfView(provided)!
  assert.equal(keepsCertificate(held, origin(provided)), true)
  assert.deepEqual(draftProblems(held, origin(provided)), [])
  // Recreating keeps nothing.
  assert.deepEqual(
    draftProblems({ ...held, recreate: true }, origin(provided)),
    ["certificate_missing"]
  )
  assert.equal(keepsUuid({ ...held, recreate: true }, origin(provided)), false)
  assert.equal(keepsUuid(held, origin(provided)), true)
})

test("bad UUIDs and paths are problems; a new listener has nothing to keep", () => {
  const draft = draftOfView(VLESS)!
  assert.deepEqual(
    draftProblems({ ...draft, uuid: "nope", wsPath: "mesh" }, origin(VLESS)),
    ["ws_path", "uuid"]
  )
  const fresh = newDraft(newListener("native_tcp"))
  const none = { view: null, restated: null }
  assert.deepEqual(updateOf(fresh, none), {
    type: "native_tcp",
    listenPort: 8421,
    enabled: true,
    remark: "",
    endpoints: [],
  })
  assert.equal(keepsUuid(fresh, none), false)
})

test("WebSocket response headers are kept by leaving them out; over TCP the block goes", () => {
  const headed: ListenerView = {
    ...VLESS,
    websocket: { path: "/mesh", host: null, responseHeaderNames: ["x-a"] },
  }
  assert.equal(editable(headed), true)
  const draft = draftOfView(headed)!
  const update = updateOf({ ...draft, wsPath: "/other" }, origin(headed))
  assert.ok(update?.type === "vless")
  // No responseHeaders: the controller keeps the listener's own.
  assert.deepEqual(update.websocket, { path: "/other" })
  const tcp = updateOf(
    { ...draft, fields: { ...draft.fields, preset: "vless_tcp" } },
    origin(headed)
  )
  assert.ok(tcp?.type === "vless")
  assert.equal(tcp.stream, "tcp")
  // Left out, the block would be kept.
  assert.equal(tcp.websocket, null)
})

test("a transport the form does not offer is edited in the source", () => {
  assert.equal(editable(VLESS), true)
  assert.equal(editable({ ...VLESS, protocol: "trojan", stream: "tcp" }), false)
})
