import type {
  ConnectionTransport,
  ManagedListenerSettings,
  ManagedListenersSnapshot,
  ManagedListenerSummary,
  VlessStreamTransport,
} from "@meshway/control-client"
import {
  zConnectionEndpoint,
  zManagedListenerSettings,
  zManagedVlessSettings,
} from "@meshway/control-client/schemas"

// A platform-managed listener as it is edited: text as typed, turned into
// the generated `ManagedListenerSettings` when saved. The generated schema
// decides what it can; the few rules it cannot express (an IP literal to
// bind, a WebSocket path) are checked here too, and the controller still
// decides. Suggested values follow docs/managed-transports-api.md; nothing
// guesses a public address.

export const PRESETS = [
  "native_udp",
  "native_tcp",
  "vless_tcp",
  "vless_websocket",
] as const
export type Preset = (typeof PRESETS)[number]

const DEFAULTS = {
  bindAddress: "0.0.0.0",
  nativePort: "8421",
  vlessPort: "443",
  serverName: "meshway.local",
  websocketPath: "/meshway",
  websocketHost: "meshway.local",
} as const

export const MAX_ENDPOINTS = 16

export type EndpointFields = { host: string; port: string }

export type ListenerFields = {
  id: string
  preset: Preset
  enabled: boolean
  bindAddress: string
  bindPort: string
  endpoints: EndpointFields[]
  serverName: string
  websocketPath: string
  websocketHost: string
}

export const transportOf = (preset: Preset): ConnectionTransport =>
  preset === "native_udp" || preset === "native_tcp" ? preset : "vless"

const streamOf = (preset: Preset): VlessStreamTransport | null =>
  preset === "vless_tcp"
    ? "tcp"
    : preset === "vless_websocket"
      ? "websocket"
      : null

export function presetOf(settings: ManagedListenerSettings): Preset {
  if (settings.transport !== "vless") return settings.transport
  return settings.vless?.stream === "websocket"
    ? "vless_websocket"
    : "vless_tcp"
}

/** What a saved listener may become: its transport is fixed, a VLESS stream is not. */
export const presetsFor = (saved: ManagedListenerSettings | null): Preset[] =>
  saved === null
    ? [...PRESETS]
    : PRESETS.filter((preset) => transportOf(preset) === saved.transport)

const STEMS: Record<Preset, string> = {
  native_udp: "native-udp",
  native_tcp: "native-tcp",
  vless_tcp: "vless",
  vless_websocket: "vless-ws",
}

/** The first free ID from the preset's suggested stem (IDs are unique in the network). */
export function suggestId(preset: Preset, taken: ReadonlySet<string>): string {
  const stem = STEMS[preset]
  if (!taken.has(stem)) return stem
  for (let n = 2; ; n += 1) {
    const id = `${stem}-${n}`
    if (!taken.has(id)) return id
  }
}

export function newListener(
  preset: Preset,
  taken: ReadonlySet<string>
): ListenerFields {
  const vless = transportOf(preset) === "vless"
  const ws = preset === "vless_websocket"
  return {
    id: suggestId(preset, taken),
    preset,
    enabled: true,
    bindAddress: DEFAULTS.bindAddress,
    bindPort: vless ? DEFAULTS.vlessPort : DEFAULTS.nativePort,
    endpoints: [],
    serverName: vless ? DEFAULTS.serverName : "",
    websocketPath: ws ? DEFAULTS.websocketPath : "",
    websocketHost: ws ? DEFAULTS.websocketHost : "",
  }
}

export function fieldsOf(settings: ManagedListenerSettings): ListenerFields {
  return {
    id: settings.id,
    preset: presetOf(settings),
    enabled: settings.enabled,
    bindAddress: settings.bind_address,
    bindPort: String(settings.bind_port),
    endpoints: settings.advertised_endpoints.map((endpoint) => ({
      host: endpoint.host,
      port: String(endpoint.port),
    })),
    serverName: settings.vless?.server_name ?? "",
    websocketPath: settings.vless?.websocket_path ?? "",
    websocketHost: settings.vless?.websocket_host ?? "",
  }
}

/**
 * A preset change carries over what was typed; a value still at the old
 * preset's suggestion follows the new one. While creating, the ID follows
 * too as long as it is the suggested one.
 */
export function withPreset(
  fields: ListenerFields,
  preset: Preset,
  creating: { taken: ReadonlySet<string> } | null
): ListenerFields {
  if (preset === fields.preset) return fields
  let next = { ...fields, preset }
  if (creating && fields.id === suggestId(fields.preset, creating.taken))
    next.id = suggestId(preset, creating.taken)
  const wasVless = transportOf(fields.preset) === "vless"
  const isVless = transportOf(preset) === "vless"
  if (wasVless !== isVless) {
    const previous = wasVless ? DEFAULTS.vlessPort : DEFAULTS.nativePort
    if (fields.bindPort.trim() === previous || fields.bindPort.trim() === "")
      next = withPort(next, isVless ? DEFAULTS.vlessPort : DEFAULTS.nativePort)
  }
  if (isVless && next.serverName.trim() === "")
    next.serverName = DEFAULTS.serverName
  if (preset === "vless_websocket") {
    if (next.websocketPath.trim() === "")
      next.websocketPath = DEFAULTS.websocketPath
    if (next.websocketHost.trim() === "")
      next.websocketHost = websocketHostFor(publicHostOf(next))
  }
  return next
}

// The basics ----------------------------------------------------------------
//
// A listener is first its transport, its public address (the first
// advertised endpoint's host) and its port (the bind port). The first
// endpoint's port is linked to the bind port while the two are equal, so a
// port typed once is both what the node opens and what dialers use; a
// different public port (a NAT mapping) unlinks them.

export const publicHostOf = (fields: ListenerFields) =>
  fields.endpoints[0]?.host ?? ""

const websocketHostFor = (publicHost: string) =>
  publicHost.trim() === "" ? DEFAULTS.websocketHost : publicHost.trim()

/** Whether the first endpoint's port still follows the bind port. */
const portLinked = (fields: ListenerFields) =>
  fields.endpoints[0]?.port.trim() === fields.bindPort.trim()

/** The bind port; the first endpoint's port follows while it is linked. */
export function withPort(fields: ListenerFields, port: string): ListenerFields {
  const [first, ...rest] = fields.endpoints
  return {
    ...fields,
    bindPort: port,
    endpoints:
      first && portLinked(fields)
        ? [{ ...first, port }, ...rest]
        : fields.endpoints,
  }
}

/**
 * The public address. Typing one into a listener without endpoints adds
 * the first, at the bind port; clearing it removes that endpoint only when
 * it is the only one and still linked, otherwise the host is left empty
 * (and invalid), so nothing typed is dropped. The WebSocket Host follows
 * while it is still the old address or the platform default.
 */
export function withPublicHost(
  fields: ListenerFields,
  host: string
): ListenerFields {
  const [first, ...rest] = fields.endpoints
  let endpoints = fields.endpoints
  if (!first) {
    if (host.trim() !== "") endpoints = [{ host, port: fields.bindPort }]
  } else if (host.trim() === "" && rest.length === 0 && portLinked(fields))
    endpoints = []
  else endpoints = [{ ...first, host }, ...rest]
  const old = publicHostOf(fields).trim()
  const wsHost = fields.websocketHost.trim()
  const follows =
    wsHost !== "" && (wsHost === old || wsHost === DEFAULTS.websocketHost)
  return {
    ...fields,
    endpoints,
    websocketHost: follows ? websocketHostFor(host) : fields.websocketHost,
  }
}

/** The first endpoint's own port; a different value from the bind port unlinks them. */
export function withPublicPort(
  fields: ListenerFields,
  port: string
): ListenerFields {
  const [first, ...rest] = fields.endpoints
  return first
    ? { ...fields, endpoints: [{ ...first, port }, ...rest] }
    : fields
}

/** A node's first listener, from the basics alone: everything else stays at the platform's defaults. */
export const initialListener = (
  fields: ListenerFields,
  taken: ReadonlySet<string>
): ListenerFields => ({
  ...fields,
  id: suggestId(fields.preset, taken),
  enabled: true,
})

// The advanced part ---------------------------------------------------------

/** What differs from the platform defaults among the advanced fields, in form order. */
export type AdvancedFact =
  | { kind: "bind_address"; value: string }
  | { kind: "public_port"; value: string }
  | { kind: "endpoints"; count: number }
  | { kind: "server_name"; value: string }
  | { kind: "websocket_path"; value: string }
  | { kind: "websocket_host"; value: string }

export function advancedFacts(fields: ListenerFields): AdvancedFact[] {
  const facts: AdvancedFact[] = []
  const bind = fields.bindAddress.trim()
  if (bind !== DEFAULTS.bindAddress)
    facts.push({ kind: "bind_address", value: bind })
  const first = fields.endpoints[0]
  if (first && !portLinked(fields))
    facts.push({ kind: "public_port", value: first.port.trim() })
  if (fields.endpoints.length > 1)
    facts.push({ kind: "endpoints", count: fields.endpoints.length })
  if (transportOf(fields.preset) === "vless") {
    const name = fields.serverName.trim()
    if (name !== DEFAULTS.serverName)
      facts.push({ kind: "server_name", value: name })
  }
  if (fields.preset === "vless_websocket") {
    if (fields.websocketPath !== DEFAULTS.websocketPath)
      facts.push({ kind: "websocket_path", value: fields.websocketPath })
    const host = fields.websocketHost.trim()
    if (host !== websocketHostFor(publicHostOf(fields)))
      facts.push({ kind: "websocket_host", value: host })
  }
  return facts
}

/**
 * The problems that sit in the advanced part. A linked public port is the
 * port field's own problem, so it does not count there.
 */
export function advancedProblems(
  fields: ListenerFields,
  problems: readonly ListenerProblem[]
): ListenerProblem[] {
  const linked = portLinked(fields)
  return problems.filter(
    (problem) =>
      problem.field !== "bind_port" &&
      problem.field !== "endpoint.0.host" &&
      !(problem.field === "endpoint.0.port" && linked)
  )
}

// Validation ---------------------------------------------------------------

export type ListenerProblem =
  | { field: "id"; kind: "id" | "id_taken" }
  | { field: "bind_address"; kind: "bind_address" }
  | { field: "bind_port"; kind: "port" }
  | { field: "endpoints"; kind: "endpoints_max" }
  | {
      field: `endpoint.${number}.host`
      kind: "endpoint_host" | "endpoint_duplicate"
      index: number
    }
  | { field: `endpoint.${number}.port`; kind: "port"; index: number }
  | { field: "server_name"; kind: "server_name" }
  | { field: "websocket_path"; kind: "websocket_path" }
  | { field: "websocket_host"; kind: "websocket_host" }

const IPV4 =
  /^(25[0-5]|2[0-4][0-9]|1[0-9]{2}|[1-9]?[0-9])(\.(25[0-5]|2[0-4][0-9]|1[0-9]{2}|[1-9]?[0-9])){3}$/
const DNS_LABEL = /^[A-Za-z0-9]([A-Za-z0-9-]{0,61}[A-Za-z0-9])?$/
const PATH = /^\/[A-Za-z0-9\-._~!$&'()*+,;=:@%/]*$/
const WS_HOST = /^(\[[0-9A-Fa-f:.]+\]|[A-Za-z0-9.-]+)(:[0-9]{1,5})?$/
const PORT = /^[0-9]{1,5}$/

function isIpv6(value: string): boolean {
  if (!value.includes(":") || !/^[0-9A-Fa-f:.]+$/.test(value)) return false
  try {
    return new URL(`http://[${value}]/`).hostname !== ""
  } catch {
    return false
  }
}

const isIpLiteral = (value: string) => IPV4.test(value) || isIpv6(value)

function portOf(raw: string): number | null {
  const text = raw.trim()
  if (!PORT.test(text)) return null
  const port = Number(text)
  return zManagedListenerSettings.shape.bind_port.safeParse(port).success
    ? port
    : null
}

function bindAddressValid(raw: string): boolean {
  const value = raw.trim()
  if (IPV4.test(value)) {
    const first = Number(value.split(".")[0])
    return !(first >= 224 && first <= 239) && value !== "255.255.255.255"
  }
  return isIpv6(value) && !value.toLowerCase().startsWith("ff")
}

function hostValid(raw: string): boolean {
  const host = raw.trim().replace(/\.$/, "")
  if (!zConnectionEndpoint.shape.host.safeParse(host).success) return false
  if (isIpLiteral(host)) return true
  return host.split(".").every((label) => DNS_LABEL.test(label))
}

function serverNameValid(raw: string): boolean {
  const name = raw.trim()
  return (
    zManagedVlessSettings.shape.server_name.safeParse(name).success &&
    name.split(".").every((label) => DNS_LABEL.test(label))
  )
}

function websocketPathValid(path: string): boolean {
  return (
    zManagedVlessSettings.shape.websocket_path.safeParse(path).success &&
    PATH.test(path)
  )
}

function websocketHostValid(raw: string): boolean {
  const host = raw.trim()
  if (!zManagedVlessSettings.shape.websocket_host.safeParse(host).success)
    return false
  const match = WS_HOST.exec(host)
  if (!match) return false
  const port = match[2]
  return port === undefined || portOf(port.slice(1)) !== null
}

const endpointKey = (endpoint: EndpointFields) =>
  `${endpoint.host.trim().replace(/\.$/, "").toLowerCase()}:${endpoint.port.trim()}`

/** What stops saving, in form order. `taken` holds the network's other listener IDs when creating. */
export function listenerProblems(
  fields: ListenerFields,
  creating: { taken: ReadonlySet<string> } | null
): ListenerProblem[] {
  const problems: ListenerProblem[] = []
  if (creating) {
    const id = fields.id.trim()
    if (
      !zManagedListenerSettings.shape.id.safeParse(id).success ||
      id.startsWith("managed.")
    )
      problems.push({ field: "id", kind: "id" })
    else if (creating.taken.has(id))
      problems.push({ field: "id", kind: "id_taken" })
  }
  if (!bindAddressValid(fields.bindAddress))
    problems.push({ field: "bind_address", kind: "bind_address" })
  if (portOf(fields.bindPort) === null)
    problems.push({ field: "bind_port", kind: "port" })
  if (fields.endpoints.length > MAX_ENDPOINTS)
    problems.push({ field: "endpoints", kind: "endpoints_max" })
  const seen = new Set<string>()
  fields.endpoints.forEach((endpoint, index) => {
    const host = hostValid(endpoint.host)
    const port = portOf(endpoint.port) !== null
    if (!host)
      problems.push({
        field: `endpoint.${index}.host`,
        kind: "endpoint_host",
        index,
      })
    if (!port)
      problems.push({ field: `endpoint.${index}.port`, kind: "port", index })
    if (host && port) {
      const key = endpointKey(endpoint)
      if (seen.has(key))
        problems.push({
          field: `endpoint.${index}.host`,
          kind: "endpoint_duplicate",
          index,
        })
      seen.add(key)
    }
  })
  if (
    transportOf(fields.preset) === "vless" &&
    !serverNameValid(fields.serverName)
  )
    problems.push({ field: "server_name", kind: "server_name" })
  if (fields.preset === "vless_websocket") {
    if (!websocketPathValid(fields.websocketPath))
      problems.push({ field: "websocket_path", kind: "websocket_path" })
    if (!websocketHostValid(fields.websocketHost))
      problems.push({ field: "websocket_host", kind: "websocket_host" })
  }
  return problems
}

/** The generated DTO; fields that do not apply to the preset are sent as null, never as hidden values. */
export function settingsOf(
  fields: ListenerFields,
  nodeId: string
): ManagedListenerSettings {
  const stream = streamOf(fields.preset)
  return {
    id: fields.id.trim(),
    node_id: nodeId,
    enabled: fields.enabled,
    transport: transportOf(fields.preset),
    bind_address: fields.bindAddress.trim(),
    bind_port: Number(fields.bindPort.trim()),
    advertised_endpoints: fields.endpoints.map((endpoint) => ({
      host: endpoint.host.trim(),
      port: Number(endpoint.port.trim()),
    })),
    vless:
      stream === null
        ? null
        : {
            stream,
            server_name: fields.serverName.trim(),
            websocket_path:
              stream === "websocket" ? fields.websocketPath : null,
            websocket_host:
              stream === "websocket" ? fields.websocketHost.trim() : null,
          },
  }
}

// Reads ----------------------------------------------------------------------

export const listenersOf = (
  snapshot: ManagedListenersSnapshot | undefined,
  nodeId: string
): ManagedListenerSummary[] =>
  (snapshot?.listeners ?? [])
    .filter((listener) => listener.settings.node_id === nodeId)
    .sort((a, b) => a.settings.id.localeCompare(b.settings.id))

/** `host:port`, with IPv6 in brackets. */
export const endpointText = (host: string, port: number | string) =>
  host.includes(":") ? `[${host}]:${port}` : `${host}:${port}`

export const certificateText = (sha256: string) =>
  sha256.length > 16 ? `${sha256.slice(0, 8)}…${sha256.slice(-8)}` : sha256
