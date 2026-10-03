import assert from "node:assert/strict"
import { test } from "node:test"

import { zManagedListenerSettings } from "../../../../../packages/control-client/src/generated/zod.gen.ts"
import {
  advancedFacts,
  advancedProblems,
  fieldsOf,
  initialListener,
  listenerProblems,
  newListener,
  presetsFor,
  settingsOf,
  suggestId,
  withPort,
  withPreset,
  withPublicHost,
  withPublicPort,
  type ListenerFields,
  type Preset,
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

/** The add-node form: a preset, then an address and a port typed into it. */
const minimal = (preset: Preset, host: string, port?: string) => {
  let fields: ListenerFields = withPreset(
    newListener("native_udp", none),
    preset,
    null
  )
  fields = withPublicHost(fields, host)
  if (port !== undefined) fields = withPort(fields, port)
  return settingsOf(initialListener(fields, new Set(["vless"])), "")
}

test("the basics alone make a complete listener at the platform defaults", () => {
  const udp = minimal("native_udp", "")
  assert.ok(
    zManagedListenerSettings.safeParse({ ...udp, node_id: NODE }).success
  )
  assert.deepEqual(udp, {
    id: "native-udp",
    node_id: "",
    enabled: true,
    transport: "native_udp",
    bind_address: "0.0.0.0",
    bind_port: 8421,
    advertised_endpoints: [],
    vless: null,
  })
  assert.deepEqual(minimal("native_udp", "203.0.113.10").advertised_endpoints, [
    { host: "203.0.113.10", port: 8421 },
  ])
  const ws = minimal("vless_websocket", "node.example.com")
  assert.equal(ws.id, "vless-ws")
  assert.equal(ws.bind_port, 443)
  assert.deepEqual(ws.advertised_endpoints, [
    { host: "node.example.com", port: 443 },
  ])
  assert.deepEqual(ws.vless, {
    stream: "websocket",
    server_name: "meshway.local",
    websocket_path: "/meshway",
    websocket_host: "node.example.com",
  })
  assert.equal(
    minimal("vless_websocket", "").vless?.websocket_host,
    "meshway.local"
  )
  assert.equal(minimal("vless_tcp", "").id, "vless-2")
})

test("an address typed before choosing WebSocket becomes its Host; the port follows the preset", () => {
  let fields = withPublicHost(
    newListener("native_udp", none),
    "edge.example.com"
  )
  fields = withPreset(fields, "vless_websocket", null)
  assert.equal(fields.websocketHost, "edge.example.com")
  assert.deepEqual(fields.endpoints, [
    { host: "edge.example.com", port: "443" },
  ])
  const typed = withPreset(withPort(fields, "9443"), "native_udp", null)
  assert.equal(typed.bindPort, "9443")
})

test("the public port follows the port until it is set apart", () => {
  let fields = withPublicHost(newListener("vless_tcp", none), "a.example")
  fields = withPort(fields, "8443")
  assert.deepEqual(fields.endpoints, [{ host: "a.example", port: "8443" }])
  fields = withPublicPort(fields, "41001")
  fields = withPort(fields, "9443")
  assert.equal(fields.bindPort, "9443")
  assert.deepEqual(fields.endpoints, [{ host: "a.example", port: "41001" }])
  assert.deepEqual(
    withPublicPort(newListener("vless_tcp", none), "1").endpoints,
    []
  )
})

test("clearing the address drops the endpoint only when nothing else was typed", () => {
  const one = withPublicHost(newListener("native_tcp", none), "a.example")
  assert.deepEqual(withPublicHost(one, "").endpoints, [])
  const mapped = withPublicHost(withPublicPort(one, "9000"), "")
  assert.deepEqual(mapped.endpoints, [{ host: "", port: "9000" }])
  assert.equal(listenerProblems(mapped, null)[0]?.field, "endpoint.0.host")
  const two = {
    ...one,
    endpoints: [...one.endpoints, { host: "b.example", port: "1" }],
  }
  assert.equal(withPublicHost(two, "").endpoints.length, 2)
})

test("the WebSocket Host follows the address until it is typed", () => {
  const ws = newListener("vless_websocket", none)
  const a = withPublicHost(ws, "a.example")
  assert.equal(a.websocketHost, "a.example")
  assert.equal(withPublicHost(a, "b.example").websocketHost, "b.example")
  assert.equal(withPublicHost(a, "").websocketHost, "meshway.local")
  const own = withPublicHost(
    { ...a, websocketHost: "cdn.example" },
    "b.example"
  )
  assert.equal(own.websocketHost, "cdn.example")
  assert.equal(
    withPublicHost(newListener("vless_tcp", none), "a.example").websocketHost,
    ""
  )
})

test("the advanced summary names what differs from the defaults", () => {
  const ws = withPublicHost(newListener("vless_websocket", none), "a.example")
  assert.deepEqual(advancedFacts(ws), [])
  assert.deepEqual(
    advancedFacts({
      ...withPublicPort(ws, "41001"),
      bindAddress: "::",
      endpoints: [
        { host: "a.example", port: "41001" },
        { host: "b.example", port: "443" },
      ],
      serverName: "edge.example.com",
      websocketPath: "/x",
      websocketHost: "cdn.example",
    }),
    [
      { kind: "bind_address", value: "::" },
      { kind: "public_port", value: "41001" },
      { kind: "endpoints", count: 2 },
      { kind: "server_name", value: "edge.example.com" },
      { kind: "websocket_path", value: "/x" },
      { kind: "websocket_host", value: "cdn.example" },
    ]
  )
  assert.deepEqual(advancedFacts(newListener("native_udp", none)), [])
})

test("a linked port's problem belongs to the port field, not the advanced part", () => {
  const linked = withPort(
    withPublicHost(newListener("native_udp", none), "a.example"),
    ""
  )
  const problems = listenerProblems(linked, null)
  assert.deepEqual(
    problems.map((problem) => problem.field),
    ["bind_port", "endpoint.0.port"]
  )
  assert.deepEqual(advancedProblems(linked, problems), [])
  const apart = withPublicPort(linked, "x")
  assert.deepEqual(
    advancedProblems(apart, listenerProblems(apart, null)).map((p) => p.field),
    ["endpoint.0.port"]
  )
})
