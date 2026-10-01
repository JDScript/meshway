import { zNodeDnsRecord } from "@meshway/control-client/schemas"

import type { ConsoleRoutes } from "./routes.ts"

export const NODE_STATE_ARGUMENT = "--state '<源节点现有 state 文件路径>'"
export const LIVE_STATUS_COMMAND = `meshway-agent status ${NODE_STATE_ARGUMENT} --live --peers --json`
export const NETCHECK_COMMAND = "meshway-agent netcheck --stun-server '<NUMERIC_IPV4_A>:3478' --stun-server '<DIFFERENT_NUMERIC_IPV4_B>:3478' --bind 0.0.0.0:0 --json"

/** Use full validated identity, never interpolate an inventory display name. */
export function peerDiagnosticCommands(sourceId: string | undefined, targetId: string | undefined) {
  if (!sourceId || !targetId || sourceId === targetId ||
      !zNodeDnsRecord.shape.node_id.safeParse(sourceId).success ||
      !zNodeDnsRecord.shape.node_id.safeParse(targetId).success) return null
  return {
    whois: `meshway-agent whois ${targetId} ${NODE_STATE_ARGUMENT} --json`,
    ping: `meshway-agent ping ${targetId} ${NODE_STATE_ARGUMENT} --count 3 --json`,
  }
}

export function diagnosticsHref(routes: Pick<ConsoleRoutes, "diagnostics">, sourceId: string): string {
  return routes.diagnostics({ source: sourceId })
}
