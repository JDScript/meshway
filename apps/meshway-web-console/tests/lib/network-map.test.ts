import assert from "node:assert/strict"
import type {
  AggregatedLink,
  NodeInventoryView,
  NodeStateView,
  RecentLink,
  StateView,
} from "@meshway/control-client"
import { test } from "vitest"

import {
  countBy,
  linkLook,
  mapNodes,
  mergeRecentLinks,
  overviewScene,
} from "@/lib/network-map"
import { STATUS_ORDER } from "@/lib/nodes/status"

const A = "aaaaaaaa-0000-4000-8000-000000000001"
const B = "bbbbbbbb-0000-4000-8000-000000000002"
const C = "cccccccc-0000-4000-8000-000000000003"

const T = "2026-10-01T00:00:00Z"

/** A node as the inventory lists it, by its hostname. */
const listed = (
  id: string,
  hostname: string,
  enabled = true
): [string, NodeInventoryView] => [
  id,
  {
    id,
    networkId: "eeeeeeee-0000-4000-8000-000000000005",
    registeredAt: T,
    hostname,
    enabled,
    identity: { type: "user", user: "owner@example.com" },
    addresses: { ipv4: null, ipv6: null },
    keys: {
      control: {
        id: "k1",
        generation: 1,
        publicKey: { algorithm: "ed25519", bytes: "00".repeat(32) },
      },
      dataplane: {
        id: "k2",
        generation: 1,
        publicKey: { algorithm: "x25519", bytes: "00".repeat(32) },
      },
    },
    controlKeyRevokedAt: null,
    dataplaneKeyRevokedAt: null,
    canDelete: true,
    desiredExecution: null,
    canRevokeControlKey: true,
    canRevokeDataplaneKey: true,
    controlKeyCreatedAt: T,
    dataplaneKeyCreatedAt: T,
  },
]

const located = (lat: number, lng: number): NodeStateView => ({
  hostname: "located",
  freshness: "current",
  derived: {
    ipInfo: {},
    location: {
      address: "203.0.113.7",
      data: { country: "JP", position: { latitude: lat, longitude: lng } },
      datasetId: "dddddddd-0000-4000-8000-000000000004",
      endpointSource: "stun",
      observedAt: "2026-10-03T00:00:00Z",
      expiresAt: "2026-10-03T00:05:00Z",
      sourceGeneration: 1,
      sourceSocketId: "s1",
    },
  },
})

const state = (
  nodes: Record<string, NodeStateView>,
  links: AggregatedLink[] = []
): StateView => ({
  asOf: "2026-10-03T00:01:00Z",
  configurationRevision: 3,
  links,
  networkId: "eeeeeeee-0000-4000-8000-000000000005",
  nodes,
  paths: [],
  sequence: 9,
  stateEpoch: "ffffffff-0000-4000-8000-000000000006",
})

const link = (
  a: string,
  b: string,
  linkId: string,
  patch: Partial<AggregatedLink> = {}
): AggregatedLink => ({
  a,
  b,
  linkId,
  evidence: "matched",
  phase: "authenticated",
  initiatorNodeId: a,
  reports: [],
  transport: "native_udp",
  ...patch,
})

test("only a node with the controller's coordinates is located; the others keep the controller's reason", () => {
  const nodes = mapNodes(
    state({
      [A]: located(35.7, 139.7),
      [B]: {
        hostname: "b",
        freshness: "awaiting_report",
        derived: { ipInfo: {}, locationAbsent: "datasetUnavailable" },
      },
      [C]: {
        hostname: "c",
        freshness: "current",
        derived: {
          ...located(0, 0).derived,
          location: {
            ...located(0, 0).derived.location!,
            data: { country: "KR", position: null },
          },
        },
      },
    }),
    new Map([listed(A, "tokyo"), listed(C, "seoul", false)])
  )
  const byId = new Map(nodes.map((node) => [node.id, node]))
  assert.equal(byId.get(A)?.unlocated, null)
  assert.equal(byId.get(A)?.name, "tokyo")
  assert.equal(byId.get(A)?.status, "current")
  assert.equal(byId.get(B)?.unlocated, "datasetUnavailable")
  assert.equal(byId.get(B)?.name, "bbbbbbbb")
  // A peer the inventory does not list: its report alone says its status.
  assert.equal(byId.get(B)?.status, "awaiting_report")
  // Turned off in the inventory, whatever its report says.
  assert.equal(byId.get(C)?.status, "disabled")
  // A country without coordinates is not a place on the globe.
  assert.equal(byId.get(C)?.unlocated, "noPosition")
})

test("a link is drawn only between two located nodes", () => {
  const nodes = mapNodes(
    state({
      [A]: located(35.7, 139.7),
      [B]: located(37.5, 127),
      [C]: { hostname: "c", freshness: "current", derived: { ipInfo: {} } },
    }),
    new Map()
  )
  const scene = overviewScene(nodes, [link(A, B, "1"), link(A, C, "2")], null)
  assert.deepEqual(
    scene.links.map((l) => l.nodeIds),
    [[A, B]]
  )
})

test("a link's look follows its evidence before its phase", () => {
  assert.equal(linkLook(link(A, B, "1")), "matched")
  assert.equal(linkLook(link(A, B, "1", { evidence: "oneSided" })), "one_sided")
  assert.equal(
    linkLook(link(A, B, "1", { evidence: "expired", phase: "authenticated" })),
    "expired"
  )
  assert.equal(
    linkLook(link(A, B, "1", { evidence: "conflicting" })),
    "conflicting"
  )
  assert.equal(
    linkLook(link(A, B, "1", { phase: "preparing", evidence: "oneSided" })),
    "preparing"
  )
  assert.equal(linkLook(link(A, B, "1", { phase: "failed" })), "failed")
})

test("counts keep the given order and leave out empty values", () => {
  assert.deepEqual(countBy(["expired", "current", "current"], STATUS_ORDER), [
    { value: "current", count: 2 },
    { value: "expired", count: 1 },
  ])
})

test("both ends' records of one link merge; the latest event decides open or closed", () => {
  const items: RecentLink[] = [
    {
      sourceNodeId: B,
      peerNodeId: A,
      linkId: "1",
      transport: "native_udp",
      lastEventAt: "2026-10-03T00:00:10Z",
      closed: false,
    },
    {
      sourceNodeId: A,
      peerNodeId: B,
      linkId: "1",
      transport: "native_udp",
      lastEventAt: "2026-10-03T00:00:30Z",
      closed: true,
    },
    {
      sourceNodeId: A,
      peerNodeId: C,
      linkId: "2",
      transport: "native_tcp",
      lastEventAt: "2026-10-03T00:00:20Z",
      closed: false,
    },
  ]
  const merged = mergeRecentLinks(items)
  assert.equal(merged.length, 2)
  assert.deepEqual(merged[0], {
    a: A,
    b: B,
    linkId: "1",
    transport: "native_udp",
    lastEventAt: "2026-10-03T00:00:30Z",
    closed: true,
    sources: [B, A],
  })
})

test("the scene draws history links apart from current ones, never both for one link, in lanes", () => {
  const nodes = mapNodes(
    state({
      [A]: located(35.7, 139.7),
      [B]: located(37.5, 127),
      [C]: { hostname: "c", freshness: "current", derived: { ipInfo: {} } },
    }),
    new Map()
  )
  const history = mergeRecentLinks([
    {
      sourceNodeId: A,
      peerNodeId: B,
      linkId: "1",
      transport: "native_udp",
      lastEventAt: "2026-10-03T00:00:10Z",
      closed: false,
    },
    {
      sourceNodeId: A,
      peerNodeId: B,
      linkId: "old",
      transport: "native_udp",
      lastEventAt: "2026-10-03T00:00:05Z",
      closed: true,
    },
    {
      sourceNodeId: A,
      peerNodeId: C,
      linkId: "3",
      transport: "native_udp",
      lastEventAt: "2026-10-03T00:00:05Z",
      closed: true,
    },
  ])
  const scene = overviewScene(nodes, [link(A, B, "1")], history)
  assert.deepEqual(
    scene.links.map((l) => [l.look, l.lane]),
    [
      ["matched", 0],
      ["history", 1],
    ]
  )
  assert.equal(overviewScene(nodes, [link(A, B, "1")], null).links.length, 1)
  assert.equal(scene.nodes.find((n) => n.id === C)?.placement, null)
})
