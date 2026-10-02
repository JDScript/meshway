import assert from "node:assert/strict"
import { test } from "node:test"

import { zManagedListenerSettings } from "../../../../../packages/control-client/src/generated/zod.gen.ts"
import {
  fieldsOf,
  listenerProblems,
  newListener,
  presetsFor,
  settingsOf,
  suggestId,
  withPreset,
} from "./listener-model.ts"

const NODE = "a".repeat(64)
const none = new Set<string>()

test("a new listener follows the documented suggestions and is valid", () => {
  const vless = newListener("vless_tcp", none)
  assert.equal(vless.id, "vless")
  assert.equal(vless.bindPort, "443")
  assert.equal(vless.serverName, "meshway.local")
  assert.deepEqual(vless.endpoints, [])
  assert.deepEqual(listenerProblems(vless, { taken: none }), [])
  assert.equal(newListener("native_udp", none).bindPort, "8421")
})

test("suggested IDs avoid the network's taken IDs", () => {
  assert.equal(suggestId("vless_tcp", new Set(["vless", "vless-2"])), "vless-3")
})

test("settings are the generated DTO; fields that do not apply are null", () => {
  const ws = withPreset(newListener("vless_tcp", none), "vless_websocket", {
    taken: none,
  })
  assert.equal(ws.id, "vless-ws")
  const settings = settingsOf(
    { ...ws, endpoints: [{ host: " node.example.com ", port: "8443" }] },
    NODE
  )
  assert.ok(zManagedListenerSettings.safeParse(settings).success)
  assert.deepEqual(settings.vless, {
    stream: "websocket",
    server_name: "meshway.local",
    websocket_path: "/meshway",
    websocket_host: "meshway.local",
  })
  assert.deepEqual(settings.advertised_endpoints, [
    { host: "node.example.com", port: 8443 },
  ])
  const udp = settingsOf(newListener("native_udp", none), NODE)
  assert.equal(udp.vless, null)
  assert.deepEqual(fieldsOf(udp), newListener("native_udp", none))
})

test("a typed port survives a preset change; a suggested one follows", () => {
  const typed = { ...newListener("native_udp", none), bindPort: "9000" }
  assert.equal(withPreset(typed, "vless_tcp", null).bindPort, "9000")
  assert.equal(
    withPreset(newListener("native_udp", none), "vless_tcp", null).bindPort,
    "443"
  )
})

test("a saved listener keeps its transport", () => {
  const saved = settingsOf(newListener("vless_tcp", none), NODE)
  assert.deepEqual(presetsFor(saved), ["vless_tcp", "vless_websocket"])
  assert.equal(presetsFor(null).length, 4)
})

test("problems name each invalid field in form order", () => {
  const fields = {
    ...newListener("vless_websocket", none),
    id: "managed.x",
    bindAddress: "example.com",
    bindPort: "70000",
    endpoints: [
      { host: "a.example", port: "1" },
      { host: "A.example.", port: "1" },
      { host: "bad host", port: "x" },
    ],
    serverName: "",
    websocketPath: "/a?b",
    websocketHost: "h/x",
  }
  assert.deepEqual(
    listenerProblems(fields, { taken: none }).map((problem) => problem.field),
    [
      "id",
      "bind_address",
      "bind_port",
      "endpoint.1.host",
      "endpoint.2.host",
      "endpoint.2.port",
      "server_name",
      "websocket_path",
      "websocket_host",
    ]
  )
  assert.deepEqual(
    listenerProblems(newListener("native_tcp", none), {
      taken: new Set(["native-tcp"]),
    }),
    [{ field: "id", kind: "id_taken" }]
  )
  assert.deepEqual(
    listenerProblems(
      { ...newListener("native_tcp", none), bindAddress: "::" },
      null
    ),
    []
  )
  assert.equal(
    listenerProblems(
      { ...newListener("native_tcp", none), bindAddress: "224.0.0.1" },
      null
    )[0]?.field,
    "bind_address"
  )
})
