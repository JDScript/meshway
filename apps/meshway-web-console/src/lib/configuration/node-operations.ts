import type {
  ConfigurationOperation,
  ConfigurationView,
  DialerSetUpdate,
  ListenerUpdate,
  NodeSettingsPatch,
  NodeSettingsView,
} from "@meshway/control-client"

import { listenerFromView, writesSecrets } from "./listener-update"
import { canonical, dialerUpdate, sameDialers } from "./operations"

// What a node's workspace puts in the draft: per node, at most one
// operation per target holding its final value. Settings
// (`setNodeSettings`) carry only the fields that differ from the base;
// tags (`setIdentityTags`) and addresses (`setAddresses`, only the families
// that differ) likewise; a listener (`setListener`) is set, removed, or
// recreated (removed then set in the same batch, so the controller makes
// new material: the one way to rotate it); a deletion (`deleteNode`)
// replaces every other operation on the node. A value equal to the base
// drops its operations.

type NodeOp = Extract<ConfigurationOperation, { nodeId: string }>

const isNodeOp = (op: ConfigurationOperation): op is NodeOp => "nodeId" in op

/** The operations on `nodeId`. */
export const nodeOps = (
  ops: readonly ConfigurationOperation[],
  nodeId: string
): NodeOp[] => ops.filter(isNodeOp).filter((op) => op.nodeId === nodeId)

/** The node's settings in the base, if the base holds the node. */
const settingsIn = (base: ConfigurationView, nodeId: string) =>
  Object.hasOwn(base.nodes, nodeId) ? base.nodes[nodeId] : undefined

// Ordering ---------------------------------------------------------------

const KIND_RANK: Record<NodeOp["type"], number> = {
  setNodeSettings: 0,
  setIdentityTags: 1,
  setAddresses: 2,
  setListener: 3,
  deleteNode: 4,
}

/**
 * The operations with those on nodes after the rest, by node, kind and
 * listener key; a recreated listener's removal stays before its setting.
 */
function ordered(ops: readonly ConfigurationOperation[]) {
  const rest = ops.filter((op) => !isNodeOp(op))
  const nodes = ops
    .filter(isNodeOp)
    .map((op, index) => ({ op, index }))
    .sort((a, b) => {
      if (a.op.nodeId !== b.op.nodeId) return a.op.nodeId < b.op.nodeId ? -1 : 1
      const kind = KIND_RANK[a.op.type] - KIND_RANK[b.op.type]
      if (kind !== 0) return kind
      const keyA = a.op.type === "setListener" ? a.op.key : ""
      const keyB = b.op.type === "setListener" ? b.op.key : ""
      if (keyA !== keyB) return keyA < keyB ? -1 : 1
      return a.index - b.index
    })
    .map(({ op }) => op)
  return [...rest, ...nodes]
}

/** `ops` with the operations `match` picks replaced by `next`. */
const replacing = (
  ops: readonly ConfigurationOperation[],
  match: (op: NodeOp) => boolean,
  next: readonly ConfigurationOperation[]
) => ordered([...ops.filter((op) => !(isNodeOp(op) && match(op))), ...next])

// Deletion ---------------------------------------------------------------

export const isDeleted = (
  ops: readonly ConfigurationOperation[],
  nodeId: string
) => nodeOps(ops, nodeId).some((op) => op.type === "deleteNode")

/** Deleting takes the node's other operations with it; undeleting drops the deletion. */
export function withDeleted(
  ops: readonly ConfigurationOperation[],
  nodeId: string,
  deleted: boolean
): ConfigurationOperation[] {
  return deleted
    ? replacing(ops, (op) => op.nodeId === nodeId, [
        { type: "deleteNode", nodeId },
      ])
    : replacing(
        ops,
        (op) => op.nodeId === nodeId && op.type === "deleteNode",
        []
      )
}

// Settings ---------------------------------------------------------------

export type NodeSettingsValue = {
  hostname: string
  remark: string
  enabled: boolean
  exitEnabled: boolean
  /** Whether the node, as an exit, also carries IPv6 (`exitFamilies`). */
  exitIpv6: boolean
  /** ISO 3166-1 alpha-2 region `auto` resolves by; null when undeclared. */
  region: string | null
  /** null: inherits the network's defaults (the field is omitted). */
  dialers: DialerSetUpdate | null
}

export const settingsBase = (view: NodeSettingsView): NodeSettingsValue => ({
  hostname: view.hostname,
  remark: view.remark,
  enabled: view.enabled,
  exitEnabled: view.exitEnabled,
  exitIpv6: (view.exitFamilies ?? ["ipv4"]).includes("ipv6"),
  region: view.region ?? null,
  dialers: view.dialers ? dialerUpdate(view.dialers) : null,
})

const sameDialerChoice = (
  a: DialerSetUpdate | null,
  b: DialerSetUpdate | null
) => (a === null || b === null ? a === b : sameDialers(a, b))

/** The node's settings in the draft; null when the base does not hold the node. */
export function settingsOf(
  ops: readonly ConfigurationOperation[],
  base: ConfigurationView,
  nodeId: string
): NodeSettingsValue | null {
  const view = settingsIn(base, nodeId)
  if (!view) return null
  const value = settingsBase(view)
  const op = nodeOps(ops, nodeId).find(
    (candidate) => candidate.type === "setNodeSettings"
  )
  if (!op) return value
  const { fields } = op
  return {
    hostname: fields.hostname ?? value.hostname,
    remark: fields.remark ?? value.remark,
    enabled: fields.enabled ?? value.enabled,
    exitEnabled: fields.exitEnabled ?? value.exitEnabled,
    exitIpv6:
      fields.exitFamilies !== undefined
        ? fields.exitFamilies.includes("ipv6")
        : value.exitIpv6,
    region: fields.region !== undefined ? fields.region : value.region,
    dialers: fields.dialers !== undefined ? fields.dialers : value.dialers,
  }
}

/** The fields of `value` that differ from `base`. */
function settingsPatch(
  base: NodeSettingsValue,
  value: NodeSettingsValue
): NodeSettingsPatch {
  const patch: NodeSettingsPatch = {}
  if (value.hostname !== base.hostname) patch.hostname = value.hostname
  if (value.remark !== base.remark) patch.remark = value.remark
  if (value.enabled !== base.enabled) patch.enabled = value.enabled
  if (value.exitEnabled !== base.exitEnabled)
    patch.exitEnabled = value.exitEnabled
  if (value.exitIpv6 !== base.exitIpv6)
    patch.exitFamilies = value.exitIpv6 ? ["ipv4", "ipv6"] : ["ipv4"]
  if (value.region !== base.region) patch.region = value.region
  if (!sameDialerChoice(value.dialers, base.dialers))
    patch.dialers = value.dialers
  return patch
}

export function withSettings(
  ops: readonly ConfigurationOperation[],
  base: ConfigurationView,
  nodeId: string,
  value: NodeSettingsValue
): ConfigurationOperation[] {
  const view = settingsIn(base, nodeId)
  if (!view || isDeleted(ops, nodeId)) return [...ops]
  const fields = settingsPatch(settingsBase(view), value)
  return replacing(
    ops,
    (op) => op.nodeId === nodeId && op.type === "setNodeSettings",
    Object.keys(fields).length === 0
      ? []
      : [{ type: "setNodeSettings", nodeId, fields }]
  )
}

// Identity tags ----------------------------------------------------------

const asSet = (values: readonly string[]) => [...new Set(values)].sort()

/** The node's tags in the draft; null for a User identity no operation gives tags. */
export function tagsOf(
  ops: readonly ConfigurationOperation[],
  base: ConfigurationView,
  nodeId: string
): string[] | null {
  const op = nodeOps(ops, nodeId).find(
    (candidate) => candidate.type === "setIdentityTags"
  )
  if (op) return op.tags
  const identity = settingsIn(base, nodeId)?.identity
  return identity?.type === "tag" ? identity.tags : null
}

/**
 * The node's tags set to `tags`. For a User identity, tags make it a Tag
 * identity; none (or null) keeps it a User identity. The controller decides
 * who may assign which tags, and refuses taking a Tag identity's last tag.
 */
export function withTags(
  ops: readonly ConfigurationOperation[],
  base: ConfigurationView,
  nodeId: string,
  tags: readonly string[] | null
): ConfigurationOperation[] {
  const identity = settingsIn(base, nodeId)?.identity
  if (!identity || isDeleted(ops, nodeId)) return [...ops]
  const next = tags === null ? null : asSet(tags)
  const unchanged =
    identity.type === "tag"
      ? next !== null && canonical(next) === canonical(asSet(identity.tags))
      : next === null || next.length === 0
  return replacing(
    ops,
    (op) => op.nodeId === nodeId && op.type === "setIdentityTags",
    unchanged || next === null
      ? []
      : [{ type: "setIdentityTags", nodeId, tags: next }]
  )
}

// Addresses --------------------------------------------------------------

export type AddressesValue = { ipv4: string | null; ipv6: string | null }

export function addressesOf(
  ops: readonly ConfigurationOperation[],
  base: ConfigurationView,
  nodeId: string
): AddressesValue | null {
  const view = settingsIn(base, nodeId)
  if (!view) return null
  const value = { ...view.addresses }
  const op = nodeOps(ops, nodeId).find(
    (candidate) => candidate.type === "setAddresses"
  )
  if (!op) return value
  return {
    ipv4: op.addresses.ipv4 !== undefined ? op.addresses.ipv4 : value.ipv4,
    ipv6: op.addresses.ipv6 !== undefined ? op.addresses.ipv6 : value.ipv6,
  }
}

/** The node's addresses set to `value`: an operation naming only the families that differ (null clears one). */
export function withAddresses(
  ops: readonly ConfigurationOperation[],
  base: ConfigurationView,
  nodeId: string,
  value: AddressesValue
): ConfigurationOperation[] {
  const view = settingsIn(base, nodeId)
  if (!view || isDeleted(ops, nodeId)) return [...ops]
  const addresses: { ipv4?: string | null; ipv6?: string | null } = {}
  if (value.ipv4 !== view.addresses.ipv4) addresses.ipv4 = value.ipv4
  if (value.ipv6 !== view.addresses.ipv6) addresses.ipv6 = value.ipv6
  return replacing(
    ops,
    (op) => op.nodeId === nodeId && op.type === "setAddresses",
    Object.keys(addresses).length === 0
      ? []
      : [{ type: "setAddresses", nodeId, addresses }]
  )
}

// Listeners --------------------------------------------------------------

/** A listener's edit: removed, or set (`recreate`: new material, removed then set). */
export type ListenerEdit =
  | { kind: "remove" }
  | { kind: "set"; listener: ListenerUpdate; recreate: boolean }

/** The listener edits the draft holds for the node, by key. */
export function listenerEdits(
  ops: readonly ConfigurationOperation[],
  nodeId: string
): Map<string, ListenerEdit> {
  const out = new Map<string, ListenerEdit>()
  for (const op of nodeOps(ops, nodeId)) {
    if (op.type !== "setListener") continue
    const before = out.get(op.key)
    if (op.listener === null) out.set(op.key, { kind: "remove" })
    else
      out.set(op.key, {
        kind: "set",
        listener: op.listener,
        recreate: before?.kind === "remove",
      })
  }
  return out
}

/** The listener keys the node has in the draft: the base's not removed, and those added. */
export function listenerKeys(
  ops: readonly ConfigurationOperation[],
  base: ConfigurationView,
  nodeId: string
): string[] {
  const keys = new Set(Object.keys(settingsIn(base, nodeId)?.listeners ?? {}))
  for (const [key, edit] of listenerEdits(ops, nodeId))
    if (edit.kind === "remove") keys.delete(key)
    else keys.add(key)
  return [...keys].sort()
}

/**
 * The listener under `key` set to `edit` (null: as in the base). Removing
 * one the base does not hold, or setting one to exactly what the view
 * restates (no new secrets, no recreation), drops its operations.
 */
export function withListener(
  ops: readonly ConfigurationOperation[],
  base: ConfigurationView,
  nodeId: string,
  key: string,
  edit: ListenerEdit | null
): ConfigurationOperation[] {
  const settings = settingsIn(base, nodeId)
  if (!settings || isDeleted(ops, nodeId)) return [...ops]
  const view = Object.hasOwn(settings.listeners, key)
    ? settings.listeners[key]
    : undefined
  const match = (op: NodeOp) =>
    op.nodeId === nodeId && op.type === "setListener" && op.key === key
  if (edit === null) return replacing(ops, match, [])
  if (edit.kind === "remove")
    return replacing(
      ops,
      match,
      view ? [{ type: "setListener", nodeId, key, listener: null }] : []
    )
  const restated = view ? listenerFromView(view) : null
  const unchanged =
    !edit.recreate &&
    restated !== null &&
    !writesSecrets(edit.listener) &&
    canonical(restated) === canonical(edit.listener)
  if (unchanged) return replacing(ops, match, [])
  const set: ConfigurationOperation = {
    type: "setListener",
    nodeId,
    key,
    listener: edit.listener,
  }
  return replacing(
    ops,
    match,
    edit.recreate && view
      ? [{ type: "setListener", nodeId, key, listener: null }, set]
      : [set]
  )
}

/** Whether the draft changes the node at all. */
export const editsNode = (
  ops: readonly ConfigurationOperation[],
  nodeId: string
) => nodeOps(ops, nodeId).length > 0
