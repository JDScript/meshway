import assert from "node:assert/strict"
import type {
  NodeInventoryView,
  ObservationTask,
  StateView,
  UserConfigurationView,
  UserEntitiesView,
  UserEvent,
} from "@meshway/control-client"
import { test } from "vitest"

import {
  closedAction,
  failureAction,
  idleTimeout,
  initialModel,
  inventoryOf,
  reconnectDelay,
  step,
  type Model,
} from "@/api/live/reducer"

const NETWORK = "10000000-0000-4000-8000-000000000001"
const EPOCH = "20000000-0000-4000-8000-000000000002"
const NODE_A = "a0000000-0000-4000-8000-00000000000a"
const NODE_B = "b0000000-0000-4000-8000-00000000000b"
const REQUEST = "c0000000-0000-4000-8000-00000000000c"

const VLESS = {
  enabled: true,
  remark: "",
  connectTimeout: "5s",
  bindInterface: "",
  localAddress: "",
  socket: { receiveBuffer: 0, sendBuffer: 0 },
  tcp: { noDelay: true },
  stream: "auto" as const,
  tls: { trust: "managed" as const },
  websocketHeaderNames: [],
}

const manager: UserConfigurationView = {
  type: "manager",
  networkId: NETWORK,
  revision: 3,
  configuration: {
    access: [],
    addressPools: { ipv4: ["100.64.0.0/10"], ipv6: [] },
    defaults: {
      access: "deny",
      topology: "allow",
      dialers: { native_udp: {}, vless: VLESS, trojan: null },
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
    groups: {},
    nodes: {},
    routing: {},
    ruleSets: {},
    tagsOwners: {},
    topology: [],
  },
}
const member: UserConfigurationView = {
  type: "member",
  networkId: NETWORK,
  revision: 3,
  nodes: {},
}

const KEY = {
  generation: 1,
  id: "50000000-0000-4000-8000-000000000005",
  publicKey: {
    algorithm: "ed25519" as const,
    bytes: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
  },
}

const node = (id: string, hostname: string): NodeInventoryView => ({
  id,
  hostname,
  networkId: NETWORK,
  registeredAt: "2026-10-03T00:00:00Z",
  enabled: true,
  identity: { type: "tag", tags: ["tag:server"] },
  addresses: { ipv4: null, ipv6: null },
  keys: { control: KEY, dataplane: KEY },
  controlKeyRevokedAt: null,
  dataplaneKeyRevokedAt: null,
  controlKeyCreatedAt: "2026-10-03T00:00:00Z",
  dataplaneKeyCreatedAt: "2026-10-03T00:00:00Z",
  canDelete: true,
  canRevokeControlKey: true,
  canRevokeDataplaneKey: true,
  netmap: null,
  netmapUnavailable: { reason: "notIssued" },
  lease: null,
  lastSyncAt: null,
  serverKey: null,
  routing: {
    requestedProfile: null,
    appliedProfile: "mesh",
    locked: false,
    canSelect: true,
  },
})

const entities = (
  nodes: Record<string, NodeInventoryView> = {}
): UserEntitiesView => ({
  network: {
    id: NETWORK,
    name: "n",
    currentRevision: 3,
    createdAt: "2026-10-03T00:00:00Z",
    membershipId: "60000000-0000-4000-8000-000000000006",
    role: "owner",
    actions: {
      readConfiguration: true,
      editConfiguration: true,
      manageMembers: true,
      rename: true,
      joinLinkRoles: ["admin", "member"],
    },
  },
  nodes,
  members: {},
})

const state: StateView = {
  asOf: "2026-10-03T00:00:00Z",
  configurationRevision: 3,
  links: [],
  networkId: NETWORK,
  nodes: {},
  paths: [],
  sequence: 1,
  stateEpoch: EPOCH,
}

/** Runs events (with their SSE ids) and collects the effects. */
function run(events: [UserEvent, string?][], model: Model = initialModel) {
  const effects = []
  for (const [event, id] of events) {
    const next = step(model, event, id)
    model = next.model
    effects.push(...next.effects)
  }
  return { model, effects }
}

const snapshot = (
  batch: string,
  configuration: UserConfigurationView = manager
): [UserEvent, string?][] => [
  [{ type: "config.snapshot", batch, requestId: REQUEST, configuration }],
  [{ type: "entities.snapshot", batch, entities: entities() }],
  [{ type: "state.snapshot", batch, state }],
  [
    {
      type: "sync.ready",
      batch,
      cursor: batch,
      heartbeatIntervalMs: 10_000,
      disconnectAfterMs: 30_000,
    },
    batch,
  ],
]

test("the snapshot is installed only at sync.ready, all at once", () => {
  const partial = run(snapshot("b1").slice(0, 3))
  assert.deepEqual(partial.effects, [])
  assert.equal(partial.model.view, null)

  const { model, effects } = run(snapshot("b1"))
  assert.equal(effects.length, 1)
  assert.equal(effects[0]?.type, "install")
  // The snapshot's request ID: a manager's configuration is a GET's answer.
  assert.equal(
    effects[0]?.type === "install" ? effects[0].requestId : null,
    REQUEST
  )
  assert.equal(model.cursor, "b1")
  assert.equal(model.view?.revision, 3)
})

test("a snapshot from another batch or out of order resyncs", () => {
  const [config, , stateFrame] = snapshot("b1")
  const mixed = run([
    config!,
    [{ type: "entities.snapshot", batch: "b2", entities: entities() }],
  ])
  assert.deepEqual(mixed.effects, [{ type: "resync" }])
  const skipped = run([config!, stateFrame!])
  assert.deepEqual(skipped.effects, [{ type: "resync" }])
  const early = run([snapshot("b1")[3]!])
  assert.deepEqual(early.effects, [{ type: "resync" }])
})

test("a delta applies only on top of the last applied cursor", () => {
  const ready = run(snapshot("b1")).model
  const delta: UserEvent = {
    type: "state.delta",
    baseCursor: "b1",
    cursor: "c2",
    stateEpoch: EPOCH,
    updated: {},
    removed: [],
    links: [],
    paths: [],
  }
  const applied = run([[delta, "c2"]], ready)
  assert.equal(applied.model.cursor, "c2")
  assert.equal(applied.effects[0]?.type, "state")
  // Without its projection metadata the State keeps what it had.
  assert.equal(applied.model.view?.state.asOf, "2026-10-03T00:00:00Z")

  const projected = run(
    [
      [
        {
          ...delta,
          asOf: "2026-10-03T00:01:00Z",
          sequence: 7,
          configurationRevision: 4,
        },
        "c2",
      ],
    ],
    ready
  ).model.view?.state
  assert.equal(projected?.asOf, "2026-10-03T00:01:00Z")
  assert.equal(projected?.sequence, 7)
  assert.equal(projected?.configurationRevision, 4)

  const gap = run([[{ ...delta, baseCursor: "c1" }, "c2"]], ready)
  assert.deepEqual(gap.effects, [{ type: "resync" }])
  assert.equal(gap.model.cursor, null)

  const epoch = run(
    [[{ ...delta, stateEpoch: "30000000-0000-4000-8000-000000000003" }, "c2"]],
    ready
  )
  assert.deepEqual(epoch.effects, [{ type: "resync" }])
})

test("an entities delta replaces and removes whole entries", () => {
  const ready = run([
    [
      {
        type: "config.snapshot",
        batch: "b1",
        requestId: REQUEST,
        configuration: manager,
      },
    ],
    [
      {
        type: "entities.snapshot",
        batch: "b1",
        entities: entities({ [NODE_A]: node(NODE_A, "a") }),
      },
    ],
    ...snapshot("b1").slice(2),
  ]).model
  const { model, effects } = run(
    [
      [
        {
          type: "entities.delta",
          baseCursor: "b1",
          cursor: "c2",
          updated: entities({ [NODE_B]: node(NODE_B, "b") }),
          removed: { nodes: [NODE_A], members: [] },
        },
        "c2",
      ],
    ],
    ready
  )
  assert.deepEqual(Object.keys(model.view?.entities.nodes ?? {}), [NODE_B])
  assert.equal(effects[0]?.type, "entities")
})

test("notices move the cursor; a revision out of sequence resyncs", () => {
  const ready = run(snapshot("b1")).model
  const changed: UserEvent = {
    type: "configuration.changed",
    networkId: NETWORK,
    previousRevision: 3,
    revision: 4,
    cause: "edit",
    actor: { type: "system" },
    changedAt: "2026-10-03T00:00:00Z",
    changedPointers: [],
    changesComplete: true,
    sourceChanged: true,
  }
  const next = run([[changed, "c2"]], ready)
  assert.deepEqual(next.effects, [{ type: "configuration", revision: 4 }])
  assert.equal(next.model.cursor, "c2")
  assert.equal(next.model.view?.revision, 4)

  const skipped = run([[{ ...changed, previousRevision: 2 }, "c2"]], ready)
  assert.deepEqual(skipped.effects, [{ type: "resync" }])

  const noId = run(
    [[{ type: "resources.changed", networkId: NETWORK, revision: 3 }]],
    ready
  )
  assert.deepEqual(noId.effects, [{ type: "resync" }])
})

test("a member's configuration change moves the cursor; new settings come after a reset", () => {
  const ready = run(snapshot("b1", member)).model
  const { model, effects } = run(
    [
      [
        {
          type: "configuration.changed",
          networkId: NETWORK,
          previousRevision: 3,
          revision: 4,
          cause: "edit",
          actor: { type: "system" },
          changedAt: "2026-10-03T00:00:00Z",
          changedPointers: [],
          changesComplete: false,
          sourceChanged: false,
        },
        "c2",
      ],
    ],
    ready
  )
  assert.deepEqual(effects, [{ type: "configuration", revision: 4 }])
  assert.equal(model.cursor, "c2")
  assert.equal(model.view?.revision, 4)
})

test("a reset drops the view; the snapshot that follows installs again", () => {
  const ready = run(snapshot("b1")).model
  const reset = run([[{ type: "sync.reset", reason: "cursor_too_old" }]], ready)
  assert.deepEqual(reset.effects, [{ type: "unsynced" }])
  assert.equal(reset.model.cursor, null)
  const again = run(snapshot("b2"), reset.model)
  assert.equal(again.model.cursor, "b2")
  const install = again.effects[0]
  assert.equal(install?.type, "install")
})

test("observation events carry the task or the removal and move the cursor", () => {
  const ready = run(snapshot("b1")).model
  const task: ObservationTask = {
    id: "40000000-0000-4000-8000-000000000001",
    subscriptionId: "40000000-0000-4000-8000-000000000004",
    sourceNodeId: NODE_A,
    targetNodeId: NODE_B,
    service: { type: "icmpEcho" },
    selection: { mode: "auto" },
    netmap: {
      revision: 3,
      serial: 1,
      digest: "d".repeat(64),
    },
    instanceId: "40000000-0000-4000-8000-000000000006",
    intervalMs: 1000,
    metrics: ["rtt"],
    unsupportedMetrics: [],
    phase: "preparing",
    expiresAt: "2026-10-03T12:00:30Z",
    renewAfter: "2026-10-03T12:00:10Z",
  }
  const { model, effects } = run(
    [
      [{ type: "observation.status", observation: task }, "c2"],
      [{ type: "observation.sample", observation: task }, "c3"],
      [
        { type: "observation.removed", subscriptionId: task.subscriptionId! },
        "c4",
      ],
    ],
    ready
  )
  assert.deepEqual(effects, [
    { type: "observation", task },
    { type: "observation", task },
    { type: "observation-removed", subscriptionId: task.subscriptionId },
  ])
  assert.equal(model.cursor, "c4")
  assert.equal(model.view, ready.view)
})

test("nodes are listed in the controller's ID order", () => {
  const listed = inventoryOf({
    [NODE_B]: node(NODE_B, "b"),
    [NODE_A]: node(NODE_A, "a"),
  })
  assert.deepEqual(
    listed.items.map((item) => item.id),
    [NODE_A, NODE_B]
  )
})

test("failures and closes map to what the stream does next", () => {
  assert.equal(failureAction({ kind: "http", status: 401 }), "sign-out")
  assert.equal(failureAction({ kind: "http", status: 403 }), "lost")
  assert.equal(failureAction({ kind: "http", status: 404 }), "lost")
  assert.equal(failureAction({ kind: "http", status: 429 }), "retry")
  assert.equal(failureAction({ kind: "http", status: 503 }), "retry")
  assert.equal(failureAction({ kind: "http", status: 409 }), "resync")
  assert.equal(failureAction({ kind: "network" }), "retry")
  assert.equal(failureAction({ kind: "decode", status: 200 }), "resync")
  assert.equal(failureAction({ kind: "cancelled" }), "stop")
  assert.equal(closedAction("session_expired"), "sign-out")
  assert.equal(closedAction("membership_revoked"), "lost")
  assert.equal(closedAction("server_shutdown"), "retry")
})

test("reconnecting backs off, bounded", () => {
  assert.equal(reconnectDelay(0, 0), 500)
  assert.equal(reconnectDelay(0, 0.999_999), 1_000)
  assert.equal(reconnectDelay(3, 0.999_999), 8_000)
  assert.equal(reconnectDelay(40, 0.999_999), 30_000)
  assert.equal(idleTimeout(null), 30_000)
  assert.equal(idleTimeout(45_000), 45_000)
  assert.equal(idleTimeout(10 ** 9), 300_000)
})
