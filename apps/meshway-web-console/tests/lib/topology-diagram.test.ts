import assert from "node:assert/strict"
import type {
  ConfigurationView,
  ListenerView,
  NodeSettingsView,
} from "@meshway/control-client"
import { test } from "vitest"

import { edgeDirections } from "@/lib/configuration/topology-view"
import type { MapConnection, MapNode } from "@/lib/network-map"
import type { NodeRoute } from "@/lib/network-routes"
import { placeOrder, topologyDiagram } from "@/lib/topology-diagram"
import { focusLayout, overviewLayout } from "@/lib/topology-layout"

const A = "11111111-1111-4111-8111-111111111111"
const R = "22222222-2222-4222-8222-222222222222"
const B = "33333333-3333-4333-8333-333333333333"
const BOX = { width: 900, height: 640 }

const mapNode = (id: string, name: string, lng: number | null): MapNode => ({
  id,
  name,
  status: "current",
  location: null,
  geo:
    lng === null
      ? null
      : {
          kind: "found",
          geo: {
            country: null,
            region: null,
            city: null,
            position: { lat: 0, lng, accuracyKm: null },
            asn: null,
          },
        },
  unlocated: lng === null ? "unknown" : null,
  receivedAt: null,
  lastAuthenticatedAt: null,
})

const edge = (a: string, b: string): MapConnection => ({
  key: a < b ? `${a}|${b}` : `${b}|${a}`,
  a: a < b ? a : b,
  b: a < b ? b : a,
  look: "allowed",
  status: "allowed",
  transports: [],
  inUse: false,
  channels: [],
})

test("nodes are ordered by longitude, unplaced ones after by name", () => {
  const order = placeOrder([
    mapNode(B, "sha", 121),
    mapNode(R, "zz", null),
    mapNode(A, "lax", -118),
    mapNode("44444444-4444-4444-8444-444444444444", "aa", null),
  ]).map((node) => node.name)
  assert.deepEqual(order, ["lax", "sha", "aa", "zz"])
})

test("layouts depend only on order and box, and stay inside it", () => {
  const ids = Array.from(
    { length: 30 },
    (_, index) => `00000000-0000-4000-8000-${String(index).padStart(12, "0")}`
  )
  const first = overviewLayout(ids, BOX)
  assert.deepEqual(first, overviewLayout(ids, BOX))
  for (const point of first.values()) {
    assert.ok(point.x >= 0 && point.x <= BOX.width)
    assert.ok(point.y >= 0 && point.y <= BOX.height)
  }
  assert.equal(
    new Set([...first.values()].map((p) => `${p.x},${p.y}`)).size,
    30
  )
})

test("the focused layout puts the source in the middle and each node on its hop ring", () => {
  const points = focusLayout(
    A,
    [A, R, B],
    {
      depth: new Map([
        [R, 1],
        [B, 2],
      ]),
      parent: new Map([
        [R, A],
        [B, R],
      ]),
    },
    BOX
  )
  const centre = points.get(A)
  const distance = (id: string) => {
    const point = points.get(id)
    assert.ok(point && centre)
    return Math.hypot(point.x - centre.x, point.y - centre.y)
  }
  assert.ok(distance(R) > 0)
  assert.ok(distance(B) > distance(R))
})

test("a selected node draws its planned hops, pointing the way traffic goes", () => {
  const nodes = [mapNode(A, "a", 0), mapNode(R, "r", 10), mapNode(B, "b", 20)]
  const routes: NodeRoute[] = [
    {
      target: R,
      status: "planned",
      hops: [{ from: A, to: R, topology: null }],
      authorized: true,
      outcome: { type: "permitted" },
      problem: null,
    },
    {
      target: B,
      status: "planned",
      hops: [
        { from: A, to: R, topology: null },
        { from: R, to: B, topology: null },
      ],
      authorized: true,
      outcome: { type: "permitted" },
      problem: null,
    },
  ]
  const base = {
    nodes,
    edges: [edge(A, R), edge(R, B), edge(A, B)],
    flows: new Map<string, Set<string>>(),
    link: null,
    box: BOX,
  }
  const overview = topologyDiagram({ ...base, routes: null, selected: null })
  assert.equal(overview.edges.length, 3)
  assert.ok(overview.edges.every((item) => item.direction === null))
  const focused = topologyDiagram({ ...base, routes, selected: A })
  assert.deepEqual(
    focused.edges.map((item) => item.direction),
    [
      { from: A, to: R },
      { from: R, to: B },
    ]
  )
  // A selected link that is not a planned hop is still drawn.
  const withLink = topologyDiagram({
    ...base,
    routes,
    selected: A,
    link: [B, A],
  })
  assert.ok(
    withLink.edges.some((item) => item.selected && item.direction === null)
  )
})

const listener = (protocol: ListenerView["protocol"]): ListenerView => ({
  bindInterface: "",
  credentialPresent: true,
  enabled: true,
  endpoints: [],
  listen: "0.0.0.0",
  listenPort: 443,
  protocol,
  remark: "",
  socket: {},
})

const settings = (
  dialers: NodeSettingsView["dialers"],
  listeners: Record<string, ListenerView>
): NodeSettingsView => ({
  hostname: "n",
  identity: { type: "tag", tags: ["tag:n"] },
  enabled: true,
  exitEnabled: false,
  remark: "",
  addresses: { ipv4: null, ipv6: null },
  listeners,
  ...(dialers === undefined ? {} : { dialers }),
})

test("each dial direction is the edge's order ∩ the dialer's dialers ∩ the listener's listeners", () => {
  const config: Pick<ConfigurationView, "nodes" | "defaults"> = {
    defaults: {
      access: "deny",
      topology: "allow",
      dialers: { native_udp: {}, native_tcp: {} },
    },
    nodes: {
      // Inherits the defaults.
      [A]: settings(undefined, { udp: listener("native_udp") }),
      // Dials nothing.
      [B]: settings(
        {},
        {
          tcp: listener("native_tcp"),
          vless: listener("native_udp"),
        }
      ),
    },
  }
  const [aToB, bToA] = edgeDirections(config, {
    a: A,
    b: B,
    transports: ["native_tcp", "native_udp"],
  })
  assert.deepEqual(aToB.transports, ["native_tcp", "native_udp"])
  assert.equal(bToA.missing, "dialer")
  const [none] = edgeDirections(config, { a: A, b: B, transports: [] })
  assert.equal(none.missing, "edge")
  const [noListener] = edgeDirections(config, {
    a: A,
    b: B,
    transports: ["vless"],
  })
  assert.equal(noListener.missing, "dialer")
})
