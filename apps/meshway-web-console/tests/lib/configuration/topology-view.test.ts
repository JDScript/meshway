import assert from "node:assert/strict"
import type {
  AggregatedLink,
  ConfigurationView,
  NodeIdentity,
  NodeSettingsView,
} from "@meshway/control-client"
import { test } from "vitest"

import {
  configurationTopology,
  TOPOLOGY_NODE_LIMIT,
} from "@/lib/configuration/topology-view"
import { mapConnections, mapNodes, overviewScene } from "@/lib/network-map"

const A = "11111111-1111-4111-8111-111111111111"
const B = "22222222-2222-4222-8222-222222222222"
const C = "33333333-3333-4333-8333-333333333333"
const node = (hostname: string, identity: NodeIdentity): NodeSettingsView => ({
  hostname,
  identity,
  enabled: true,
  remark: "",
  addresses: { ipv4: null, ipv6: null },
  listeners: {},
  exitEnabled: false,
})
const config = (
  defaultAction: "allow" | "deny" = "allow"
): Pick<ConfigurationView, "nodes" | "groups" | "topology" | "defaults"> => ({
  defaults: { access: "deny", topology: defaultAction, dialers: {} },
  nodes: {
    [A]: node("a", { type: "user", user: "a@example.com" }),
    [B]: node("b", { type: "tag", tags: ["tag:relay"] }),
    [C]: node("c", { type: "user", user: "c@example.com" }),
  },
  groups: { "group:clients": ["a@example.com", "c@example.com"] },
  topology: [],
})
const pairs = (input: ReturnType<typeof config>) =>
  configurationTopology(input).edges.map((edge) => [edge.a, edge.b])

test("empty rules use the real default: allow is complete, deny has no edges", () => {
  assert.deepEqual(pairs(config()), [
    [A, B],
    [A, C],
    [B, C],
  ])
  assert.deepEqual(pairs(config("deny")), [])
})

test("any matching block wins, including a later block and reversed ends", () => {
  const input = config()
  input.topology = [
    { action: "allow", a: "*", b: "*" },
    { action: "block", a: `node:${B}`, b: `node:${A}` },
  ]
  assert.deepEqual(pairs(input), [
    [A, C],
    [B, C],
  ])
  input.topology.reverse()
  assert.deepEqual(pairs(input), [
    [A, C],
    [B, C],
  ])
})

test("group, user and tag declarations expand from node identities, not tag ownership", () => {
  const input = config("deny")
  input.topology = [{ action: "allow", a: "group:clients", b: "tag:relay" }]
  assert.deepEqual(pairs(input), [
    [A, B],
    [B, C],
  ])
  input.topology = [{ action: "allow", a: "a@example.com", b: "c@example.com" }]
  assert.deepEqual(pairs(input), [[A, C]])
  input.nodes[A] = node("a", { type: "tag", tags: ["tag:relay"] })
  assert.deepEqual(pairs(input), [])
})

test("the first matching allow supplies the restriction; empty and unrestricted differ", () => {
  const input = config("deny")
  input.topology = [
    { action: "allow", a: "*", b: "*", transports: [] },
    { action: "allow", a: "*", b: "*", transports: ["native_tcp"] },
  ]
  const view = configurationTopology(input)
  assert.ok(
    view.edges.every((edge) => edge.rule === 0 && edge.transports?.length === 0)
  )
  assert.ok(
    mapConnections([], null, view).every(
      (edge) => edge.status === "unavailable"
    )
  )
  assert.ok(
    configurationTopology(config()).edges.every(
      (edge) => edge.rule === null && edge.transports === null
    )
  )
  input.topology = [
    {
      action: "allow",
      a: "*",
      b: "*",
      transports: ["vless", "native_tcp", "native_udp"],
    },
  ]
  assert.deepEqual(configurationTopology(input).edges[0]?.transports, [
    "vless",
    "native_tcp",
    "native_udp",
  ])
})

test("configuration edges and nodes survive absent runtime reports and missing listeners", () => {
  const input = config()
  const original = input.nodes[A]
  assert.ok(original)
  input.nodes[A] = { ...original, enabled: false }
  const declared = configurationTopology(input)
  const edges = mapConnections([], null, declared)
  assert.equal(edges.length, 3)
  assert.ok(edges.every((edge) => edge.status === "allowed"))
  const nodes = mapNodes(undefined, new Map(), () => null, input.nodes)
  assert.deepEqual(
    nodes.map((value) => value.name),
    ["a", "b", "c"]
  )
  assert.equal(overviewScene(nodes, edges).nodes.length, 3)
})

test("runtime channels annotate declared edges and cannot reintroduce a blocked edge", () => {
  const input = config()
  input.topology = [{ action: "block", a: `node:${A}`, b: `node:${B}` }]
  const ends: [string, string][] = [
    [A, B],
    [A, C],
  ]
  const links = ends.map(([a, b], index): AggregatedLink => ({
    a,
    b,
    linkId: String(index),
    phase: "authenticated",
    evidence: "oneSided",
    transport: "native_udp",
    initiatorNodeId: a,
    reports: [],
  }))
  const edges = mapConnections(links, null, configurationTopology(input))
  assert.deepEqual(
    edges.map((edge) => [edge.a, edge.b, edge.status]),
    [
      [A, C, "connected"],
      [B, C, "allowed"],
    ]
  )
})

test("large drawings have an explicit bounded window which keeps the selected node", () => {
  const input = config()
  for (let i = 4; i < TOPOLOGY_NODE_LIMIT + 10; i++)
    input.nodes[`00000000-0000-4000-8000-${String(i).padStart(12, "0")}`] =
      node(String(i), { type: "tag", tags: ["tag:relay"] })
  const result = configurationTopology(input, C)
  assert.equal(result.nodeIds.length, TOPOLOGY_NODE_LIMIT)
  assert.ok(result.totalNodes > result.nodeIds.length)
  assert.ok(result.nodeIds.includes(C))
  assert.equal(
    result.edges.length,
    (TOPOLOGY_NODE_LIMIT * (TOPOLOGY_NODE_LIMIT - 1)) / 2
  )
})
