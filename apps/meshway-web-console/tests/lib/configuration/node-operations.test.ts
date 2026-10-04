import assert from "node:assert/strict"
import type {
  ConfigurationView,
  ListenerView,
  NodeSettingsView,
} from "@meshway/control-client"
import { test } from "vitest"

import { inPrefix, sameAddress } from "@/lib/configuration/addresses"
import { listenerFromView } from "@/lib/configuration/listener-update"
import {
  addressesOf,
  isDeleted,
  listenerEdits,
  listenerKeys,
  settingsOf,
  tagsOf,
  withAddresses,
  withDeleted,
  withListener,
  withSettings,
  withTags,
} from "@/lib/configuration/node-operations"
import { nodeProblems } from "@/lib/configuration/node-problems"
import { withValue } from "@/lib/configuration/operations"

const A = "11111111-1111-4111-8111-111111111111"
const B = "22222222-2222-4222-8222-222222222222"

const UDP: ListenerView = {
  protocol: "native_udp",
  listen: "0.0.0.0",
  listenPort: 8421,
  bindInterface: "",
  enabled: true,
  remark: "",
  endpoints: [{ host: "edge.example.com", port: 8421 }],
  socket: {},
  credentialPresent: false,
}

const node = (
  hostname: string,
  ipv4: string,
  extra: Partial<NodeSettingsView> = {}
): NodeSettingsView => ({
  hostname,
  remark: "",
  enabled: true,
  exitEnabled: false,
  identity: { type: "tag", tags: ["tag:a"] },
  addresses: { ipv4, ipv6: null },
  listeners: {},
  ...extra,
})

const VIEW: ConfigurationView = {
  access: [],
  addressPools: { ipv4: ["100.64.0.0/10"], ipv6: ["fd42::/48"] },
  defaults: { access: "deny", topology: "allow", dialers: {} },
  dns: { domain: "a.example" },
  groups: {},
  nodes: {
    [A]: node("a", "100.64.0.1", { listeners: { udp: UDP } }),
    [B]: node("b", "100.64.0.2", {
      identity: { type: "user", user: "u@example.com" },
      dialers: { native_udp: { priority: 100 } },
    }),
  },
  routing: {},
  ruleSets: {},
  tagsOwners: { "tag:a": [], "tag:b": [] },
  topology: [],
}

test("settings collapse to one operation with only the changed fields", () => {
  const base = settingsOf([], VIEW, A)
  assert.ok(base)
  let ops = withSettings([], VIEW, A, { ...base, hostname: "x" })
  ops = withSettings(ops, VIEW, A, {
    ...settingsOf(ops, VIEW, A)!,
    hostname: "web",
    enabled: false,
  })
  assert.deepEqual(ops, [
    {
      type: "setNodeSettings",
      nodeId: A,
      fields: { hostname: "web", enabled: false },
    },
  ])
  assert.equal(settingsOf(ops, VIEW, A)?.hostname, "web")
  // Back to the base drops it.
  assert.deepEqual(withSettings(ops, VIEW, A, base), [])
})

test("dialers: null inherits, {} dials nothing, the same set is no change", () => {
  const b = settingsOf([], VIEW, B)!
  assert.deepEqual(b.dialers, { native_udp: { priority: 100 } })
  const inherit = withSettings([], VIEW, B, { ...b, dialers: null })
  assert.deepEqual(inherit, [
    { type: "setNodeSettings", nodeId: B, fields: { dialers: null } },
  ])
  assert.equal(settingsOf(inherit, VIEW, B)?.dialers, null)
  const none = withSettings([], VIEW, B, { ...b, dialers: {} })
  assert.deepEqual(settingsOf(none, VIEW, B)?.dialers, {})
  assert.deepEqual(
    withSettings([], VIEW, B, {
      ...b,
      dialers: { native_udp: { priority: 100 } },
    }),
    []
  )
  // A node without its own set inherits.
  assert.equal(settingsOf([], VIEW, A)?.dialers, null)
})

test("tags: a set, the base's drops it; a User identity takes tags only when given some", () => {
  let ops = withTags([], VIEW, A, ["tag:b", "tag:a"])
  assert.deepEqual(ops, [
    { type: "setIdentityTags", nodeId: A, tags: ["tag:a", "tag:b"] },
  ])
  assert.deepEqual(withTags(ops, VIEW, A, ["tag:a"]), [])
  assert.equal(tagsOf([], VIEW, B), null)
  assert.deepEqual(withTags([], VIEW, B, []), [])
  ops = withTags([], VIEW, B, ["tag:a"])
  assert.deepEqual(tagsOf(ops, VIEW, B), ["tag:a"])
})

test("addresses name only the families that differ", () => {
  const ops = withAddresses([], VIEW, A, { ipv4: "100.64.0.9", ipv6: null })
  assert.deepEqual(ops, [
    { type: "setAddresses", nodeId: A, addresses: { ipv4: "100.64.0.9" } },
  ])
  assert.deepEqual(addressesOf(ops, VIEW, A), {
    ipv4: "100.64.0.9",
    ipv6: null,
  })
  assert.deepEqual(
    withAddresses(ops, VIEW, A, { ipv4: "100.64.0.1", ipv6: null }),
    []
  )
  const cleared = withAddresses([], VIEW, A, { ipv4: null, ipv6: null })
  assert.deepEqual(cleared[0], {
    type: "setAddresses",
    nodeId: A,
    addresses: { ipv4: null },
  })
})

test("a listener restated as the view is no change; a recreation removes then sets", () => {
  const restated = listenerFromView(UDP)
  assert.ok(restated)
  assert.deepEqual(
    withListener([], VIEW, A, "udp", {
      kind: "set",
      listener: restated,
      recreate: false,
    }),
    []
  )
  const moved = { ...restated, listenPort: 9000 }
  let ops = withListener([], VIEW, A, "udp", {
    kind: "set",
    listener: moved,
    recreate: true,
  })
  assert.deepEqual(
    ops.map((op) => op.type === "setListener" && op.listener === null),
    [true, false]
  )
  assert.deepEqual(listenerEdits(ops, A).get("udp"), {
    kind: "set",
    listener: moved,
    recreate: true,
  })
  ops = withListener(ops, VIEW, A, "udp", { kind: "remove" })
  assert.deepEqual(listenerKeys(ops, VIEW, A), [])
  ops = withListener(ops, VIEW, A, "tcp", {
    kind: "set",
    listener: { type: "native_tcp", listenPort: 8422 },
    recreate: false,
  })
  assert.deepEqual(listenerKeys(ops, VIEW, A), ["tcp"])
  // Removing one the base does not hold is nothing.
  assert.deepEqual(withListener([], VIEW, A, "tcp", { kind: "remove" }), [])
  assert.deepEqual(withListener(ops, VIEW, A, "udp", null).length, 1)
})

test("deletion replaces the node's other operations and is undone alone", () => {
  let ops = withSettings([], VIEW, A, {
    ...settingsOf([], VIEW, A)!,
    remark: "r",
  })
  ops = withValue(ops, VIEW, "access", [{ src: ["*"], dst: ["*"], ip: ["*"] }])
  ops = withDeleted(ops, A, true)
  assert.deepEqual(
    ops.map((op) => op.type),
    ["replaceAccess", "deleteNode"]
  )
  assert.equal(isDeleted(ops, A), true)
  // A deleted node takes no edits.
  assert.deepEqual(withTags(ops, VIEW, A, ["tag:b"]), ops)
  assert.deepEqual(
    withDeleted(ops, A, false).map((op) => op.type),
    ["replaceAccess"]
  )
})

test("operations are ordered: the rest first, then by node and kind", () => {
  let ops = withListener([], VIEW, B, "x", {
    kind: "set",
    listener: { type: "native_udp", listenPort: 1 },
    recreate: false,
  })
  ops = withSettings(ops, VIEW, B, {
    ...settingsOf(ops, VIEW, B)!,
    remark: "r",
  })
  ops = withTags(ops, VIEW, A, ["tag:b"])
  ops = withValue(ops, VIEW, "dns", { domain: "b.example" })
  assert.deepEqual(
    ops.map((op) => [op.type, "nodeId" in op ? op.nodeId : ""]),
    [
      ["replaceDns", ""],
      ["setIdentityTags", A],
      ["setNodeSettings", B],
      ["setListener", B],
    ]
  )
})

test("node problems: hostnames, tags and addresses at the controller's pointers", () => {
  let ops = withSettings([], VIEW, A, {
    ...settingsOf([], VIEW, A)!,
    hostname: "b",
  })
  ops = withAddresses(ops, VIEW, A, { ipv4: "10.0.0.1", ipv6: "fd42::2" })
  ops = withTags(ops, VIEW, A, ["tag:zz"])
  assert.deepEqual(
    nodeProblems(ops, VIEW).map((problem) => [problem.path, problem.code]),
    [
      [`/nodes/${A}/hostname`, "hostname_taken"],
      [`/nodes/${A}/identity/tags`, "tag"],
      [`/nodes/${A}/addresses/ipv4`, "address_pool"],
    ]
  )
  const taken = withAddresses([], VIEW, A, { ipv4: "100.64.0.2", ipv6: null })
  assert.deepEqual(
    nodeProblems(taken, VIEW).map((problem) => problem.code),
    ["address_taken"]
  )
  const bad = withSettings([], VIEW, A, {
    ...settingsOf([], VIEW, A)!,
    hostname: "Bad Name",
    dialers: { vless: { connectTimeout: "soon" } },
  })
  assert.deepEqual(
    nodeProblems(bad, VIEW).map((problem) => [problem.path, problem.code]),
    [
      [`/nodes/${A}/hostname`, "hostname"],
      [`/nodes/${A}/dialers/vless`, "dialer"],
    ]
  )
  // A deleted node's hostname is free.
  const freed = withSettings(withDeleted([], B, true), VIEW, A, {
    ...settingsOf([], VIEW, A)!,
    hostname: "b",
  })
  assert.deepEqual(nodeProblems(freed, VIEW), [])
})

test("addresses inside prefixes, both families", () => {
  assert.equal(inPrefix("100.64.0.9", "100.64.0.0/10"), true)
  assert.equal(inPrefix("100.128.0.1", "100.64.0.0/10"), false)
  assert.equal(inPrefix("fd42::1", "fd42::/48"), true)
  assert.equal(inPrefix("fd42:0:1::1", "fd42::/48"), false)
  assert.equal(inPrefix("::ffff:100.64.0.1", "::ffff:0:0/96"), true)
  assert.equal(inPrefix("fd42::1", "100.64.0.0/10"), false)
  assert.equal(sameAddress("fd42::1", "fd42:0:0:0:0:0:0:1"), true)
})
