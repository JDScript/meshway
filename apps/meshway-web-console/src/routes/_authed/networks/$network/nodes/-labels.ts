import type {
  KeyAlgorithm,
  NodeIdentity,
  TransportProtocol,
} from "@meshway/control-client"

import { agoTime } from "@/lib/format"
import type { Preset } from "@/lib/nodes/listener-model"
import type { EnrollmentKeyState, KeyExpiry } from "@/lib/nodes/model"
import type { NodeStatus } from "@/lib/nodes/status"
import { statusLabel } from "@/lib/state-labels"
import { m } from "@/paraglide/messages.js"

/**
 * A node's status in a row or a header: an offline node says when it was
 * last seen (by this browser's clock, which ticks between reports); any
 * other status, its word.
 */
export function statusText(
  status: NodeStatus,
  seen: string | null,
  now: number
): string {
  if (status === "expired" && seen)
    return m.nodes_last_seen({ time: agoTime(seen, now) ?? seen })
  return statusLabel[status]()
}

/** The user's email, or the tags, as one line. */
export const identityText = (identity: NodeIdentity) =>
  identity.type === "user" ? identity.user : identity.tags.join(", ")

export const presetLabel: Record<Preset, () => string> = {
  native_udp: () => m.nodes_preset_native_udp(),
  native_tcp: () => m.nodes_preset_native_tcp(),
  vless_tcp: () => m.nodes_preset_vless_tcp(),
  vless_websocket: () => m.nodes_preset_vless_websocket(),
}

export const protocolLabel: Record<TransportProtocol, () => string> = {
  native_udp: () => m.nodes_preset_native_udp(),
  native_tcp: () => m.nodes_preset_native_tcp(),
  vless: () => m.nodes_protocol_vless(),
  trojan: () => m.nodes_protocol_trojan(),
}

export const algorithmLabel: Record<KeyAlgorithm, string> = {
  ed25519: "Ed25519",
  x25519: "X25519",
}

export type KeyPurpose = "control" | "dataplane"

export const purposeLabel: Record<KeyPurpose, () => string> = {
  control: () => m.nodes_key_control(),
  dataplane: () => m.nodes_key_dataplane(),
}

export const keyStateLabel: Record<EnrollmentKeyState, () => string> = {
  pending: () => m.nodes_enrollment_pending(),
  consumed: () => m.nodes_enrollment_consumed(),
  revoked: () => m.nodes_enrollment_revoked(),
  expired: () => m.nodes_enrollment_expired(),
}

export const expiryLabel: Record<KeyExpiry, () => string> = {
  3_600: () => m.nodes_expiry_hour(),
  86_400: () => m.nodes_expiry_day(),
  604_800: () => m.nodes_expiry_days({ count: "7" }),
  2_592_000: () => m.nodes_expiry_days({ count: "30" }),
}
