import type {
  ConnectionNode,
  MeshAccessGrant,
  MeshAccessPolicy,
  MeshAccessSnapshot,
  PolicyDiagnostic,
} from "@meshway/control-client"

// Editor-side model for mesh IPv4 access grants. A grant is one exact,
// ordered, directional path (source, via…, destination). The draft is the
// generated MeshAccessPolicy itself: there are no free-text fields, so nothing
// needs a separate form representation outside the open builder.

/** Controller caps for the access policy. The editor never claims more. */
export const ACCESS_LIMITS = {
  grants: 128,
  pathMin: 2,
  pathMax: 8,
  /** Serialized policy cap; the controller enforces its own. */
  policyBytes: 128 * 1024,
  /** Request body cap of the save endpoint. */
  requestBytes: 256 * 1024,
} as const

/** Contract shape of any grant ID, including ones written by other clients. */
export const GRANT_ID_PATTERN = /^[A-Za-z0-9._-]{1,64}$/

/** IDs this editor generates: never typed by the administrator. */
export const GENERATED_GRANT_ID = /^[0-9a-f]{64}$/

export function newGrantId(
  fill: (bytes: Uint8Array) => void = (bytes) => {
    globalThis.crypto.getRandomValues(bytes)
  }
): string {
  const bytes = new Uint8Array(32)
  fill(bytes)
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("")
}

export const emptyAccess: MeshAccessPolicy = { grants: [] }

// Keys are listed explicitly so the comparison never depends on the object
// key order the generated schema happens to produce.
export const pathKey = (path: readonly string[]) => JSON.stringify(path)
export const accessKey = (policy: MeshAccessPolicy) =>
  JSON.stringify(policy.grants.map((grant) => [grant.id, grant.path]))

export function sameAccess(a: MeshAccessPolicy, b: MeshAccessPolicy): boolean {
  return accessKey(a) === accessKey(b)
}

export function copyAccess(policy: MeshAccessPolicy): MeshAccessPolicy {
  return { grants: policy.grants.map((grant) => ({ id: grant.id, path: [...grant.path] })) }
}

export function reversePath(path: readonly string[]): string[] {
  return [...path].reverse()
}

export type GrantChange = "new" | "modified" | "unchanged"

export function grantChange(grant: MeshAccessGrant, base: MeshAccessPolicy): GrantChange {
  const saved = base.grants.find((candidate) => candidate.id === grant.id)
  if (!saved) return "new"
  return pathKey(saved.path) === pathKey(grant.path) ? "unchanged" : "modified"
}

export function removedGrantCount(draft: MeshAccessPolicy, base: MeshAccessPolicy): number {
  const ids = new Set(draft.grants.map((grant) => grant.id))
  return base.grants.filter((grant) => !ids.has(grant.id)).length
}

/** The exact reverse path is granted too (it is never implied). */
export function hasReverse(grant: MeshAccessGrant, grants: readonly MeshAccessGrant[]): boolean {
  const reverse = pathKey(reversePath(grant.path))
  return grants.some((candidate) => pathKey(candidate.path) === reverse)
}

// --- Node references ------------------------------------------------------

/**
 * A grant may only reference active nodes that have published a connection
 * key. Anything else stays visible and flagged so it can be edited or
 * deleted explicitly; it is never dropped or rewritten on load.
 */
export type AccessRef =
  | { kind: "ready"; entry: ConnectionNode }
  | { kind: "no_key"; entry: ConnectionNode }
  | { kind: "revoked"; entry: ConnectionNode }
  | { kind: "unknown"; id: string }

export function resolveAccessNode(inventory: readonly ConnectionNode[], id: string): AccessRef {
  const entry = inventory.find((candidate) => candidate.node.id === id)
  if (!entry) return { kind: "unknown", id }
  if (entry.node.revoked_at !== null) return { kind: "revoked", entry }
  if (entry.public_key === null) return { kind: "no_key", entry }
  return { kind: "ready", entry }
}

export function accessRefLabel(ref: AccessRef): string {
  switch (ref.kind) {
    case "ready":
      return ref.entry.node.name
    case "no_key":
      return `${ref.entry.node.name}（未发布公钥）`
    case "revoked":
      return `${ref.entry.node.name}（已撤销）`
    case "unknown":
      return `未知节点 ${ref.id.slice(0, 12)}…`
  }
}

const refProblems = {
  no_key: "节点未发布连接公钥，不能出现在授权路径中。",
  revoked: "节点已撤销，授权不能再引用它；请修改或删除这条授权。",
  unknown: "节点不在清单中，授权不能引用它；请修改或删除这条授权。",
} as const

export function refProblem(ref: AccessRef): string | null {
  return ref.kind === "ready" ? null : refProblems[ref.kind]
}

export function readyNodes(inventory: readonly ConnectionNode[]): ConnectionNode[] {
  return inventory.filter((entry) => resolveAccessNode(inventory, entry.node.id).kind === "ready")
}

// --- Validation -------------------------------------------------------------

/** `grantId` null: policy-level. `field` mirrors the request body. */
export type AccessIssue = { grantId: string | null; field: string; message: string }

/** Path-only checks: shape, uniqueness and every node usable. */
export function validatePath(
  path: readonly string[],
  inventory: readonly ConnectionNode[],
  grantId: string | null = null,
  prefix = "path"
): AccessIssue[] {
  const issues: AccessIssue[] = []
  const issue = (field: string, message: string) => issues.push({ grantId, field, message })
  if (path.length < ACCESS_LIMITS.pathMin) {
    issue(prefix, "路径至少包含来源和目标两个节点。")
  } else if (path.length > ACCESS_LIMITS.pathMax) {
    issue(prefix, `路径最多 ${ACCESS_LIMITS.pathMax} 个节点（含来源和目标）。`)
  }
  const seen = new Set<string>()
  path.forEach((id, index) => {
    const field = `${prefix}[${index}]`
    if (id === "") {
      issue(field, "请选择节点。")
      return
    }
    if (seen.has(id)) {
      issue(field, "同一节点不能在路径中出现两次。")
      return
    }
    seen.add(id)
    const problem = refProblem(resolveAccessNode(inventory, id))
    if (problem) issue(field, problem)
  })
  return issues
}

/** Whole-policy checks run before saving; the controller validates again. */
export function validateAccess(
  policy: MeshAccessPolicy,
  inventory: readonly ConnectionNode[]
): AccessIssue[] {
  const issues: AccessIssue[] = []
  if (policy.grants.length > ACCESS_LIMITS.grants) {
    issues.push({ grantId: null, field: "grants", message: `最多 ${ACCESS_LIMITS.grants} 条授权。` })
  }
  if (new TextEncoder().encode(JSON.stringify(policy)).length > ACCESS_LIMITS.policyBytes) {
    issues.push({ grantId: null, field: "grants", message: "授权总大小超过 128 KiB 上限。" })
  }
  const ids = new Set<string>()
  const paths = new Set<string>()
  policy.grants.forEach((grant, index) => {
    const prefix = `grants[${index}]`
    if (!GRANT_ID_PATTERN.test(grant.id)) {
      issues.push({ grantId: grant.id, field: `${prefix}.id`, message: "授权 ID 格式无效。" })
    } else if (ids.has(grant.id)) {
      issues.push({ grantId: grant.id, field: `${prefix}.id`, message: "授权 ID 重复。" })
    }
    ids.add(grant.id)
    issues.push(...validatePath(grant.path, inventory, grant.id, `${prefix}.path`))
    const key = pathKey(grant.path)
    if (paths.has(key)) {
      issues.push({ grantId: grant.id, field: `${prefix}.path`, message: "已有另一条完全相同路径的授权。" })
    }
    paths.add(key)
  })
  return issues
}

export function issuesForGrant(issues: readonly AccessIssue[], id: string): AccessIssue[] {
  return issues.filter((issue) => issue.grantId === id)
}

/** Controller diagnostics carry the grant ID in `rule_id`. */
export function diagnosticsForGrant(
  diagnostics: readonly PolicyDiagnostic[],
  id: string
): PolicyDiagnostic[] {
  return diagnostics.filter((diagnostic) => diagnostic.rule_id === id)
}

// --- Builder form -----------------------------------------------------------

export type GrantForm = {
  /** Generated when the builder opens, so a restored session keeps it. */
  id: string
  /** "" while not chosen. */
  source: string
  via: string[]
  destination: string
  /** Also add the exact reverse path as a separate grant. Default off. */
  addReverse: boolean
}

/** One opening of the grant builder. */
export type GrantSession = {
  id: number
  mode: "new" | "edit"
  initial: GrantForm
  /** Unapplied form contents to resume with (restored after navigating away). */
  working?: GrantForm
}

/** An open builder holding edits that are not applied to the draft yet. */
export type WorkingGrant = { session: GrantSession; form: GrantForm }

export function newGrantForm(id: string): GrantForm {
  return { id, source: "", via: [], destination: "", addReverse: false }
}

export function toGrantForm(grant: MeshAccessGrant): GrantForm {
  const [source = "", ...rest] = grant.path
  const destination = rest.length > 0 ? (rest[rest.length - 1] ?? "") : ""
  return {
    id: grant.id,
    source,
    via: rest.slice(0, -1),
    destination,
    addReverse: false,
  }
}

export function formPath(form: GrantForm): string[] {
  return [form.source, ...form.via, form.destination]
}

export function sameGrantForm(a: GrantForm, b: GrantForm): boolean {
  return (
    a.id === b.id &&
    a.addReverse === b.addReverse &&
    pathKey(formPath(a)) === pathKey(formPath(b))
  )
}

/** Form checks against the other grants in the draft. */
export function validateGrantForm(
  form: GrantForm,
  others: readonly MeshAccessGrant[],
  inventory: readonly ConnectionNode[]
): AccessIssue[] {
  const path = formPath(form)
  const issues = validatePath(path, inventory, form.id)
  const key = pathKey(path)
  if (others.some((grant) => pathKey(grant.path) === key)) {
    issues.push({ grantId: form.id, field: "path", message: "已有完全相同路径的授权。" })
  }
  const added = 1 + (form.addReverse && !reverseExists(form, others) ? 1 : 0)
  if (others.length + added > ACCESS_LIMITS.grants) {
    issues.push({
      grantId: form.id,
      field: "grants",
      message: `最多 ${ACCESS_LIMITS.grants} 条授权${added > 1 ? "（含反向授权）" : ""}。`,
    })
  }
  return issues
}

/** The reverse of the form's path is already among the other grants. */
export function reverseExists(form: GrantForm, others: readonly MeshAccessGrant[]): boolean {
  const reverse = pathKey(reversePath(formPath(form)))
  return others.some((grant) => pathKey(grant.path) === reverse)
}

/**
 * Grants produced by applying a valid form: the grant itself, plus the exact
 * reverse path under a fresh ID when explicitly requested and not already
 * present.
 */
export function grantsFromForm(
  form: GrantForm,
  others: readonly MeshAccessGrant[],
  makeId: () => string = newGrantId
): MeshAccessGrant[] {
  const path = formPath(form)
  const grant = { id: form.id, path }
  if (!form.addReverse || reverseExists(form, others)) return [grant]
  return [grant, { id: makeId(), path: reversePath(path) }]
}

export type PathRole = "source" | "via" | "destination"

export function pathRole(index: number, length: number): PathRole {
  return index === 0 ? "source" : index === length - 1 ? "destination" : "via"
}

export const pathRoleLabels: Record<PathRole, string> = {
  source: "来源",
  via: "经由",
  destination: "目标",
}

/** Field paths from validation or diagnostics, for display. */
export function accessFieldLabel(field: string): string {
  if (field === "grants") return "授权列表"
  const match = /(?:^|\.)path(?:\[(\d+)\])?$/.exec(field)
  if (match) return match[1] === undefined ? "路径" : `路径第 ${Number(match[1]) + 1} 个节点`
  if (field.endsWith(".id")) return "授权 ID"
  return field
}

// --- Save outcome -------------------------------------------------------------

export type AccessSaveFailure =
  | { status: "conflict"; message: string }
  | { status: "invalid"; message: string }
  | { status: "error"; message: string }

/**
 * Maps a non-401 save failure to what the editor shows. None of them carry a
 * replacement policy: the draft and its CAS base stay exactly as they were.
 * `httpStatus` is null for transport failures (no HTTP response).
 */
export function accessSaveFailure(
  httpStatus: number | null,
  expectedRevision: number,
  detail: string
): AccessSaveFailure {
  if (httpStatus === 409) {
    return {
      status: "conflict",
      message: `Controller 的配置版本已不是 r${expectedRevision}：节点、公钥、路由、连接或授权在你编辑期间被修改。未保存任何内容，草稿仍保留在编辑器中。`,
    }
  }
  if (httpStatus === 422) {
    return {
      status: "invalid",
      message:
        "Controller 拒绝保存：授权没有通过校验（路径形状、重复节点，或引用了已撤销 / 未绑定公钥的节点），或超出容量（128 条授权、128 KiB、每个节点最多涉及 64 个对端身份）。已保存的授权保持不变。",
    }
  }
  return { status: "error", message: `保存失败：${detail}` }
}

/**
 * Editor state after a save completes. Only a successful save moves the CAS
 * base; the controller's canonical policy replaces the draft only when the
 * draft is still exactly what was sent. Failures leave both untouched.
 */
export function settleSave(
  state: { base: MeshAccessSnapshot; draft: MeshAccessPolicy },
  result: { status: "saved"; snapshot: MeshAccessSnapshot } | AccessSaveFailure,
  sentKey: string
): { base: MeshAccessSnapshot; draft: MeshAccessPolicy } {
  if (result.status !== "saved") return state
  return {
    base: result.snapshot,
    draft: accessKey(state.draft) === sentKey ? copyAccess(result.snapshot.policy) : state.draft,
  }
}
