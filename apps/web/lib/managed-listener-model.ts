import type {
  ConnectionCandidate,
  ConnectionEndpoint,
  ConnectionsSnapshot,
  ConnectionTransport,
  ManagedListenerSettings,
  ManagedListenerSummary,
  ManagedListenersSnapshot,
  VlessStreamTransport,
} from "@meshway/control-client"

import { endpointLabel, hostProblem, stableId } from "./connections-model.ts"

// Form model for platform-managed listeners. Text inputs keep their raw
// strings; `toSettings` produces the exact generated DTO. Local checks mirror
// the controller's validation (apps/controller/src/store/managed_transports/mod.rs)
// because a rejected save carries only its HTTP status; the controller stays
// the authority and normalizes bind, endpoints and the TLS name.

export type ManagedPreset = "native_udp" | "native_tcp" | "vless_tcp" | "vless_websocket"

export const managedPresets: readonly ManagedPreset[] = ["native_udp", "native_tcp", "vless_tcp", "vless_websocket"]

export const presetLabels: Record<ManagedPreset, string> = {
  native_udp: "Native UDP",
  native_tcp: "Native TCP",
  vless_tcp: "VLESS · TLS",
  vless_websocket: "VLESS · WebSocket",
}

export const presetShortLabels: Record<ManagedPreset, string> = {
  native_udp: "UDP",
  native_tcp: "TCP",
  vless_tcp: "VLESS TLS",
  vless_websocket: "VLESS WS",
}

const presetIdStems: Record<ManagedPreset, string> = {
  native_udp: "native-udp",
  native_tcp: "native-tcp",
  vless_tcp: "vless",
  vless_websocket: "vless-ws",
}

/** Suggested values from docs/managed-transports-api.md; never a guessed public address. */
export const MANAGED_DEFAULTS = {
  bindAddress: "0.0.0.0",
  nativePort: "8421",
  vlessPort: "443",
  serverName: "meshway.local",
  websocketPath: "/meshway",
  websocketHost: "meshway.local",
} as const

export const MANAGED_LIMITS = {
  idLength: 48,
  endpoints: 16,
  pathLength: 1024,
  hostLength: 253,
} as const

export type EndpointField = { key: string; host: string; port: string }

export type ManagedListenerForm = {
  id: string
  preset: ManagedPreset
  enabled: boolean
  bindAddress: string
  bindPort: string
  endpoints: EndpointField[]
  serverName: string
  websocketPath: string
  websocketHost: string
}

export function transportOf(preset: ManagedPreset): ConnectionTransport {
  return preset === "native_udp" ? "native_udp" : preset === "native_tcp" ? "native_tcp" : "vless"
}

export function streamOf(preset: ManagedPreset): VlessStreamTransport | null {
  return preset === "vless_tcp" ? "tcp" : preset === "vless_websocket" ? "websocket" : null
}

export function presetOf(settings: ManagedListenerSettings): ManagedPreset {
  if (settings.transport === "native_udp") return "native_udp"
  if (settings.transport === "native_tcp") return "native_tcp"
  return settings.vless?.stream === "websocket" ? "vless_websocket" : "vless_tcp"
}

/** Presets an existing listener may move to: the transport is fixed, the VLESS stream is not. */
export function allowedPresets(original: ManagedListenerSettings | null): ManagedPreset[] {
  if (original === null) return [...managedPresets]
  return managedPresets.filter((preset) => transportOf(preset) === original.transport)
}

export function suggestManagedId(preset: ManagedPreset, taken: Iterable<string>, prefix = ""): string {
  const stem = prefix === "" ? presetIdStems[preset] : `${prefix}-${presetIdStems[preset]}`
  return stableId(stem.slice(0, MANAGED_LIMITS.idLength - 4), taken)
}

export function newManagedForm(preset: ManagedPreset, taken: Iterable<string>, prefix = ""): ManagedListenerForm {
  const vless = transportOf(preset) === "vless"
  return {
    id: suggestManagedId(preset, taken, prefix),
    preset,
    enabled: true,
    bindAddress: MANAGED_DEFAULTS.bindAddress,
    bindPort: vless ? MANAGED_DEFAULTS.vlessPort : MANAGED_DEFAULTS.nativePort,
    endpoints: [],
    serverName: vless ? MANAGED_DEFAULTS.serverName : "",
    websocketPath: preset === "vless_websocket" ? MANAGED_DEFAULTS.websocketPath : "",
    websocketHost: preset === "vless_websocket" ? MANAGED_DEFAULTS.websocketHost : "",
  }
}

export function fromSettings(settings: ManagedListenerSettings): ManagedListenerForm {
  return {
    id: settings.id,
    preset: presetOf(settings),
    enabled: settings.enabled,
    bindAddress: settings.bind_address,
    bindPort: String(settings.bind_port),
    endpoints: settings.advertised_endpoints.map((endpoint, index) => ({
      key: `e${index}`,
      host: endpoint.host,
      port: String(endpoint.port),
    })),
    serverName: settings.vless?.server_name ?? "",
    websocketPath: settings.vless?.websocket_path ?? "",
    websocketHost: settings.vless?.websocket_host ?? "",
  }
}

export function newEndpointFieldKey(form: ManagedListenerForm): string {
  const taken = new Set(form.endpoints.map((endpoint) => endpoint.key))
  let n = form.endpoints.length
  while (taken.has(`e${n}`)) n += 1
  return `e${n}`
}

function firstHostname(form: ManagedListenerForm): string | null {
  for (const endpoint of form.endpoints) {
    const host = endpoint.host.trim()
    if (host !== "" && hostProblem(host) === null && !isIpLiteral(host)) return host.replace(/\.$/, "").toLowerCase()
  }
  return null
}

function suggestedHost(form: ManagedListenerForm): string {
  return firstHostname(form) ?? MANAGED_DEFAULTS.websocketHost
}

/**
 * While creating, the WebSocket Host follows the first advertised hostname
 * as long as it still holds the previous suggestion; a typed Host is kept.
 */
export function followSuggestions(previous: ManagedListenerForm, next: ManagedListenerForm): ManagedListenerForm {
  if (next.preset !== "vless_websocket" || previous.preset !== "vless_websocket") return next
  if (next.websocketHost !== previous.websocketHost || previous.websocketHost.trim() !== suggestedHost(previous)) return next
  return { ...next, websocketHost: suggestedHost(next) }
}

/**
 * Applies a preset change, carrying over what the user typed. Suggested
 * values (port, ID, TLS / WebSocket defaults) follow the preset only while
 * they still hold the previous suggestion.
 */
export function changePreset(
  form: ManagedListenerForm,
  preset: ManagedPreset,
  options: { idFollows: boolean; taken: Iterable<string>; prefix?: string }
): ManagedListenerForm {
  if (preset === form.preset) return form
  const wasVless = transportOf(form.preset) === "vless"
  const isVless = transportOf(preset) === "vless"
  const next: ManagedListenerForm = { ...form, preset }
  if (options.idFollows) next.id = suggestManagedId(preset, options.taken, options.prefix)
  if (wasVless !== isVless) {
    const previous = wasVless ? MANAGED_DEFAULTS.vlessPort : MANAGED_DEFAULTS.nativePort
    if (form.bindPort.trim() === previous || form.bindPort.trim() === "") {
      next.bindPort = isVless ? MANAGED_DEFAULTS.vlessPort : MANAGED_DEFAULTS.nativePort
    }
  }
  if (isVless && next.serverName.trim() === "") next.serverName = MANAGED_DEFAULTS.serverName
  if (preset === "vless_websocket") {
    if (next.websocketPath.trim() === "") next.websocketPath = MANAGED_DEFAULTS.websocketPath
    if (next.websocketHost.trim() === "") next.websocketHost = firstHostname(form) ?? MANAGED_DEFAULTS.websocketHost
  }
  return next
}

// ---------------------------------------------------------------------------
// Validation

export type ManagedField =
  | "id"
  | "bind_address"
  | "bind_port"
  | "advertised_endpoints"
  | `advertised_endpoints[${number}].host`
  | `advertised_endpoints[${number}].port`
  | "server_name"
  | "websocket_path"
  | "websocket_host"

export type ManagedIssue = { field: ManagedField; message: string }

const IPV4 = /^(25[0-5]|2[0-4][0-9]|1[0-9]{2}|[1-9]?[0-9])(\.(25[0-5]|2[0-4][0-9]|1[0-9]{2}|[1-9]?[0-9])){3}$/
const ID = /^[A-Za-z0-9._-]+$/
const DNS_LABEL = /^[A-Za-z0-9]([A-Za-z0-9-]{0,61}[A-Za-z0-9])?$/
// RFC 3986 path characters; stricter than the controller only for characters
// no client would put into a WebSocket upgrade path.
const WS_PATH = /^\/[A-Za-z0-9\-._~!$&'()*+,;=:@%/]*$/
const WS_HOST = /^(\[[0-9A-Fa-f:.]+\]|[A-Za-z0-9.-]+)(:[0-9]{1,5})?$/

function validIpv6(value: string): boolean {
  if (!value.includes(":") || !/^[0-9A-Fa-f:.]+$/.test(value)) return false
  try {
    return new URL(`http://[${value}]/`).hostname !== ""
  } catch {
    return false
  }
}

export function isIpLiteral(value: string): boolean {
  return IPV4.test(value) || validIpv6(value)
}

/** Bind address problem: an IP literal; unspecified is allowed, multicast / broadcast are not. */
export function bindAddressProblem(raw: string): string | null {
  const value = raw.trim()
  if (value === "") return "监听地址不能为空"
  if (value.startsWith("[")) return "IPv6 地址不需要方括号"
  if (IPV4.test(value)) {
    const [a = 0] = value.split(".").map(Number)
    if (a >= 224 && a <= 239) return "不能监听组播地址"
    if (value === "255.255.255.255") return "不能监听广播地址"
    return null
  }
  if (validIpv6(value)) return value.toLowerCase().startsWith("ff") ? "不能监听组播地址" : null
  return "必须是本机 IP 地址（如 0.0.0.0、:: 或网卡地址），不支持主机名"
}

function portValue(raw: string): number | null {
  const text = raw.trim()
  if (!/^[0-9]{1,5}$/.test(text)) return null
  const port = Number(text)
  return port >= 1 && port <= 65535 ? port : null
}

export function serverNameProblem(raw: string): string | null {
  const name = raw.trim()
  if (name === "") return "TLS 名称不能为空"
  if (name.length > MANAGED_LIMITS.hostLength) return "最长 253 个字符"
  if (!name.split(".").every((label) => DNS_LABEL.test(label))) {
    return "只能是 DNS 名称：字母、数字、连字符，以点分隔"
  }
  return null
}

export function websocketPathProblem(raw: string): string | null {
  const path = raw
  if (path.trim() === "") return "WebSocket 路径不能为空"
  if (!path.startsWith("/")) return "路径必须以 / 开头"
  if (path.length > MANAGED_LIMITS.pathLength) return "最长 1024 个字符"
  if (/[?#]/.test(path)) return "路径不能包含 ? 或 #"
  if (/\s/.test(path)) return "路径不能包含空格或换行"
  if (!WS_PATH.test(path)) return "路径只能包含 URL 路径字符（非 ASCII 请先百分号编码）"
  return null
}

export function websocketHostProblem(raw: string): string | null {
  const host = raw.trim()
  if (host === "") return "Host 不能为空"
  if (host.length > MANAGED_LIMITS.hostLength) return "最长 253 个字符"
  if (/[/\\?#@]/.test(host) || /\s/.test(host)) return "Host 只写主机名，可带 :端口，不含路径"
  const match = WS_HOST.exec(host)
  if (!match) return "不是有效的 Host（主机名或 [IPv6]，可带 :端口）"
  const port = match[2]
  if (port !== undefined && portValue(port.slice(1)) === null) return "端口必须在 1–65535"
  return null
}

export function validateManagedForm(
  form: ManagedListenerForm,
  context: {
    /** True when creating: the ID is checked against `takenIds`. */
    creating: boolean
    /** Listener IDs already used in this network (managed and manual). */
    takenIds: Iterable<string>
  }
): ManagedIssue[] {
  const issues: ManagedIssue[] = []
  const id = form.id.trim()
  if (context.creating) {
    if (id === "") issues.push({ field: "id", message: "ID 不能为空" })
    else if (id.length > MANAGED_LIMITS.idLength) issues.push({ field: "id", message: "ID 最长 48 个字符" })
    else if (!ID.test(id)) issues.push({ field: "id", message: "只能使用字母、数字、点、下划线和连字符" })
    else if (id.startsWith("managed.")) issues.push({ field: "id", message: "不能以 managed. 开头（平台保留）" })
    else if (new Set(context.takenIds).has(id)) issues.push({ field: "id", message: "本网络已有同名监听；ID 在整张网络内唯一" })
  }
  const bind = bindAddressProblem(form.bindAddress)
  if (bind) issues.push({ field: "bind_address", message: bind })
  if (portValue(form.bindPort) === null) issues.push({ field: "bind_port", message: "端口必须在 1–65535" })

  if (form.endpoints.length > MANAGED_LIMITS.endpoints) {
    issues.push({ field: "advertised_endpoints", message: `最多 ${MANAGED_LIMITS.endpoints} 个对外地址` })
  }
  const seen = new Set<string>()
  form.endpoints.forEach((endpoint, index) => {
    const host = hostProblem(endpoint.host)
    if (host) issues.push({ field: `advertised_endpoints[${index}].host`, message: `对外地址 ${index + 1}：${host}` })
    const port = portValue(endpoint.port)
    if (port === null) issues.push({ field: `advertised_endpoints[${index}].port`, message: `对外地址 ${index + 1}：端口必须在 1–65535` })
    if (!host && port !== null) {
      const key = `${endpoint.host.trim().replace(/\.$/, "").toLowerCase()}:${port}`
      if (seen.has(key)) issues.push({ field: `advertised_endpoints[${index}].host`, message: `对外地址 ${index + 1} 与前面的地址重复` })
      seen.add(key)
    }
  })

  if (transportOf(form.preset) === "vless") {
    const name = serverNameProblem(form.serverName)
    if (name) issues.push({ field: "server_name", message: name })
  }
  if (form.preset === "vless_websocket") {
    const path = websocketPathProblem(form.websocketPath)
    if (path) issues.push({ field: "websocket_path", message: path })
    const host = websocketHostProblem(form.websocketHost)
    if (host) issues.push({ field: "websocket_host", message: host })
  }
  return issues
}

/**
 * The exact generated DTO. Fields that do not apply to the preset are sent
 * as null, never as stale hidden values.
 */
export function toSettings(form: ManagedListenerForm, nodeId: string): ManagedListenerSettings {
  const stream = streamOf(form.preset)
  return {
    id: form.id.trim(),
    node_id: nodeId,
    enabled: form.enabled,
    transport: transportOf(form.preset),
    bind_address: form.bindAddress.trim(),
    bind_port: Number(form.bindPort.trim()),
    advertised_endpoints: form.endpoints.map(
      (endpoint): ConnectionEndpoint => ({ host: endpoint.host.trim(), port: Number(endpoint.port.trim()) })
    ),
    vless:
      stream === null
        ? null
        : {
            stream,
            server_name: form.serverName.trim(),
            websocket_path: stream === "websocket" ? form.websocketPath : null,
            websocket_host: stream === "websocket" ? form.websocketHost.trim() : null,
          },
  }
}

/** Comparable key of what a form would save (whitespace-insensitive like the DTO). */
export function formKey(form: ManagedListenerForm): string {
  return JSON.stringify(toSettings(form, ""))
}

export function sameForm(a: ManagedListenerForm, b: ManagedListenerForm): boolean {
  return formKey(a) === formKey(b) && a.endpoints.length === b.endpoints.length
}

// ---------------------------------------------------------------------------
// Reads

export function managedFor(snapshot: ManagedListenersSnapshot, nodeId: string): ManagedListenerSummary[] {
  return snapshot.listeners.filter((listener) => listener.settings.node_id === nodeId)
}

export function managedById(snapshot: ManagedListenersSnapshot | null, id: string): ManagedListenerSummary | undefined {
  return snapshot?.listeners.find((listener) => listener.settings.id === id)
}

/** Every listener ID in the network: managed (enabled or not) and manual plan listeners. */
export function takenListenerIds(managed: ManagedListenersSnapshot, connections: ConnectionsSnapshot | null): string[] {
  const ids = new Set(managed.listeners.map((listener) => listener.settings.id))
  for (const listener of connections?.configuration.listeners ?? []) ids.add(listener.id)
  return [...ids]
}

/** Manual (non-managed) listeners of a node: legacy local-profile or hand-written plan entries. */
export function manualListenerIds(managed: ManagedListenersSnapshot | null, connections: ConnectionsSnapshot, nodeId: string): string[] {
  const managedIds = new Set(managed?.listeners.map((listener) => listener.settings.id) ?? [])
  return connections.configuration.listeners
    .filter((listener) => listener.node_id === nodeId && !managedIds.has(listener.id) && !listener.credential_ref?.startsWith("managed."))
    .map((listener) => listener.id)
}

/** A listener is managed when the managed read lists it, or (read failed) its VLESS reference says so. */
export function isManagedListener(managedIds: ReadonlySet<string> | null, id: string, credentialRef: string | null): boolean {
  return managedIds?.has(id) === true || credentialRef === `managed.${id}`
}

export function certificatePreview(sha256: string): string {
  return sha256.length > 16 ? `${sha256.slice(0, 8)}…${sha256.slice(-8)}` : sha256
}

// ---------------------------------------------------------------------------
// Change impact, shown before saving

export type ReferencingCandidate = { id: string; dialerNodeId: string; endpoint: string; endpointListed: boolean }

/** Saved dial candidates that target a listener; the controller keeps them through disable / edits. */
export function referencingCandidates(
  connections: ConnectionsSnapshot | null,
  listenerId: string,
  advertised: { host: string; port: string }[]
): ReferencingCandidate[] {
  if (!connections) return []
  const listed = new Set(
    advertised.map((endpoint) => `${endpoint.host.trim().replace(/\.$/, "").toLowerCase()}:${endpoint.port.trim()}`)
  )
  return connections.configuration.candidates
    .filter((candidate: ConnectionCandidate) => candidate.listener_id === listenerId)
    .map((candidate) => ({
      id: candidate.id,
      dialerNodeId: candidate.dialer_node_id,
      endpoint: endpointLabel(candidate.endpoint.host, candidate.endpoint.port),
      endpointListed: listed.has(`${candidate.endpoint.host.replace(/\.$/, "").toLowerCase()}:${candidate.endpoint.port}`),
    }))
}

export type ChangeImpact = {
  /** Disabling while candidates still reference it: the saved plan becomes invalid until repaired. */
  disablesReferenced: number
  /** Re-enabling restores referencing candidates. */
  reenables: boolean
  /** Candidates whose saved endpoint is no longer advertised: they stop matching. */
  orphanedCandidates: ReferencingCandidate[]
  /** TLS name change: the platform issues a new certificate and UUID; the old generation is retired. */
  rotatesMaterial: boolean
}

export function changeImpact(
  original: ManagedListenerSettings | null,
  form: ManagedListenerForm,
  candidates: ReferencingCandidate[]
): ChangeImpact {
  const enabledBefore = original?.enabled ?? false
  return {
    disablesReferenced: original !== null && enabledBefore && !form.enabled ? candidates.length : 0,
    reenables: original !== null && !enabledBefore && form.enabled && candidates.length > 0,
    orphanedCandidates: form.enabled ? candidates.filter((candidate) => !candidate.endpointListed) : [],
    rotatesMaterial:
      original !== null &&
      original.vless !== null &&
      transportOf(form.preset) === "vless" &&
      original.vless.server_name !== form.serverName.trim().toLowerCase(),
  }
}

/** Fields that differ between two saved versions (for a conflict notice). */
export function changedFields(a: ManagedListenerSettings, b: ManagedListenerSettings): string[] {
  const out: string[] = []
  if (a.enabled !== b.enabled) out.push("启用状态")
  if (a.bind_address !== b.bind_address || a.bind_port !== b.bind_port) out.push("本机监听")
  if (JSON.stringify(a.advertised_endpoints) !== JSON.stringify(b.advertised_endpoints)) out.push("对外地址")
  if (a.vless?.stream !== b.vless?.stream) out.push("VLESS 底层")
  if (a.vless?.server_name !== b.vless?.server_name) out.push("TLS 名称")
  if (a.vless?.websocket_path !== b.vless?.websocket_path || a.vless?.websocket_host !== b.vless?.websocket_host) out.push("WebSocket")
  return out
}

export function describeSettings(settings: ManagedListenerSettings): string {
  const bind = settings.bind_address.includes(":") ? `[${settings.bind_address}]:${settings.bind_port}` : `${settings.bind_address}:${settings.bind_port}`
  return `${presetLabels[presetOf(settings)]} · ${bind}`
}
