import { zMeshDnsConfiguration, zNodeDnsRecord } from "@meshway/control-client/schemas"

/** Mirrors crates/protocol/src/network/mod.rs::canonical_node_dns_name.
 * Only use the controller's saved, normalized zone; never normalize a draft or
 * infer aliases from display names. Validate both identity and resulting name
 * against the generated wire schemas before presenting a copyable value.
 */
export function canonicalNodeDnsName(nodeId: string, savedZone: string): string | null {
  if (!zNodeDnsRecord.shape.node_id.safeParse(nodeId).success ||
      !zMeshDnsConfiguration.shape.zone.safeParse(savedZone).success ||
      savedZone.trim() !== savedZone) return null
  const parsed = zNodeDnsRecord.shape.canonical_name.safeParse(`id.${nodeId.slice(0, 32)}.${nodeId.slice(32)}.${savedZone}`)
  return parsed.success ? parsed.data : null
}

// A path placeholder is intentional: each node has its own existing state.
export const DNS_STATUS_COMMAND = "meshway-agent dns status --state '<该节点现有 state 文件路径>'"
