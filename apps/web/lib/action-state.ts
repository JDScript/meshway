import type {
  CreateEnrollmentTokenResponse,
  NodeSummary,
  PolicySnapshot,
  PreviewPolicyResponse,
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

export const enrollmentTtlOptions = [
  { seconds: 900, label: "15 分钟" },
  { seconds: 3600, label: "1 小时" },
  { seconds: 86400, label: "24 小时" },
] as const

export const defaultEnrollmentTtl = 3600
