import type { Metadata } from "next"

import { ControllerUnavailable } from "@/components/common/controller-states"
import { ConnectionsEditor } from "@/components/connections/connections-editor"
import { PageBody, PageHeader } from "@/components/shell/page-header"
import { controllerUrl, fetchConnections } from "@/lib/controller"
import { formatClock } from "@/lib/time"

export const metadata: Metadata = { title: "连接配置" }

export default async function ConnectionsPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>
}) {
  const [connections, params] = await Promise.all([fetchConnections(), searchParams])
  const node = typeof params.node === "string" && params.node.length <= 128 ? params.node : null

  return (
    <PageBody width="wide">
      <PageHeader
        title="连接配置"
        description="声明节点在哪里监听、由谁主动发起连接。与路由策略分开：连接只决定哪些节点可以相邻，不授权流量。"
        actions={
          connections.ok ? (
            <span className="text-xs text-muted-foreground tabular-nums">
              读取于 {formatClock(connections.fetchedAt)}
            </span>
          ) : null
        }
      />
      {connections.ok ? (
        <ConnectionsEditor snapshot={connections.data} focusNodeId={node} />
      ) : (
        <ControllerUnavailable url={controllerUrl()} error={connections.error} />
      )}
    </PageBody>
  )
}
