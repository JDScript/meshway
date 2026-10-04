import assert from "node:assert/strict"
import { test } from "vitest"

import {
  advancedSet,
  isHost,
  listenerOf,
  listenerProblems,
  newListener,
  presetsIn,
  withPreset,
} from "@/features/nodes/listener-model"

test("a new listener suggests its transport's port", () => {
  assert.equal(newListener("native_udp").port, "8421")
  assert.equal(newListener("vless_websocket").port, "443")
})

test("a port at the old suggestion follows the transport; a typed one stays", () => {
  assert.equal(withPreset(newListener("native_udp"), "vless_tcp").port, "443")
  const typed = { ...newListener("native_udp"), port: "9000" }
  assert.equal(withPreset(typed, "vless_tcp").port, "9000")
})

test("only the catalog's protocols are offered", () => {
  assert.deepEqual(presetsIn(["native_udp", "vless"]), [
    "native_udp",
    "vless_tcp",
    "vless_websocket",
  ])
})

test("hosts are IPs or DNS names", () => {
  for (const host of [
    "1.2.3.4",
    "::1",
    "edge.example.com",
    "edge.example.com.",
    "例子.测试",
  ])
    assert.ok(isHost(host), host)
  for (const host of [
    "",
    "a_b.example",
    "-a.example",
    "https://x",
    "a..b",
    "x:443",
  ])
    assert.ok(!isHost(host), host)
})

test("a UDP listener sends only what was typed", () => {
  assert.deepEqual(listenerOf(newListener("native_udp")), {
    type: "native_udp",
    listenPort: 8421,
  })
  assert.deepEqual(
    listenerOf({ ...newListener("native_tcp"), host: "edge.example.com" }),
    {
      type: "native_tcp",
      listenPort: 8421,
      endpoints: [{ host: "edge.example.com", port: 8421 }],
    }
  )
})

test("a public port of its own and a bind address", () => {
  const fields = {
    ...newListener("native_udp"),
    host: "203.0.113.7",
    publicPort: "40000",
    listen: "::",
  }
  assert.ok(advancedSet(fields))
  assert.deepEqual(listenerOf(fields), {
    type: "native_udp",
    listenPort: 8421,
    listen: "::",
    endpoints: [{ host: "203.0.113.7", port: 40000 }],
  })
})

test("VLESS takes its TLS name from the public address unless one is typed", () => {
  const ws = { ...newListener("vless_websocket"), host: "edge.example.com" }
  assert.deepEqual(listenerOf(ws), {
    type: "vless",
    listenPort: 443,
    endpoints: [{ host: "edge.example.com", port: 443 }],
    stream: "websocket",
    tls: { serverName: "edge.example.com" },
  })
  const named = { ...newListener("vless_tcp"), serverName: "mesh.internal" }
  assert.deepEqual(listenerOf(named), {
    type: "vless",
    listenPort: 443,
    tls: { serverName: "mesh.internal" },
  })
})

test("problems stop the listener", () => {
  assert.deepEqual(listenerProblems(newListener("vless_tcp")), [
    "server_name_missing",
  ])
  assert.equal(listenerOf(newListener("vless_tcp")), null)
  assert.deepEqual(
    listenerProblems({
      ...newListener("native_udp"),
      port: "70000",
      host: "bad host",
      listen: "example.com",
    }),
    ["port", "host", "listen"]
  )
  assert.deepEqual(
    listenerProblems({
      ...newListener("native_udp"),
      host: "a.example",
      publicPort: "0",
    }),
    ["public_port"]
  )
  // A public port without a public address is not sent, so not checked.
  assert.deepEqual(
    listenerProblems({ ...newListener("native_udp"), publicPort: "x" }),
    []
  )
})
