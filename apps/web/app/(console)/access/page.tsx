import type { Metadata } from "next"

import { AccessEditor } from "@/components/access/access-editor"
import { ControllerUnavailable } from "@/components/common/controller-states"
import { PageBody, PageHeader } from "@/components/shell/page-header"
import { controllerUrl, fetchConnections, fetchMeshAccess } from "@/lib/controller"
import { formatClock } from "@/lib/time"

export const metadata: Metadata = { title: "访问授权" }

export default async function AccessPage() {
  // Published connection keys come from the connections inventory: a grant may
  // only name active nodes that have one.
  const [access, connections] = await Promise.all([fetchMeshAccess(), fetchConnections()])

  return (
    <PageBody width="wide">
      <PageHeader
        title="访问授权"
        description="按完整有序路径放行 Mesh IPv4 流量。默认拒绝；每条授权只放行单一方向。"
        actions={
          access.ok ? (
            <span className="text-xs text-muted-foreground tabular-nums">
              读取于 {formatClock(access.fetchedAt)}
            </span>
          ) : null
        }
      />
      {!access.ok ? (
        <ControllerUnavailable url={controllerUrl()} error={access.error} />
      ) : !connections.ok ? (
        <ControllerUnavailable url={controllerUrl()} error={connections.error} />
      ) : (
        <AccessEditor snapshot={access.data} inventory={connections.data.nodes} />
      )}
    </PageBody>
  )
}
