import type {
  ConnectionNode,
  ConnectionRuntimesSnapshot,
  ConnectionsSnapshot,
  CreateEnrollmentTokenResponse,
  MeshAccessSnapshot,
  NodeList,
  NodeSummary,
  PolicySnapshot,
  PreviewFlowResponse,
  PreviewPolicyResponse,
  TagSnapshot,
} from "@meshway/control-client"

// Serializable results returned by Server Actions to client forms. None of
// them ever contain the administrator credential.

export type LoginState =
  | { status: "idle" }
  | { status: "malformed"; message: string }
  | { status: "invalid" }
  | { status: "unavailable"; detail: string }

export type EnrollmentState =
  | { status: "idle" }
  | { status: "error"; message: string }
  | {
      status: "created"
      enrollment: CreateEnrollmentTokenResponse
      controllerUrl: string
      createdAt: number
    }

export type RevokeState =
  | { status: "idle" }
  | { status: "error"; message: string }
  | { status: "revoked"; node: NodeSummary }

export type CountryState =
  | { status: "idle" }
  | { status: "error"; message: string }
  | { status: "conflict"; message: string }
  | { status: "saved"; node: NodeSummary }

export type PreviewResult =
  | { status: "previewed"; response: PreviewPolicyResponse }
  | { status: "error"; message: string }

/** `conflict` and `invalid` never carry a replacement policy: the draft stays. */
export type SaveResult =
  | { status: "saved"; snapshot: PolicySnapshot }
  | { status: "conflict"; message: string }
  | { status: "invalid"; message: string }
  | { status: "error"; message: string }

export type ReloadResult =
  | { status: "loaded"; snapshot: PolicySnapshot; nodes: NodeSummary[] }
  | { status: "error"; message: string }

/** `conflict` and `invalid` never carry a replacement plan: the draft stays. */
export type ConnectionsSaveResult =
  /** `fetchedAt`: when the server received it, on the same clock as page reads. */
  | { status: "saved"; snapshot: ConnectionsSnapshot; fetchedAt: number }
  | { status: "conflict"; message: string }
  | { status: "invalid"; message: string }
  | { status: "error"; message: string }

export type ConnectionsReloadResult =
  | { status: "loaded"; snapshot: ConnectionsSnapshot; fetchedAt: number }
  | { status: "error"; message: string }

/** `conflict` and `invalid` never carry a replacement policy: the draft stays. */
export type AccessSaveResult =
  | { status: "saved"; snapshot: MeshAccessSnapshot }
  | { status: "conflict"; message: string }
  | { status: "invalid"; message: string }
  | { status: "error"; message: string }

/** Access rules, the inventory and the tag catalog read together. */
export type AccessReloadResult =
  | { status: "loaded"; snapshot: MeshAccessSnapshot; inventory: ConnectionNode[]; tags: TagSnapshot }
  | { status: "error"; message: string }

/** `conflict` and `invalid` never carry a replacement: the draft stays. */
export type TagSaveResult =
  | { status: "saved"; snapshot: TagSnapshot }
  | { status: "conflict"; message: string }
  | { status: "invalid"; message: string }
  | { status: "error"; message: string }

export type TagReloadResult =
  | { status: "loaded"; snapshot: TagSnapshot; inventory: ConnectionNode[]; access: MeshAccessSnapshot }
  | { status: "error"; message: string }

/** The controller's plan for one flow under the saved access and routing policies. */
export type FlowPreviewResult =
  | { status: "previewed"; response: PreviewFlowResponse }
  | { status: "error"; message: string }

/** Background observation refresh; never touches an editor's draft base. */
export type RuntimeReadResult =
  | { status: "loaded"; snapshot: ConnectionRuntimesSnapshot; fetchedAt: number }
  | { status: "error"; message: string }

export type PartRead<T> = { ok: true; data: T } | { ok: false; error: string }

/**
 * One bounded status refresh for a node page: public inventory, connection
 * snapshot (receipts) and runtime self-reports, read together at `fetchedAt`.
 * Each part fails independently; nothing here is ever a draft base.
 */
export type NodeStatusResult =
  | {
      status: "loaded"
      fetchedAt: number
      nodes: PartRead<NodeList>
      connections: PartRead<ConnectionsSnapshot>
      runtimes: PartRead<ConnectionRuntimesSnapshot>
    }
  | { status: "error"; message: string }

export const enrollmentTtlOptions = [
  { seconds: 900, label: "15 分钟" },
  { seconds: 3600, label: "1 小时" },
  { seconds: 86400, label: "24 小时" },
] as const

export const defaultEnrollmentTtl = 3600
