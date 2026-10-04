import type {
  DialerSetUpdate,
  DialerStream,
  TransportProtocol,
} from "@meshway/control-client"
import { zDialerSetUpdate } from "@meshway/control-client/schemas"

// A dialer set as the forms edit it (`defaults.dialers`, a node's
// `dialers`): which protocols a node may dial with, their priority (lower
// first), connect timeout and whether each is on. Keys are protocol names
// (docs/v2/configuration-reference/dialers.md). A protocol added here is
// `{}`: the controller fills its defaults. Fields the forms do not show
// (bind interface, local address, socket, TCP, TLS) stay as they were.
//
// A node's dialers inherit (docs/v2/configuration-reference/inheritance.md):
// omitted takes the network's defaults, an explicit `{}` dials nothing, a
// non-empty set replaces the defaults whole.

export const DIALER_PROTOCOLS = [
  "native_udp",
  "native_tcp",
  "vless",
  "trojan",
] as const satisfies readonly TransportProtocol[]
export type DialerProtocol = (typeof DIALER_PROTOCOLS)[number]

/** The priority a protocol takes when its entry gives none. */
export const DEFAULT_PRIORITY: Record<DialerProtocol, number> = {
  native_udp: 100,
  native_tcp: 200,
  vless: 300,
  trojan: 400,
}

/** The connect timeout an entry takes when it gives none. */
export const DEFAULT_CONNECT_TIMEOUT = "5s"

type Entry = {
  enabled?: boolean
  priority?: number
  connectTimeout?: string
  stream?: DialerStream
}

const entryOf = (set: DialerSetUpdate, protocol: DialerProtocol) =>
  set[protocol] as Entry | undefined

export type DialerRow = {
  protocol: DialerProtocol
  enabled: boolean
  /** As written; null when the protocol's default applies. */
  priority: number | null
  /** The priority it dials at. */
  effectivePriority: number
  /** As written; null when the default applies. */
  connectTimeout: string | null
  /** VLESS only: as written, null for the default (auto). */
  stream: DialerStream | null
}

/** The protocols a set holds, in the order they are tried (priority, then key). */
export function dialerRows(set: DialerSetUpdate): DialerRow[] {
  return DIALER_PROTOCOLS.flatMap((protocol) => {
    const entry = entryOf(set, protocol)
    if (!entry) return []
    return [
      {
        protocol,
        enabled: entry.enabled ?? true,
        priority: entry.priority ?? null,
        effectivePriority: entry.priority ?? DEFAULT_PRIORITY[protocol],
        connectTimeout: entry.connectTimeout ?? null,
        stream: protocol === "vless" ? (entry.stream ?? null) : null,
      },
    ]
  }).sort(
    (a, b) =>
      a.effectivePriority - b.effectivePriority ||
      (a.protocol < b.protocol ? -1 : 1)
  )
}

/** The set with `protocol` in it (`{}`, the controller's defaults) or taken out. */
export function withProtocol(
  set: DialerSetUpdate,
  protocol: DialerProtocol,
  on: boolean
): DialerSetUpdate {
  const next = { ...set }
  if (!on) delete next[protocol]
  else if (!next[protocol]) next[protocol] = {}
  return next
}

type EntryField = "enabled" | "priority" | "connectTimeout" | "stream"

/**
 * The set with one field of `protocol`'s entry set; undefined leaves it to
 * the default. Other fields of the entry are kept as they were.
 */
export function withEntryField<F extends EntryField>(
  set: DialerSetUpdate,
  protocol: DialerProtocol,
  field: F,
  value: Entry[F] | undefined
): DialerSetUpdate {
  const entry = set[protocol]
  if (!entry) return set
  const nextEntry: Record<string, unknown> = { ...entry }
  if (value === undefined) delete nextEntry[field]
  else nextEntry[field] = value
  return { ...set, [protocol]: nextEntry }
}

/** A priority as typed: digits only; "" for the default. */
export function priorityOf(raw: string): number | undefined {
  const digits = raw.replace(/[^0-9]/g, "")
  return digits === "" ? undefined : Number(digits)
}

/** The protocols of a set whose fields the generated schema refuses. */
export function dialerProblems(set: DialerSetUpdate): DialerProtocol[] {
  const parsed = zDialerSetUpdate.safeParse(set)
  if (parsed.success) return []
  const found = new Set<DialerProtocol>()
  for (const issue of parsed.error.issues) {
    const key = issue.path[0]
    const protocol = DIALER_PROTOCOLS.find((known) => known === key)
    if (protocol) found.add(protocol)
  }
  return DIALER_PROTOCOLS.filter((protocol) => found.has(protocol))
}

/** How a node's dialers stand: inheriting the defaults, dialing nothing, or a set of its own. */
export type DialerMode = "inherit" | "none" | "own"

export function dialerMode(dialers: DialerSetUpdate | null): DialerMode {
  if (dialers === null) return "inherit"
  return Object.keys(dialers).length === 0 ? "none" : "own"
}

/**
 * A node's dialers after choosing `mode`: inheriting drops its own set,
 * none is `{}`, its own starts from the defaults (a copy to change).
 */
export function withDialerMode(
  current: DialerSetUpdate | null,
  mode: DialerMode,
  defaults: DialerSetUpdate
): DialerSetUpdate | null {
  if (mode === "inherit") return null
  if (mode === "none") return {}
  return current !== null && Object.keys(current).length > 0
    ? current
    : { ...defaults }
}
