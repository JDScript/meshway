import assert from "node:assert/strict"
import type { ConfigurationView } from "@meshway/control-client"
import { test } from "vitest"

import { edits, valueOf, withValue } from "@/lib/configuration/operations"

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

test("an edge's transports are kept; any and none are two values", () => {
  const rule = { action: "allow" as const, a: "*", b: "tag:relay" }
  const limited = {
    ...VIEW,
    topology: [
      { ...rule, transports: ["vless" as const, "native_tcp" as const] },
    ],
  }
  // The same set in another order is no change.
  assert.deepEqual(
    withValue([], limited, "topology", [
      { ...rule, transports: ["native_tcp", "vless"] },
    ]),
    []
  )
  const [none] = withValue([], limited, "topology", [
    { ...rule, transports: [] },
  ])
  assert.deepEqual(none, {
    type: "replaceTopology",
    topology: [{ ...rule, transports: [] }],
  })
  // Any (omitted) is not none, and an edit elsewhere keeps the transports.
  assert.equal(withValue([], limited, "topology", [rule]).length, 1)
  const [remarked] = withValue([], limited, "topology", [
    { ...rule, remark: "relay", transports: ["vless", "native_tcp"] },
  ])
  assert.deepEqual(remarked, {
    type: "replaceTopology",
    topology: [
      { ...rule, remark: "relay", transports: ["vless", "native_tcp"] },
    ],
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

test("a topology default change sends the topology alone", () => {
  const base = valueOf([], VIEW, "defaults")
  const ops = withValue([], VIEW, "defaults", { ...base, topology: "deny" })
  // Omitted fields are kept: no access, no dialers restated.
  assert.deepEqual(ops, [
    { type: "replaceDefaults", defaults: { topology: "deny" } },
  ])
  assert.equal(valueOf(ops, VIEW, "defaults").topology, "deny")
  assert.deepEqual(valueOf(ops, VIEW, "defaults").dialers, base.dialers)
  assert.deepEqual(withValue(ops, VIEW, "defaults", base), [])
})

test("the topology and dialer defaults share one replaceDefaults", () => {
  const base = valueOf([], VIEW, "defaults")
  let ops = withValue([], VIEW, "defaults", { ...base, topology: "deny" })
  const dialers = { ...base.dialers, native_tcp: {} }
  ops = withValue(ops, VIEW, "defaults", {
    ...valueOf(ops, VIEW, "defaults"),
    dialers,
  })
  assert.equal(ops.length, 1)
  const op = ops[0]
  assert.ok(op?.type === "replaceDefaults")
  assert.equal(op.defaults.topology, "deny")
  assert.deepEqual(op.defaults.dialers?.native_tcp, {})
  // Key order does not make a change.
  const reordered = Object.fromEntries(Object.entries(base.dialers).reverse())
  assert.deepEqual(
    withValue([], VIEW, "defaults", { ...base, dialers: reordered }),
    []
  )
})

test("routing edits collapse into one replaceRouting; back to base drops it", () => {
  const first = withValue([], VIEW, "routing", {
    mesh: [{ src: ["*"], dst: ["tag:web"], via: [] }],
  })
  const second = withValue(first, VIEW, "routing", {
    mesh: [
      { src: ["*"], dst: ["tag:web"], via: [] },
      { src: ["*"], dst: ["tag:db"], via: [] },
    ],
  })
  assert.equal(second.length, 1)
  assert.equal(second[0]?.type, "replaceRouting")
  assert.equal(valueOf(second, VIEW, "routing").mesh?.length, 2)
  assert.deepEqual(
    withValue(second, VIEW, "routing", { mesh: [], egress: { rules: [] } }),
    []
  )
})

test("rule sets go before the routing that names them", () => {
  const ops = withValue(
    withValue([], VIEW, "routing", {
      egress: {
        rules: [{ match: { ruleSets: ["o"] }, action: { type: "direct" } }],
      },
    }),
    VIEW,
    "ruleSets",
    { o: { type: "inline", ipCIDR: ["10.0.0.0/8"] } }
  )
  assert.deepEqual(
    ops.map((op) => op.type),
    ["replaceRuleSets", "replaceRouting"]
  )
  assert.equal(edits(ops, "ruleSets"), true)
})
