import type {
  ConnectionsSnapshot,
  ConnectionCandidate,
  ConnectionCapability,
  ConnectionEndpoint,
  ConnectionListener,
  ConnectionNode,
  ConnectionPlan,
  ConnectionTransport,
} from "@meshway/control-client"

// Editor-side model for the connection control intent. Drafts mirror the
// generated ConnectionPlan DTO field for field; text inputs keep their raw
// string so typing is not disrupted, and `toPlan` produces the exact DTO sent.
// Local checks mirror the controller's validation to give actionable hints
// (a rejected save carries no per-item diagnostics); the controller remains
// the authority and normalizes hosts and bind addresses.

export type Transport = ConnectionTransport

/** Controller caps (crates/protocol CONNECTIONS.md). The editor never claims more. */
export const CONNECTION_LIMITS = {
  listeners: 128,
  candidates: 256,
  endpointsPerListener: 16,
  planBytes: 128 * 1024,
  idLength: 64,
} as const

export const transports: readonly Transport[] = ["native_udp", "native_tcp", "vless"]

export const transportLabels: Record<Transport, string> = {
  native_udp: "Native UDP",
  native_tcp: "Native TCP",
  vless: "VLESS",
}

export const transportHints: Record<Transport, string> = {
  native_udp: "Meshway 原生 Noise 实验载体，数据报",
  native_tcp: "Meshway 原生载体，字节流",
  vless: "字节流；双方各自引用本地配置档",
}

export function needsProfile(transport: Transport): boolean {
  return transport === "vless"
}

export type EndpointDraft = { key: string; host: string; port: string }

export type ListenerDraft = {
  key: string
  id: string
  nodeId: string
  transport: Transport
  bind: string
  endpoints: EndpointDraft[]
  /** "" means null; required for VLESS, must stay empty for native carriers. */
  credentialRef: string
}

export type CandidateDraft = {
  key: string
  id: string
  dialerNodeId: string
  listenerId: string
  /** "" when not chosen yet. */
  host: string
  port: string
  priority: string
  credentialRef: string
}

export type ConnectionsDraft = {
  listeners: ListenerDraft[]
  candidates: CandidateDraft[]
}

export const emptyDraft: ConnectionsDraft = { listeners: [], candidates: [] }

/**
 * Candidate form state: the draft item plus the chosen target node, which is
 * UI-only (the acceptor always comes from the listener's owner) but must
 * survive navigation before an entry is picked.
 */
export type CandidateForm = CandidateDraft & { targetNodeId: string }

/**
 * Which side the opening context fixes: a per-node editor opens "this node
 * listens" (`node`), "this node dials" (`dialer`) or "another node dials this
 * node" (`target`). The global editor fixes nothing.
 */
export type SessionFixed = "node" | "dialer" | "target" | null

/** One opening of a listener / candidate sheet. */
export type ItemSession<T> = {
  /** Increments per opening so the form remounts with fresh state. */
  id: number
  mode: "new" | "edit"
  /** What the item was when the sheet opened; unapplied edits compare against it. */
  initial: T
  /** Unapplied form contents to resume with (restored after navigating away). */
  working?: T
  fixed?: SessionFixed
}

export type ListenerSession = ItemSession<ListenerDraft>
export type CandidateSession = ItemSession<CandidateForm>

/** An open sheet holding edits that are not applied to the draft yet. */
export type WorkingSheet =
  | { kind: "listener"; session: ListenerSession; form: ListenerDraft }
  | { kind: "candidate"; session: CandidateSession; form: CandidateForm }

export function toCandidateForm(candidate: CandidateDraft, listeners: ListenerDraft[]): CandidateForm {
  const listener = listeners.find((existing) => existing.id.trim() === candidate.listenerId)
  return { ...candidate, targetNodeId: listener?.nodeId ?? "" }
}

export function fromCandidateForm(form: CandidateForm): CandidateDraft {
  return {
    key: form.key,
    id: form.id,
    dialerNodeId: form.dialerNodeId,
    listenerId: form.listenerId,
    host: form.host,
    port: form.port,
    priority: form.priority,
    credentialRef: form.credentialRef,
  }
}

/** `${stem}<n>` with the smallest n not already taken. */
export function freshKey(stem: string, taken: Iterable<string>): string {
  const used = new Set(taken)
  let n = 0
  while (used.has(`${stem}${n}`)) n += 1
  return `${stem}${n}`
}

/** A new advertised-endpoint row key, unique among the listener's current rows. */
export function newEndpointKey(listener: ListenerDraft): string {
  return freshKey(`${listener.key}-e`, listener.endpoints.map((endpoint) => endpoint.key))
}

export function sameItem<T>(a: T, b: T): boolean {
  return JSON.stringify(a) === JSON.stringify(b)
}

export function fromPlan(plan: ConnectionPlan): ConnectionsDraft {
  return {
    listeners: plan.listeners.map((listener, index) => ({
      key: `saved-l${index}`,
      id: listener.id,
      nodeId: listener.node_id,
      transport: listener.transport,
      bind: listener.bind,
      endpoints: listener.advertised_endpoints.map((endpoint, e) => ({
        key: `saved-l${index}-e${e}`,
        host: endpoint.host,
        port: String(endpoint.port),
      })),
      credentialRef: listener.credential_ref ?? "",
    })),
    candidates: plan.candidates.map((candidate, index) => ({
      key: `saved-c${index}`,
      id: candidate.id,
      dialerNodeId: candidate.dialer_node_id,
      listenerId: candidate.listener_id,
      host: candidate.endpoint.host,
      port: String(candidate.endpoint.port),
      priority: String(candidate.priority),
      credentialRef: candidate.credential_ref ?? "",
    })),
  }
}

const DIGITS = /^[0-9]{1,5}$/

function parsePort(value: string): number {
  const text = value.trim()
  return DIGITS.test(text) ? Number(text) : Number.NaN
}

function nullable(value: string): string | null {
  const text = value.trim()
  return text === "" ? null : text
}

export function toEndpoint(host: string, port: string): ConnectionEndpoint {
  return { host: host.trim(), port: parsePort(port) }
}

export function toListener(listener: ListenerDraft): ConnectionListener {
  return {
    id: listener.id.trim(),
    node_id: listener.nodeId,
    transport: listener.transport,
    bind: listener.bind.trim(),
    advertised_endpoints: listener.endpoints.map((endpoint) =>
      toEndpoint(endpoint.host, endpoint.port)
    ),
    credential_ref: nullable(listener.credentialRef),
  }
}

export function toCandidate(candidate: CandidateDraft): ConnectionCandidate {
  return {
    id: candidate.id.trim(),
    dialer_node_id: candidate.dialerNodeId,
    listener_id: candidate.listenerId,
    endpoint: toEndpoint(candidate.host, candidate.port),
    priority: parsePort(candidate.priority),
    credential_ref: nullable(candidate.credentialRef),
  }
}

/** Only meaningful once `validateConnections` reports no issues. */
export function toPlan(draft: ConnectionsDraft): ConnectionPlan {
  return {
    listeners: draft.listeners.map(toListener),
    candidates: draft.candidates.map(toCandidate),
  }
}

export function planBytes(plan: ConnectionPlan): number {
  return new TextEncoder().encode(JSON.stringify(plan)).length
}

// Structural identity over raw draft text. The controller stores collections
// sorted by ID and endpoints sorted by host/port, so neither order counts as a
// change. Built with one fixed key order, independent of DTO key order.
function listenerKey(listener: ListenerDraft): string {
  return JSON.stringify([
    listener.id.trim(),
    listener.nodeId,
    listener.transport,
    listener.bind.trim(),
    listener.endpoints.map((e) => [e.host.trim(), e.port.trim()]).sort(compareTuple),
    listener.credentialRef.trim(),
  ])
}

function candidateKey(candidate: CandidateDraft): string {
  return JSON.stringify([
    candidate.id.trim(),
    candidate.dialerNodeId,
    candidate.listenerId,
    candidate.host.trim(),
    candidate.port.trim(),
    candidate.priority.trim(),
    candidate.credentialRef.trim(),
  ])
}

function compareTuple(a: string[], b: string[]): number {
  return JSON.stringify(a) < JSON.stringify(b) ? -1 : JSON.stringify(a) > JSON.stringify(b) ? 1 : 0
}

export function draftKey(draft: ConnectionsDraft): string {
  return JSON.stringify([
    draft.listeners.map(listenerKey).sort(),
    draft.candidates.map(candidateKey).sort(),
  ])
}

export function planKey(plan: ConnectionPlan): string {
  return draftKey(fromPlan(plan))
}

export type ItemChange = "new" | "modified" | "unchanged"

export function listenerChange(listener: ListenerDraft, base: ConnectionPlan): ItemChange {
  const saved = base.listeners.find((candidate) => candidate.id === listener.id.trim())
  if (!saved) return "new"
  const [savedDraft] = fromPlan({ listeners: [saved], candidates: [] }).listeners
  return savedDraft && listenerKey(savedDraft) === listenerKey(listener) ? "unchanged" : "modified"
}

export function candidateChange(candidate: CandidateDraft, base: ConnectionPlan): ItemChange {
  const saved = base.candidates.find((existing) => existing.id === candidate.id.trim())
  if (!saved) return "new"
  const [savedDraft] = fromPlan({ listeners: [], candidates: [saved] }).candidates
  return savedDraft && candidateKey(savedDraft) === candidateKey(candidate) ? "unchanged" : "modified"
}

export function removedCount(draft: ConnectionsDraft, base: ConnectionPlan) {
  const listenerIds = new Set(draft.listeners.map((listener) => listener.id.trim()))
  const candidateIds = new Set(draft.candidates.map((candidate) => candidate.id.trim()))
  return {
    listeners: base.listeners.filter((listener) => !listenerIds.has(listener.id)).length,
    candidates: base.candidates.filter((candidate) => !candidateIds.has(candidate.id)).length,
  }
}

// ---------------------------------------------------------------------------
// Node identity and capability readiness.

export type NodeRef =
  | { kind: "active"; entry: ConnectionNode }
  | { kind: "revoked"; entry: ConnectionNode }
  | { kind: "unknown"; id: string }

export function resolveConnectionNode(nodes: ConnectionNode[], id: string): NodeRef {
  const entry = nodes.find((candidate) => candidate.node.id === id)
  if (!entry) return { kind: "unknown", id }
  return entry.node.revoked_at === null ? { kind: "active", entry } : { kind: "revoked", entry }
}

export function nodeName(nodes: ConnectionNode[], id: string): string {
  const ref = resolveConnectionNode(nodes, id)
  if (ref.kind === "unknown") return id === "" ? "未选择" : `未知节点 ${id}`
  return ref.kind === "revoked" ? `${ref.entry.node.name}（已撤销）` : ref.entry.node.name
}

export type Role = "accept" | "dial"

export function capabilityFor(
  entry: ConnectionNode,
  transport: Transport
): ConnectionCapability | undefined {
  return entry.capabilities.find((capability) => capability.transport === transport)
}

/** Same rule as the controller: active, published key, declared role. */
export function canServe(entry: ConnectionNode, transport: Transport, role: Role): boolean {
  if (entry.node.revoked_at !== null || entry.public_key === null) return false
  const capability = capabilityFor(entry, transport)
  if (!capability) return false
  return role === "accept" ? capability.can_accept : capability.can_dial
}

export type Readiness = "revoked" | "no_key" | "no_capabilities" | "ready"

export function readiness(entry: ConnectionNode): Readiness {
  if (entry.node.revoked_at !== null) return "revoked"
  if (entry.public_key === null) return "no_key"
  if (entry.capabilities.length === 0) return "no_capabilities"
  return "ready"
}

export const readinessLabels: Record<Readiness, string> = {
  revoked: "已撤销",
  no_key: "未发布公钥",
  no_capabilities: "当前构建未声明承载能力",
  ready: "可配置",
}

/** Why this node cannot take the role, or null when it can. */
export function roleBlocker(entry: ConnectionNode, transport: Transport, role: Role): string | null {
  const state = readiness(entry)
  if (state !== "ready") return readinessLabels[state]
  const capability = capabilityFor(entry, transport)
  if (!capability) return `未声明 ${transportLabels[transport]}`
  if (role === "accept" && !capability.can_accept) return `${transportLabels[transport]} 不可监听`
  if (role === "dial" && !capability.can_dial) return `${transportLabels[transport]} 不可发起`
  return null
}

/** Why this node cannot take the role on any carrier, or null when it can on at least one. */
export function anyRoleBlocker(entry: ConnectionNode, role: Role): string | null {
  const state = readiness(entry)
  if (state !== "ready") return readinessLabels[state]
  if (transports.some((transport) => canServe(entry, transport, role))) return null
  return role === "accept" ? "没有可监听的载体" : "没有可发起的载体"
}

export type ReceiptState = "revoked" | "never" | "current" | "behind"

/** Connection receipts are independent of the IP / routing receipt. */
export function connectionReceipt(entry: ConnectionNode, revision: number): ReceiptState {
  if (entry.node.revoked_at !== null) return "revoked"
  if (entry.acknowledged_revision === 0) return "never"
  return entry.acknowledged_revision === revision ? "current" : "behind"
}

export const connectionReceiptLabels: Record<ReceiptState, string> = {
  revoked: "不再下发",
  never: "从未保存",
  current: "已保存当前版本",
  behind: "待保存",
}

/** Grouped public-key display; the key itself is public. */
export function fingerprint(publicKey: string): string {
  return `${publicKey.slice(0, 8)}…${publicKey.slice(-8)}`
}

// ---------------------------------------------------------------------------
// Local validation, mirroring the controller's connection compiler.

export type ConnectionIssue = {
  /** Draft key of the listener / candidate, or null for plan-level issues. */
  itemKey: string | null
  field: string
  message: string
}

const IDENTIFIER = /^[A-Za-z0-9._-]{1,64}$/
const IPV4 = /^(25[0-5]|2[0-4][0-9]|1[0-9]{2}|[1-9]?[0-9])(\.(25[0-5]|2[0-4][0-9]|1[0-9]{2}|[1-9]?[0-9])){3}$/
const DOTTED_NUMERIC = /^[0-9.]+$/
const IPV6ISH = /^[0-9A-Fa-f:.]+$/
const LABEL = /^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$/

export function validIdentifier(value: string): boolean {
  return IDENTIFIER.test(value)
}

function validPort(value: string): boolean {
  const port = parsePort(value)
  return Number.isInteger(port) && port >= 1 && port <= 65535
}

function ipv4Unusable(host: string, bind: boolean): boolean {
  const [a = 0, b = 0, c = 0, d = 0] = host.split(".").map(Number)
  if (!bind && a === 0 && b === 0 && c === 0 && d === 0) return true
  if (a >= 224 && a <= 239) return true
  return a === 255 && b === 255 && c === 255 && d === 255
}

function ipv6Unusable(host: string, bind: boolean): boolean {
  const lower = host.toLowerCase()
  if (!bind && /^[0:]+$/.test(lower)) return true
  if (lower.startsWith("ff")) return true
  const mapped = /^::ffff:([0-9.]+)$/.exec(lower)?.[1]
  return mapped !== undefined && IPV4.test(mapped) && ipv4Unusable(mapped, bind)
}

/** Advertised host problem, or null. Non-ASCII names are left to the controller's IDNA check. */
export function hostProblem(raw: string): string | null {
  const host = raw.trim()
  if (host === "") return "主机不能为空"
  if ([...host].length > 253) return "主机最长 253 个字符"
  if (host.startsWith("[") || host.endsWith("]")) return "IPv6 地址不需要方括号"
  if (host.includes(":")) {
    if (!IPV6ISH.test(host)) return "不是有效的 IPv6 地址"
    return ipv6Unusable(host, false) ? "不能广播未指定、组播或广播地址" : null
  }
  if (DOTTED_NUMERIC.test(host.replace(/\.$/, ""))) {
    if (host.endsWith(".") || !IPV4.test(host)) return "IPv4 地址必须使用规范写法"
    return ipv4Unusable(host, false) ? "不能广播 0.0.0.0、组播或广播地址" : null
  }
  // eslint-disable-next-line no-control-regex
  if (!/^[\x00-\x7f]*$/.test(host)) return null
  const name = host.replace(/\.$/, "").toLowerCase()
  if (name === "" || name.length > 253 || !name.split(".").every((label) => LABEL.test(label))) {
    return "不是有效的 DNS 主机名"
  }
  return null
}

/** Bind socket address problem, or null. Unspecified addresses are allowed here. */
export function bindProblem(raw: string): string | null {
  const bind = raw.trim()
  if (bind === "") return "监听地址不能为空"
  if (bind.length > 128) return "监听地址过长"
  const v6 = /^\[([^\]]+)\]:([0-9]{1,5})$/.exec(bind)
  const v4 = /^([0-9.]+):([0-9]{1,5})$/.exec(bind)
  const match = v6 ?? v4
  if (!match) return "格式为 IP:端口，IPv6 写作 [地址]:端口；不支持主机名"
  const [, host = "", port = ""] = match
  if (!validPort(port)) return "端口必须在 1–65535"
  if (v6) {
    if (!IPV6ISH.test(host) || !host.includes(":")) return "不是有效的 IPv6 地址"
    return ipv6Unusable(host, true) ? "不能监听组播地址" : null
  }
  if (!IPV4.test(host)) return "不是有效的 IPv4 地址"
  return ipv4Unusable(host, true) ? "不能监听组播或广播地址" : null
}

function sameEndpoint(a: { host: string; port: string }, b: { host: string; port: string }) {
  const host = (value: string) => value.trim().replace(/\.$/, "").toLowerCase()
  return host(a.host) === host(b.host) && parsePort(a.port) === parsePort(b.port)
}

export function listenerHasEndpoint(listener: ListenerDraft, host: string, port: string): boolean {
  return listener.endpoints.some((endpoint) => sameEndpoint(endpoint, { host, port }))
}

export function validateListener(
  listener: ListenerDraft,
  others: ListenerDraft[],
  nodes: ConnectionNode[]
): ConnectionIssue[] {
  const issues: ConnectionIssue[] = []
  const add = (field: string, message: string) =>
    issues.push({ itemKey: listener.key, field, message })
  const id = listener.id.trim()
  if (!validIdentifier(id)) add("listeners.id", "ID 为 1–64 个字母、数字、点、下划线或连字符")
  else if (others.some((other) => other.key !== listener.key && other.id.trim() === id)) {
    add("listeners.id", `监听器 ID「${id}」重复`)
  }
  if (listener.nodeId === "") add("listeners.node_id", "选择监听节点")
  else {
    const ref = resolveConnectionNode(nodes, listener.nodeId)
    const blocker =
      ref.kind === "unknown" ? "节点不在清单中" : roleBlocker(ref.entry, listener.transport, "accept")
    if (blocker) add("listeners.node_id", `监听节点不可用：${blocker}`)
  }
  const bind = bindProblem(listener.bind)
  if (bind) add("listeners.bind", bind)
  const ref = listener.credentialRef.trim()
  if (needsProfile(listener.transport)) {
    if (!validIdentifier(ref)) add("listeners.credential_ref", "VLESS 需要本地配置档引用（1–64 个 ASCII 标识符）")
  } else if (ref !== "") {
    add("listeners.credential_ref", "原生载体不使用配置档引用，请清空")
  }
  if (listener.endpoints.length === 0) add("listeners.advertised_endpoints", "至少需要一个对外地址")
  if (listener.endpoints.length > CONNECTION_LIMITS.endpointsPerListener) {
    add("listeners.advertised_endpoints", `最多 ${CONNECTION_LIMITS.endpointsPerListener} 个对外地址`)
  }
  listener.endpoints.forEach((endpoint, index) => {
    const host = hostProblem(endpoint.host)
    if (host) add(`listeners.advertised_endpoints[${index}].host`, `地址 ${index + 1}：${host}`)
    if (!validPort(endpoint.port)) {
      add(`listeners.advertised_endpoints[${index}].port`, `地址 ${index + 1}：端口必须在 1–65535`)
    }
    if (listener.endpoints.slice(0, index).some((earlier) => sameEndpoint(earlier, endpoint))) {
      add(`listeners.advertised_endpoints[${index}]`, `地址 ${index + 1} 与前面的地址重复`)
    }
  })
  return issues
}

export function validateCandidate(
  candidate: CandidateDraft,
  others: CandidateDraft[],
  listeners: ListenerDraft[],
  nodes: ConnectionNode[]
): ConnectionIssue[] {
  const issues: ConnectionIssue[] = []
  const add = (field: string, message: string) =>
    issues.push({ itemKey: candidate.key, field, message })
  const id = candidate.id.trim()
  if (!validIdentifier(id)) add("candidates.id", "ID 为 1–64 个字母、数字、点、下划线或连字符")
  else if (others.some((other) => other.key !== candidate.key && other.id.trim() === id)) {
    add("candidates.id", `候选 ID「${id}」重复`)
  }
  const priority = parsePort(candidate.priority)
  if (!Number.isInteger(priority) || priority < 0 || priority > 65535) {
    add("candidates.priority", "优先级为 0–65535 的整数")
  }
  if (candidate.host.trim() === "") add("candidates.endpoint", "选择要拨号的对外地址")
  const listener = listeners.find((existing) => existing.id.trim() === candidate.listenerId)
  if (candidate.listenerId === "") add("candidates.listener_id", "选择目标监听器")
  else if (!listener) add("candidates.listener_id", `监听器「${candidate.listenerId}」不在草稿中`)
  if (candidate.dialerNodeId === "") add("candidates.dialer_node_id", "选择发起节点")
  if (!listener) return issues

  if (candidate.dialerNodeId !== "") {
    if (candidate.dialerNodeId === listener.nodeId) {
      add("candidates.dialer_node_id", "节点不能拨号自己的监听器")
    } else {
      const ref = resolveConnectionNode(nodes, candidate.dialerNodeId)
      const blocker =
        ref.kind === "unknown" ? "节点不在清单中" : roleBlocker(ref.entry, listener.transport, "dial")
      if (blocker) add("candidates.dialer_node_id", `发起节点不可用：${blocker}`)
    }
  }
  if (candidate.host.trim() !== "" && !listenerHasEndpoint(listener, candidate.host, candidate.port)) {
    add("candidates.endpoint", "地址不在监听器的对外地址列表中")
  }
  const ref = candidate.credentialRef.trim()
  if (needsProfile(listener.transport)) {
    if (!validIdentifier(ref)) add("candidates.credential_ref", "VLESS 需要发起方的本地配置档引用")
  } else if (ref !== "") {
    add("candidates.credential_ref", "原生载体不使用配置档引用，请清空")
  }
  const duplicate = others.find(
    (other) =>
      other.key !== candidate.key &&
      other.dialerNodeId === candidate.dialerNodeId &&
      other.listenerId === candidate.listenerId &&
      sameEndpoint(other, candidate)
  )
  if (duplicate && candidate.host.trim() !== "") {
    add("candidates", `与候选「${duplicate.id.trim() || "（空 ID）"}」的发起节点、监听器和地址完全相同`)
  }
  return issues
}

export function validateConnections(
  draft: ConnectionsDraft,
  nodes: ConnectionNode[]
): ConnectionIssue[] {
  const issues: ConnectionIssue[] = []
  if (draft.listeners.length > CONNECTION_LIMITS.listeners) {
    issues.push({ itemKey: null, field: "configuration", message: `最多 ${CONNECTION_LIMITS.listeners} 个监听器` })
  }
  if (draft.candidates.length > CONNECTION_LIMITS.candidates) {
    issues.push({ itemKey: null, field: "configuration", message: `最多 ${CONNECTION_LIMITS.candidates} 个拨号候选` })
  }
  for (const listener of draft.listeners) {
    issues.push(...validateListener(listener, draft.listeners, nodes))
  }
  for (const candidate of draft.candidates) {
    issues.push(...validateCandidate(candidate, draft.candidates, draft.listeners, nodes))
  }
  if (issues.length === 0 && planBytes(toPlan(draft)) > CONNECTION_LIMITS.planBytes) {
    issues.push({ itemKey: null, field: "configuration", message: "连接配置超过 128 KiB 上限" })
  }
  return issues
}

const fieldNames: [prefix: string, label: string][] = [
  ["listeners.id", "监听器 ID"],
  ["listeners.node_id", "监听节点"],
  ["listeners.bind", "本机监听地址"],
  ["listeners.advertised_endpoints", "对外地址"],
  ["listeners.credential_ref", "配置档引用"],
  ["candidates.id", "候选 ID"],
  ["candidates.dialer_node_id", "发起节点"],
  ["candidates.listener_id", "目标监听器"],
  ["candidates.endpoint", "拨号地址"],
  ["candidates.priority", "优先级"],
  ["candidates.credential_ref", "配置档引用"],
  ["candidates", "拨号候选"],
  ["configuration", "连接配置"],
]

export function connectionFieldLabel(field: string): string {
  return fieldNames.find(([prefix]) => field === prefix || field.startsWith(`${prefix}[`) || field.startsWith(`${prefix}.`))?.[1] ?? field
}

export function endpointLabel(host: string, port: string | number): string {
  const text = host.trim()
  return `${text.includes(":") ? `[${text}]` : text}:${String(port).trim()}`
}

export function uniqueId(taken: string[], stem: string): string {
  const used = new Set(taken.map((id) => id.trim()))
  let n = 1
  while (used.has(`${stem}-${n}`)) n += 1
  return `${stem}-${n}`
}

/** Lowercase slug from a node name for suggested IDs; falls back to the stem. */
export function slug(value: string, fallback: string): string {
  const text = value
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 40)
  return text === "" ? fallback : text
}

// ---------------------------------------------------------------------------
// Node-first workflow helpers. They only derive suggestions and views from
// the draft; the DTO sent is still produced by `toPlan`.

export const transportSlugs: Record<Transport, string> = {
  native_udp: "udp",
  native_tcp: "tcp",
  vless: "vless",
}

/** `stem` when free, otherwise `stem-2`, `stem-3`… (IDs are case-sensitive). */
export function stableId(stem: string, taken: Iterable<string>): string {
  const used = new Set([...taken].map((id) => id.trim()))
  const base = stem.slice(0, CONNECTION_LIMITS.idLength - 4)
  if (!used.has(base)) return base
  let n = 2
  while (used.has(`${base}-${n}`)) n += 1
  return `${base}-${n}`
}

function nodeStem(nodes: ConnectionNode[], id: string): string {
  const ref = resolveConnectionNode(nodes, id)
  const name = ref.kind === "unknown" ? id : ref.entry.node.name
  return slug(name, "node").slice(0, 20)
}

/** Readable, stable listener ID such as `lax-udp`. */
export function suggestListenerId(
  nodes: ConnectionNode[],
  nodeId: string,
  transport: Transport,
  taken: Iterable<string>
): string {
  return stableId(`${nodeStem(nodes, nodeId)}-${transportSlugs[transport]}`, taken)
}

/** Readable, stable candidate ID such as `sha-to-lax-udp`. */
export function suggestCandidateId(
  nodes: ConnectionNode[],
  dialerNodeId: string,
  acceptorNodeId: string,
  transport: Transport,
  taken: Iterable<string>
): string {
  const dialer = dialerNodeId === "" ? "dial" : nodeStem(nodes, dialerNodeId)
  const acceptor = acceptorNodeId === "" ? "node" : nodeStem(nodes, acceptorNodeId)
  return stableId(`${dialer}-to-${acceptor}-${transportSlugs[transport]}`, taken)
}

// --- bind address --------------------------------------------------------

export type BindMode = "ipv4_any" | "ipv6_any" | "address"
export type BindParts = { mode: BindMode; address: string; port: string }

/** Splits a raw bind into interface choice and port; `composeBind` reverses it exactly. */
export function parseBind(raw: string): BindParts {
  const text = raw.trim()
  if (text === "") return { mode: "ipv4_any", address: "", port: "" }
  const v6 = /^\[([^\]]*)\]:([^:]*)$/.exec(text)
  if (v6) {
    const [, address = "", port = ""] = v6
    return address === "::" ? { mode: "ipv6_any", address: "", port } : { mode: "address", address, port }
  }
  const v4 = /^([^:[\]]*):([^:]*)$/.exec(text)
  if (v4) {
    const [, address = "", port = ""] = v4
    return address === "0.0.0.0" ? { mode: "ipv4_any", address: "", port } : { mode: "address", address, port }
  }
  return { mode: "address", address: text, port: "" }
}

export function composeBind(parts: BindParts): string {
  const port = parts.port.trim()
  if (parts.mode === "ipv4_any") return `0.0.0.0:${port}`
  if (parts.mode === "ipv6_any") return `[::]:${port}`
  const address = parts.address.trim()
  return address.includes(":") ? `[${address}]:${port}` : `${address}:${port}`
}

/**
 * Advisory hint for an advertised host that is syntactically valid but likely
 * not what other nodes can reach. Never blocks: the controller decides.
 */
export function endpointHint(raw: string, nodes: ConnectionNode[]): string | null {
  const host = raw.trim().replace(/\.$/, "").toLowerCase()
  if (host === "") return null
  if (nodes.some((entry) => entry.node.mesh_ipv4 === host || entry.node.mesh_ipv6.toLowerCase() === host)) {
    return "这是 Mesh 隧道内地址，隧道建立前其他节点用它连不上。请填写公网 / 局域网 IP 或域名。"
  }
  if (!host.includes(".") && !host.includes(":") && nodes.some((entry) => entry.node.name.toLowerCase() === host)) {
    return "节点名称不会自动解析成地址；除非你的 DNS 能解析它，请填写 IP 或完整域名。"
  }
  if (host === "localhost" || host === "::1" || /^127\./.test(host)) {
    return "回环地址只适用于同一台机器上的节点。"
  }
  return null
}

// --- per-node view -------------------------------------------------------

export type NodeScope = {
  /** Listeners this node owns. */
  listeners: ListenerDraft[]
  /** Candidates this node dials. */
  outgoing: CandidateDraft[]
  /** Candidates other nodes dial into this node's listeners. */
  incoming: CandidateDraft[]
}

export function nodeScope(draft: ConnectionsDraft, nodeId: string): NodeScope {
  const owned = new Set(
    draft.listeners.filter((listener) => listener.nodeId === nodeId).map((listener) => listener.id.trim())
  )
  return {
    listeners: draft.listeners.filter((listener) => listener.nodeId === nodeId),
    outgoing: draft.candidates.filter((candidate) => candidate.dialerNodeId === nodeId),
    incoming: draft.candidates.filter(
      (candidate) => candidate.dialerNodeId !== nodeId && owned.has(candidate.listenerId)
    ),
  }
}

// --- dial entries --------------------------------------------------------

export type DialEntry = {
  /** Option value; JSON keeps IPv6 colons unambiguous. */
  value: string
  listener: ListenerDraft
  host: string
  port: string
  /** Why the dialer cannot use this entry, or null. */
  blocker: string | null
}

export const entryValue = (listenerId: string, host: string, port: string) =>
  JSON.stringify([listenerId.trim(), host.trim(), port.trim()])

/**
 * Every advertised endpoint of the target node's listeners, flagged when the
 * dialer lacks the carrier or already dials exactly that entry.
 */
export function dialEntries(
  draft: ConnectionsDraft,
  nodes: ConnectionNode[],
  dialerNodeId: string,
  targetNodeId: string,
  candidateKey: string
): DialEntry[] {
  const dialer = dialerNodeId === "" ? null : resolveConnectionNode(nodes, dialerNodeId)
  const entries: DialEntry[] = []
  for (const listener of draft.listeners) {
    if (listener.nodeId !== targetNodeId || listener.id.trim() === "") continue
    for (const endpoint of listener.endpoints) {
      if (endpoint.host.trim() === "") continue
      const taken = draft.candidates.some(
        (candidate) =>
          candidate.key !== candidateKey &&
          candidate.dialerNodeId === dialerNodeId &&
          candidate.listenerId === listener.id.trim() &&
          sameEndpoint(candidate, endpoint)
      )
      const blocker =
        dialer === null
          ? null
          : dialer.kind === "unknown"
            ? "发起节点不在清单中"
            : roleBlocker(dialer.entry, listener.transport, "dial") ?? (taken ? "已配置过这个入口" : null)
      entries.push({
        value: entryValue(listener.id, endpoint.host, endpoint.port),
        listener,
        host: endpoint.host.trim(),
        port: endpoint.port.trim(),
        blocker,
      })
    }
  }
  return entries
}

/** Nodes that own at least one listener in the draft, other than `exclude`. */
export function listeningNodeIds(draft: ConnectionsDraft, exclude: string): string[] {
  return [...new Set(draft.listeners.map((listener) => listener.nodeId))].filter(
    (id) => id !== "" && id !== exclude
  )
}

// --- primary / backup ----------------------------------------------------

function acceptorOf(candidate: CandidateDraft, listeners: ListenerDraft[]): string | null {
  return listeners.find((listener) => listener.id.trim() === candidate.listenerId)?.nodeId ?? null
}

/** Candidates from one dialer to one acceptor node, in primary-first order. */
export function pairCandidates(
  draft: ConnectionsDraft,
  dialerNodeId: string,
  acceptorNodeId: string
): CandidateDraft[] {
  const priority = (candidate: CandidateDraft) => {
    const value = parsePort(candidate.priority)
    return Number.isInteger(value) ? value : Number.POSITIVE_INFINITY
  }
  return draft.candidates
    .filter(
      (candidate) =>
        candidate.dialerNodeId === dialerNodeId && acceptorOf(candidate, draft.listeners) === acceptorNodeId
    )
    .sort((a, b) => priority(a) - priority(b) || (a.id.trim() < b.id.trim() ? -1 : a.id.trim() > b.id.trim() ? 1 : 0))
}

/** Lower-first: the first entry for a pair is 0, later ones go behind it. */
export function nextPriority(draft: ConnectionsDraft, dialerNodeId: string, acceptorNodeId: string, exceptKey: string): string {
  const values = pairCandidates(draft, dialerNodeId, acceptorNodeId)
    .filter((candidate) => candidate.key !== exceptKey)
    .map((candidate) => parsePort(candidate.priority))
    .filter((value) => Number.isInteger(value))
  if (values.length === 0) return "0"
  return String(Math.min(65535, Math.max(...values) + 10))
}

export type DialRole = { rank: number; total: number; tied: boolean }

/** Position of a candidate among its dialer → acceptor pair. */
export function dialRole(candidate: CandidateDraft, draft: ConnectionsDraft): DialRole | null {
  const acceptor = acceptorOf(candidate, draft.listeners)
  if (acceptor === null || candidate.dialerNodeId === "") return null
  const pair = pairCandidates(draft, candidate.dialerNodeId, acceptor)
  const rank = pair.findIndex((existing) => existing.key === candidate.key)
  if (rank < 0) return null
  const tied = pair.some(
    (existing) => existing.key !== candidate.key && existing.priority.trim() === candidate.priority.trim()
  )
  return { rank, total: pair.length, tied }
}

export function dialRoleLabel(role: DialRole | null): string | null {
  if (role === null) return null
  if (role.total === 1) return "唯一入口"
  return role.rank === 0 ? "主用" : `备用 ${role.rank}`
}

// --- form autofill -------------------------------------------------------
// Suggestions follow the user's choices until the user overrides them: a
// field is regenerated only while it still holds the previous suggestion.

function listenerTransportFor(entry: ConnectionNode | undefined, current: Transport): Transport {
  if (!entry) return current
  if (canServe(entry, current, "accept")) return current
  return transports.find((transport) => canServe(entry, transport, "accept")) ?? current
}

export function autofillListener(
  next: ListenerDraft,
  previous: ListenerDraft,
  mode: "new" | "edit",
  others: ListenerDraft[],
  nodes: ConnectionNode[]
): ListenerDraft {
  let form = next
  if (form.nodeId !== previous.nodeId) {
    const ref = resolveConnectionNode(nodes, form.nodeId)
    form = { ...form, transport: listenerTransportFor(ref.kind === "active" ? ref.entry : undefined, form.transport) }
  }
  if (mode === "new") {
    const taken = others.map((listener) => listener.id)
    const was = suggestListenerId(nodes, previous.nodeId, previous.transport, taken)
    // Only while the user has not just typed an ID of their own.
    if (form.id === previous.id && (previous.id.trim() === was || previous.id.trim() === "")) {
      form = { ...form, id: suggestListenerId(nodes, form.nodeId, form.transport, taken) }
    }
  }
  const oldPort = parseBind(previous.bind).port.trim()
  const newPort = parseBind(form.bind).port.trim()
  if (oldPort !== newPort) {
    form = {
      ...form,
      endpoints: form.endpoints.map((endpoint) =>
        endpoint.port.trim() === "" || endpoint.port.trim() === oldPort ? { ...endpoint, port: newPort } : endpoint
      ),
    }
  }
  return form
}

function transportOfEntry(draft: ConnectionsDraft, form: CandidateForm): Transport {
  const listener =
    draft.listeners.find((existing) => existing.id.trim() === form.listenerId) ??
    draft.listeners.find((existing) => existing.nodeId === form.targetNodeId)
  return listener?.transport ?? "native_udp"
}

export function autofillCandidate(
  next: CandidateForm,
  previous: CandidateForm,
  mode: "new" | "edit",
  draft: ConnectionsDraft,
  nodes: ConnectionNode[]
): CandidateForm {
  let form = next
  const others = draft.candidates.filter((candidate) => candidate.key !== form.key)
  const peersChanged = form.dialerNodeId !== previous.dialerNodeId || form.targetNodeId !== previous.targetNodeId
  if (peersChanged) {
    const entries = dialEntries(draft, nodes, form.dialerNodeId, form.targetNodeId, form.key)
    const usable = entries.filter((entry) => entry.blocker === null)
    const current = entries.find(
      (entry) => entry.value === entryValue(form.listenerId, form.host, form.port) && entry.blocker === null
    )
    if (!current) {
      const [only] = usable.length === 1 ? usable : []
      if (only) form = { ...form, listenerId: only.listener.id.trim(), host: only.host, port: only.port }
      // A different target invalidates the entry; a different dialer keeps
      // it visible (and flagged) for an explicit choice.
      else if (form.targetNodeId !== previous.targetNodeId) form = { ...form, listenerId: "", host: "", port: "" }
    }
  }
  if (mode === "new") {
    const scoped = { ...draft, candidates: others }
    const wasPriority =
      previous.dialerNodeId !== "" && previous.targetNodeId !== ""
        ? nextPriority(scoped, previous.dialerNodeId, previous.targetNodeId, form.key)
        : "0"
    if (previous.priority.trim() === wasPriority && form.priority === previous.priority) {
      form = {
        ...form,
        priority:
          form.dialerNodeId !== "" && form.targetNodeId !== ""
            ? nextPriority(scoped, form.dialerNodeId, form.targetNodeId, form.key)
            : "0",
      }
    }
    const taken = others.map((candidate) => candidate.id)
    const wasId = suggestCandidateId(nodes, previous.dialerNodeId, previous.targetNodeId, transportOfEntry(draft, previous), taken)
    if ((previous.id.trim() === wasId || previous.id.trim() === "") && form.id === previous.id) {
      form = { ...form, id: suggestCandidateId(nodes, form.dialerNodeId, form.targetNodeId, transportOfEntry(draft, form), taken) }
    }
  }
  return form
}

/** Initial form for a new candidate in the given context. */
export function newCandidateForm(
  key: string,
  draft: ConnectionsDraft,
  nodes: ConnectionNode[],
  context: { dialerNodeId?: string; targetNodeId?: string; listenerId?: string }
): CandidateForm {
  const listener = context.listenerId
    ? draft.listeners.find((existing) => existing.id.trim() === context.listenerId)
    : undefined
  const blank: CandidateForm = {
    key,
    id: "",
    dialerNodeId: "",
    listenerId: "",
    host: "",
    port: "",
    priority: "0",
    credentialRef: "",
    targetNodeId: "",
  }
  const chosen: CandidateForm = {
    ...blank,
    dialerNodeId: context.dialerNodeId ?? "",
    targetNodeId: listener?.nodeId ?? context.targetNodeId ?? "",
  }
  let form = autofillCandidate(chosen, blank, "new", draft, nodes)
  if (listener) {
    const usable = dialEntries(draft, nodes, form.dialerNodeId, form.targetNodeId, key).filter(
      (entry) => entry.listener.key === listener.key && entry.blocker === null
    )
    const [only] = usable.length === 1 ? usable : []
    form = only
      ? { ...form, listenerId: only.listener.id.trim(), host: only.host, port: only.port }
      : { ...form, listenerId: listener.id.trim(), host: "", port: "" }
    const taken = draft.candidates.map((candidate) => candidate.id)
    form = { ...form, id: suggestCandidateId(nodes, form.dialerNodeId, form.targetNodeId, listener.transport, taken) }
  }
  return form
}

/** Initial form for a new listener on `nodeId` (or the first node able to accept). */
export function newListenerForm(
  key: string,
  draft: ConnectionsDraft,
  nodes: ConnectionNode[],
  nodeId: string | null
): ListenerDraft {
  const ready = (entry: ConnectionNode) => transports.some((transport) => canServe(entry, transport, "accept"))
  const ref = nodeId ? resolveConnectionNode(nodes, nodeId) : null
  const owner = ref?.kind === "active" ? ref.entry : nodeId ? undefined : nodes.find(ready)
  const ownerId = nodeId ?? owner?.node.id ?? ""
  const transport = listenerTransportFor(owner, "native_udp")
  return {
    key,
    id: suggestListenerId(nodes, ownerId, transport, draft.listeners.map((listener) => listener.id)),
    nodeId: ownerId,
    transport,
    bind: composeBind({ mode: "ipv4_any", address: "", port: "" }),
    endpoints: [{ key: `${key}-e0`, host: "", port: "" }],
    credentialRef: "",
  }
}

/** A connections snapshot with the server time it was read at. */
export type StampedSnapshot = { snapshot: ConnectionsSnapshot; readAt: number }

/**
 * Freshest snapshot among page renders, polls and action responses (save,
 * explicit reload). A higher revision always wins, so nothing rolls back;
 * at the same revision the later read wins, because receipts and contact
 * change without bumping the revision. All `readAt` values come from the
 * web server's clock. Only ever a mount / display source: a kept dirty draft
 * restores its own CAS base from the draft store.
 */
export function latestSnapshot(...sources: (StampedSnapshot | null)[]): StampedSnapshot | null {
  let best: StampedSnapshot | null = null
  for (const source of sources) {
    if (source === null) continue
    if (
      best === null ||
      source.snapshot.revision > best.snapshot.revision ||
      (source.snapshot.revision === best.snapshot.revision && source.readAt > best.readAt)
    ) {
      best = source
    }
  }
  return best
}
