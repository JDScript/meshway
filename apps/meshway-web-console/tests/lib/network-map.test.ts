import assert from "node:assert/strict"
import type {
  AggregatedLink,
  NodeInventoryView,
  NodeStateView,
  RecentLink,
  StateView,
} from "@meshway/control-client"
import { test } from "vitest"

import type { AddressGeo, GeoOf } from "@/lib/geo"
import {
  countBy,
  locationAddresses,
  mapConnections,
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
    netmap: null,
    canRevokeControlKey: true,
    canRevokeDataplaneKey: true,
    controlKeyCreatedAt: T,
    dataplaneKeyCreatedAt: T,
  },
]

/** A node whose location the server rests on `address`; its own GeoIP data are not used. */
const located = (address: string): NodeStateView => ({
  hostname: "located",
  freshness: "current",
  derived: {
    ipInfo: {},
    location: {
      address,
      data: { country: "US", position: { latitude: 1, longitude: 2 } },
      datasetId: "dddddddd-0000-4000-8000-000000000004",
      endpointSource: "stun",
      observedAt: "2026-10-03T00:00:00Z",
      expiresAt: "2026-10-03T00:05:00Z",
      sourceGeneration: 1,
      sourceSocketId: "s1",
    },
  },
})

/** What geojs says of an address: a country, and coordinates when given. */
const found = (lat?: number, lng?: number): AddressGeo => ({
  kind: "found",
  geo: {
    country: "JP",
    region: null,
    city: null,
    position:
      lat !== undefined && lng !== undefined
        ? { lat, lng, accuracyKm: null }
        : null,
    asn: null,
  },
})

const geoOf =
  (answers: Record<string, AddressGeo>): GeoOf =>
  (ip) =>
    answers[ip] ?? null

const TOKYO = "203.0.113.7"
const SEOUL = "203.0.113.8"
const PLACES = geoOf({ [TOKYO]: found(35.7, 139.7), [SEOUL]: found(37.5, 127) })

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

test("registered nodes without a State report remain visible with unknown status", () => {
  const nodes = mapNodes(
    state({}),
    new Map([listed(A, "new-node")]),
    () => null
  )
  assert.equal(nodes.length, 1)
  assert.equal(nodes[0]?.id, A)
  assert.equal(nodes[0]?.status, "unknown")
  assert.equal(nodes[0]?.location, null)
  assert.equal(nodes[0]?.receivedAt, null)
})

test("a node is located where geojs places the server's address; the others say why not", () => {
  const view = state({
    [A]: located(TOKYO),
    [B]: {
      hostname: "b",
      freshness: "awaiting_report",
      derived: { ipInfo: {}, locationAbsent: "noObservation" },
    },
    [C]: located("203.0.113.9"),
    "dddddddd-0000-4000-8000-000000000004": located("203.0.113.10"),
    "eeeeeeee-0000-4000-8000-000000000005": located("203.0.113.11"),
    "ffffffff-0000-4000-8000-000000000006": located("203.0.113.12"),
  })
  assert.deepEqual([...locationAddresses(view).keys()].sort(), [
    "203.0.113.10",
    "203.0.113.11",
    "203.0.113.12",
    TOKYO,
    "203.0.113.9",
  ])
  const nodes = mapNodes(
    view,
    new Map([listed(A, "tokyo"), listed(C, "seoul", false)]),
    geoOf({
      [TOKYO]: found(35.7, 139.7),
      "203.0.113.9": found(),
      "203.0.113.10": { kind: "off" },
      "203.0.113.11": { kind: "failed" },
      "203.0.113.12": { kind: "pending" },
    })
  )
  const byId = new Map(nodes.map((node) => [node.id, node.unlocated]))
  assert.equal(byId.get(A), null)
  assert.equal(byId.get(B), "noObservation")
  // A country without coordinates is not a place on the globe.
  assert.equal(byId.get(C), "noPosition")
  assert.equal(byId.get("dddddddd-0000-4000-8000-000000000004"), "lookupOff")
  assert.equal(byId.get("eeeeeeee-0000-4000-8000-000000000005"), "lookupFailed")
  // The server's own coordinates do not stand in while geojs is asked.
  assert.equal(
    byId.get("ffffffff-0000-4000-8000-000000000006"),
    "lookupPending"
  )
  const scene = overviewScene(nodes, [])
  assert.deepEqual(scene.nodes.find((node) => node.id === A)?.placement, {
    lat: 35.7,
    lng: 139.7,
    accuracyKm: null,
  })
})

test("a node is named by the inventory and takes its status from it", () => {
  const nodes = mapNodes(
    state({
      [A]: located(TOKYO),
      [B]: {
        hostname: "b",
        freshness: "awaiting_report",
        derived: { ipInfo: {}, locationAbsent: "noObservation" },
      },
      [C]: located(SEOUL),
    }),
    new Map([listed(A, "tokyo"), listed(C, "seoul", false)]),
    PLACES
  )
  const byId = new Map(nodes.map((node) => [node.id, node]))
  assert.equal(byId.get(A)?.name, "tokyo")
  assert.equal(byId.get(A)?.status, "current")
  // A peer the inventory does not list is named by the State.
  assert.equal(byId.get(B)?.name, "b")
  // A peer the inventory does not list: its report alone says its status.
  assert.equal(byId.get(B)?.status, "awaiting_report")
  // Turned off in the inventory, whatever its report says.
  assert.equal(byId.get(C)?.status, "disabled")
})

test("a link is drawn only between two located nodes", () => {
  const nodes = mapNodes(
    state({
      [A]: located(TOKYO),
      [B]: located(SEOUL),
      [C]: { hostname: "c", freshness: "current", derived: { ipInfo: {} } },
    }),
    new Map(),
    PLACES
  )
  const scene = overviewScene(
    nodes,
    mapConnections([link(A, B, "1"), link(A, C, "2")], null)
  )
  assert.deepEqual(
    scene.links.map((l) => l.nodeIds),
    [[A, B]]
  )
})

test("the maps show one adjacency for a visible current channel and its old predecessor", () => {
  const links = [
    link(A, B, "old", { evidence: "expired", transport: "native_tcp" }),
    link(A, B, "new", { evidence: "oneSided" }),
  ]
  const edges = mapConnections(links, null)
  assert.equal(edges.length, 1)
  assert.equal(edges[0]?.status, "connected")
  assert.equal(edges[0]?.look, "matched")
  assert.deepEqual(edges[0]?.transports, ["native_udp"])
  const nodes = mapNodes(
    state({ [A]: located(TOKYO), [B]: located(SEOUL) }),
    new Map(),
    PLACES
  )
  assert.deepEqual(
    overviewScene(nodes, mapConnections(links, null)).links.map((edge) => [
      edge.nodeIds,
      edge.look,
    ]),
    [[[A, B], "matched"]]
  )
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

test("the scene draws at most one edge per pair, with history only for pairs absent from current adjacency", () => {
  const nodes = mapNodes(
    state({
      [A]: located(TOKYO),
      [B]: located(SEOUL),
      [C]: { hostname: "c", freshness: "current", derived: { ipInfo: {} } },
    }),
    new Map(),
    PLACES
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
  const scene = overviewScene(nodes, mapConnections([link(A, B, "1")], history))
  assert.deepEqual(
    scene.links.map((l) => [l.look, l.lane]),
    [["matched", 0]]
  )
  assert.equal(
    overviewScene(nodes, mapConnections([link(A, B, "1")], null)).links.length,
    1
  )
  assert.equal(mapConnections([link(A, B, "1")], history).length, 2)
  assert.equal(scene.nodes.find((n) => n.id === C)?.placement, null)
})
