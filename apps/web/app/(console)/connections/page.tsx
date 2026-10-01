import type { Metadata } from "next"
import { redirect } from "next/navigation"

import { ControllerUnavailable } from "@/components/common/controller-states"
import { ConnectionsWorkspace } from "@/components/connections/connections-workspace"
import { PageBody, PageHeader } from "@/components/shell/page-header"
import { controllerUrl, fetchConnectionRuntimes, fetchConnections } from "@/lib/controller"
import { formatClock } from "@/lib/time"

export const metadata: Metadata = { title: "连接配置" }

export default async function ConnectionsPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>
}) {
  const params = await searchParams
  // Per-node editing lives on the node page now.
  if (typeof params.node === "string" && params.node !== "" && params.node.length <= 128) {
    redirect(`/nodes/${encodeURIComponent(params.node)}?tab=connections`)
  }
  const [connections, runtimes] = await Promise.all([fetchConnections(), fetchConnectionRuntimes()])

  return (
    <PageBody width="wide">
      <PageHeader
        title="连接配置"
        description="哪些节点接收连接、谁主动连接谁。连接只决定节点之间能否建立认证通道，不授权流量。"
        actions={
          connections.ok ? (
            <span className="text-xs text-muted-foreground tabular-nums">
              读取于 {formatClock(connections.fetchedAt)}
            </span>
          ) : null
        }
      />
      {connections.ok ? (
        <ConnectionsWorkspace
          snapshot={connections.data}
          initialRuntimes={runtimes}
        />
      ) : (
        <ControllerUnavailable url={controllerUrl()} error={connections.error} />
      )}
    </PageBody>
  )
}
