import assert from "node:assert/strict"
import { test } from "node:test"
import type { ConfigurationView } from "@meshway/control-client"

import { defaultsUpdate, edits, valueOf, withValue } from "./operations.ts"

const VLESS = {
  enabled: true,
  remark: "",
  priority: 300,
  connectTimeout: "5s",
  bindInterface: "",
  localAddress: "",
  socket: { receiveBuffer: 0, sendBuffer: 0 },
  tcp: { noDelay: true },
  stream: "auto" as const,
  tls: { trust: "managed" as const },
  websocketHeaderNames: ["x-a"],
}

const VIEW: ConfigurationView = {
  access: [
    {
      remark: "all",
      src: ["*"],
      dst: ["*"],
      ip: ["*"],
      bidirectional: false,
    },
  ],
  addressPools: { ipv4: ["100.64.0.0/10"], ipv6: ["fd42::/48"] },
  defaults: {
    access: "deny",
    topology: "allow",
    dialers: { native_udp: { priority: 100 }, vless: VLESS, trojan: null },
  },
  dns: {
    domain: "a.example",
    magicDNS: true,
    overrideLocal: false,
    servers: {},
    default: [],
    routes: {},
    searchDomains: [],
  },
  groups: { "group:dev": ["a@example.com", "b@example.com"] },
  nodes: {},
  routing: {},
  ruleSets: {},
  tagsOwners: {},
  topology: [],
}

test("a part's value is its operation's, else the base's", () => {
  assert.equal(valueOf([], VIEW, "access"), VIEW.access)
  const ops = withValue([], VIEW, "access", [])
  assert.deepEqual(valueOf(ops, VIEW, "access"), [])
  assert.equal(edits(ops, "access"), true)
  assert.equal(edits(ops, "topology"), false)
})

test("equal to the base the way the controller reads it: no operation", () => {
  // Omitted defaults, and sets in another order.
  const same = [{ remark: "all", src: ["*", "*"], dst: ["*"], ip: ["*"] }]
  assert.deepEqual(withValue([], VIEW, "access", same), [])
  assert.deepEqual(
    withValue([], VIEW, "groups", {
      "group:dev": ["b@example.com", "a@example.com"],
    }),
    []
  )
  assert.deepEqual(
    withValue([], VIEW, "dns", { domain: "a.example" }),
    [],
    "the DNS defaults are the stated ones"
  )
  // Rule order is still a change.
  const two = withValue([], VIEW, "access", [...VIEW.access, ...VIEW.access])
  assert.equal(two.length, 1)
})

test("rules are written without their default remark and direction", () => {
  const [op] = withValue([], VIEW, "access", [
    { remark: "", src: ["*"], dst: ["tag:a"], ip: ["*"], bidirectional: false },
  ])
  assert.deepEqual(op, {
    type: "replaceAccess",
    access: [{ src: ["*"], dst: ["tag:a"], ip: ["*"] }],
  })
})

test("one operation per part, in a fixed order whatever the editing order", () => {
  let ops = withValue([], VIEW, "addressPools", {
    ipv4: ["100.80.0.0/16"],
    ipv6: ["fd42::/48"],
  })
  ops = withValue(ops, VIEW, "access", [])
  ops = withValue(ops, VIEW, "access", [VIEW.access[0]!, VIEW.access[0]!])
  assert.deepEqual(
    ops.map((op) => op.type),
    ["replaceAccess", "setAddressPools"]
  )
  // Back to the base drops only that part's operation.
  ops = withValue(ops, VIEW, "access", VIEW.access)
  assert.deepEqual(
    ops.map((op) => op.type),
    ["setAddressPools"]
  )
})

test("the topology default restates the dialers the view shows", () => {
  const update = defaultsUpdate(VIEW, "deny")
  assert.equal(update.topology, "deny")
  assert.equal(update.access, "deny")
  assert.deepEqual(update.dialers.native_udp, { priority: 100 })
  assert.equal("trojan" in update.dialers, false)
  // WebSocket headers are not in the view: omitted, so they are kept.
  assert.equal(
    update.dialers.vless && "websocket" in update.dialers.vless,
    false
  )
  assert.equal(
    update.dialers.vless && "websocketHeaderNames" in update.dialers.vless,
    false
  )
  const ops = withValue([], VIEW, "topologyDefault", "deny")
  assert.equal(valueOf(ops, VIEW, "topologyDefault"), "deny")
  assert.deepEqual(withValue(ops, VIEW, "topologyDefault", "allow"), [])
})
