import type {
  ClientFailure,
  ClosedReason,
  ConfigurationApplyProgress,
  ErrorResponse,
  NodeInventoryResponse,
  NodeInventoryView,
  ObservationTask,
  StateView,
  UserConfigurationView,
  UserEntitiesView,
  UserEvent,
} from "@meshway/control-client"

// The network event stream (`watchNetwork`) as plain data: what one decoded
// event does to this tab's view of the network, and what the cache must do
// about it. The initial snapshots are staged and installed together at
// `sync.ready`; every later event must continue from the last applied one,
// else the view is dropped and synced afresh. No I/O here (`stream.ts`).

/** One complete, installed view of the network, as the stream delivered it. */
export type View = {
  configuration: UserConfigurationView
  entities: UserEntitiesView
  state: StateView
  /** The configuration revision the stream is at (its view or later notices). */
  revision: number
}

type Staged = {
  batch: string
  /** The request carrying the stream, as a GET's answer names its own. */
  requestId: string
  configuration: UserConfigurationView
  progress: ConfigurationApplyProgress | null
  entities?: UserEntitiesView
  state?: StateView
}

export type Model = {
  /** The last event whose effects were all applied; resuming sends it. */
  cursor: string | null
  /** null until a sync is ready, and again after a reset. */
  view: View | null
  staged: Staged | null
}

export const initialModel: Model = { cursor: null, view: null, staged: null }

export type Effect =
  /** A complete sync is ready: replace every read it covers. */
  | {
      type: "install"
      view: View
      /** The previous view, when this sync replaced one. */
      previous: View | null
      progress: ConfigurationApplyProgress | null
      /** The sync's request ID: a manager's configuration is then the GET's answer. */
      requestId: string
      disconnectAfterMs: number
    }
  | { type: "entities"; previous: UserEntitiesView; next: UserEntitiesView }
  | { type: "state"; state: StateView }
  | { type: "configuration"; revision: number }
  | { type: "progress"; progress: ConfigurationApplyProgress }
  | { type: "resources" }
  /** This session's observation subscription, as the controller now shows it. */
  | { type: "observation"; task: ObservationTask }
  /** The subscription ended or its terminal view was dropped. */
  | { type: "observation-removed"; subscriptionId: string }
  /** The view is no longer current (a reset); the server syncs it again. */
  | { type: "unsynced" }
  /** The stream broke its own continuity: reconnect without a cursor. */
  | { type: "resync" }
  | { type: "closed"; reason: ClosedReason }

export type Step = { model: Model; effects: Effect[] }

const resync = (): Step => ({
  model: initialModel,
  effects: [{ type: "resync" }],
})

function replaceEntries<Value>(
  current: Readonly<Record<string, Value>>,
  updated: Readonly<Record<string, Value>>,
  removed: readonly string[]
): Record<string, Value> {
  const next: Record<string, Value> = { ...current }
  for (const id of removed) delete next[id]
  return Object.assign(next, updated)
}

/**
 * `state.delta`: each updated node replaced whole, removed ones dropped,
 * links and paths replaced; the projection's time, sequence and revision
 * replaced when the delta carries them (an older one may not).
 */
function applyStateDelta(
  state: StateView,
  delta: Extract<UserEvent, { type: "state.delta" }>
): StateView {
  return {
    ...state,
    stateEpoch: delta.stateEpoch,
    asOf: delta.asOf ?? state.asOf,
    sequence: delta.sequence ?? state.sequence,
    configurationRevision:
      delta.configurationRevision ?? state.configurationRevision,
    nodes: replaceEntries(state.nodes, delta.updated, delta.removed),
    links: delta.links,
    adjacencies: delta.adjacencies,
    paths: delta.paths,
  }
}

/** `entities.delta`: each updated node and member replaced whole, removed ones dropped. */
function applyEntitiesDelta(
  entities: UserEntitiesView,
  delta: Extract<UserEvent, { type: "entities.delta" }>
): UserEntitiesView {
  return {
    network: delta.updated.network,
    nodes: replaceEntries(
      entities.nodes,
      delta.updated.nodes,
      delta.removed.nodes
    ),
    members: replaceEntries(
      entities.members,
      delta.updated.members,
      delta.removed.members
    ),
  }
}

/** The stream's nodes as `listNodes` returns them: ordered by ID, as the controller orders them. */
export function inventoryOf(
  nodes: Readonly<Record<string, NodeInventoryView>>
): NodeInventoryResponse {
  return {
    items: Object.values(nodes).sort((a, b) =>
      a.id < b.id ? -1 : a.id > b.id ? 1 : 0
    ),
  }
}

/** Applies one decoded event, delivered with SSE `id`. */
export function step(model: Model, event: UserEvent, id?: string): Step {
  switch (event.type) {
    case "config.snapshot": {
      const staged: Staged = {
        batch: event.batch,
        requestId: event.requestId,
        configuration: event.configuration,
        progress: event.progress ?? null,
      }
      // A fresh snapshot without a reset first still replaces the view.
      if (model.view)
        return {
          model: { cursor: null, view: null, staged },
          effects: [{ type: "unsynced" }],
        }
      return { model: { ...model, staged }, effects: [] }
    }
    case "entities.snapshot": {
      const staged = model.staged
      if (!staged || staged.batch !== event.batch || staged.entities)
        return resync()
      return {
        model: { ...model, staged: { ...staged, entities: event.entities } },
        effects: [],
      }
    }
    case "state.snapshot": {
      const staged = model.staged
      if (
        !staged ||
        staged.batch !== event.batch ||
        !staged.entities ||
        staged.state
      )
        return resync()
      return {
        model: { ...model, staged: { ...staged, state: event.state } },
        effects: [],
      }
    }
    case "sync.ready": {
      const staged = model.staged
      if (
        !staged?.entities ||
        !staged.state ||
        staged.batch !== event.batch ||
        (id !== undefined && id !== event.cursor)
      )
        return resync()
      const view: View = {
        configuration: staged.configuration,
        entities: staged.entities,
        state: staged.state,
        revision: staged.configuration.revision,
      }
      return {
        model: { cursor: event.cursor, view, staged: null },
        effects: [
          {
            type: "install",
            view,
            previous: model.view,
            progress: staged.progress,
            requestId: staged.requestId,
            disconnectAfterMs: event.disconnectAfterMs,
          },
        ],
      }
    }
    case "sync.reset":
      return { model: initialModel, effects: [{ type: "unsynced" }] }
    case "stream.closed":
      return {
        model,
        effects: [{ type: "closed", reason: event.reason }],
      }
  }

  // Everything else continues an installed view and carries its cursor.
  const view = model.view
  if (!view || model.staged || id === undefined) return resync()
  const at = (next: View): Model => ({ cursor: id, view: next, staged: null })

  switch (event.type) {
    case "state.delta": {
      if (
        event.baseCursor !== model.cursor ||
        event.cursor !== id ||
        event.stateEpoch !== view.state.stateEpoch
      )
        return resync()
      const state = applyStateDelta(view.state, event)
      return {
        model: at({ ...view, state }),
        effects: [{ type: "state", state }],
      }
    }
    case "entities.delta": {
      if (event.baseCursor !== model.cursor || event.cursor !== id)
        return resync()
      const entities = applyEntitiesDelta(view.entities, event)
      return {
        model: at({ ...view, entities }),
        effects: [
          { type: "entities", previous: view.entities, next: entities },
        ],
      }
    }
    case "configuration.changed": {
      if (event.previousRevision !== view.revision) return resync()
      // A member's own settings, when they change, come in a new batch
      // after `sync.reset`; the controller sends it.
      return {
        model: at({ ...view, revision: event.revision }),
        effects: [{ type: "configuration", revision: event.revision }],
      }
    }
    case "configuration.progress":
      return {
        model: at(view),
        effects: [{ type: "progress", progress: event.progress }],
      }
    case "resources.changed":
      return { model: at(view), effects: [{ type: "resources" }] }
    case "observation.status":
    case "observation.sample":
      return {
        model: at(view),
        effects: [{ type: "observation", task: event.observation }],
      }
    case "observation.removed":
      return {
        model: at(view),
        effects: [
          {
            type: "observation-removed",
            subscriptionId: event.subscriptionId,
          },
        ],
      }
  }
}

/** What a failed connection means for the stream. */
export type FailureAction =
  /** The session is rejected: sign out, as every read does on 401. */
  | "sign-out"
  /** This account cannot (or can no longer) read the network. */
  | "lost"
  /** Try again later, resuming from the cursor. */
  | "retry"
  /** Try again later from a full sync: the cursor or a frame was refused. */
  | "resync"
  /** Cancelled, or a request this console built wrongly: stop. */
  | "stop"

export function failureAction(
  failure: ClientFailure<ErrorResponse>
): FailureAction {
  switch (failure.kind) {
    case "cancelled":
    case "configuration":
    case "request":
      return "stop"
    case "network":
      return "retry"
    case "decode":
    case "body_limit":
      return "resync"
    case "http":
      if (failure.status === 401) return "sign-out"
      if (failure.status === 403 || failure.status === 404) return "lost"
      if (failure.status === 429 || failure.status >= 500) return "retry"
      return "resync"
  }
}

/** What the controller closing the stream means for it. */
export function closedAction(reason: ClosedReason): FailureAction {
  switch (reason) {
    case "session_expired":
      return "sign-out"
    case "membership_revoked":
      return "lost"
    case "server_shutdown":
    case "node_lease_expired":
      return "retry"
  }
}

const FIRST_DELAY_MS = 1_000
const MAX_DELAY_MS = 30_000

/**
 * How long to wait before reconnect `attempt` (0 first): doubling from one
 * second up to thirty, each between half and all of that so tabs spread out.
 * `random` is in [0, 1).
 */
export function reconnectDelay(attempt: number, random: number): number {
  const ceiling = Math.min(
    MAX_DELAY_MS,
    FIRST_DELAY_MS * 2 ** Math.min(attempt, 5)
  )
  return Math.round(ceiling * (0.5 + random / 2))
}

/** The idle bound for reading the stream: the controller's, within what the client accepts. */
export function idleTimeout(disconnectAfterMs: number | null): number {
  if (disconnectAfterMs === null || disconnectAfterMs < 1_000) return 30_000
  return Math.min(disconnectAfterMs, 300_000)
}
