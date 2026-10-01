import type { Metadata } from "next"

import { AccessEditor } from "@/components/access/access-editor"
import { ControllerUnavailable } from "@/components/common/controller-states"
import { PageBody, PageHeader } from "@/components/shell/page-header"
import { controllerUrl, fetchConnections, fetchMeshAccess, fetchTags } from "@/lib/controller"
import { formatClock } from "@/lib/time"

export const metadata: Metadata = { title: "访问授权" }

export default async function AccessPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>
}) {
  // The inventory shows which nodes have published keys (rules stay valid
  // without one; the data plane waits); tags give each rule's members.
  const [access, connections, tags, params] = await Promise.all([
    fetchMeshAccess(),
    fetchConnections(),
    fetchTags(),
    searchParams,
  ])
  const focusRuleId = typeof params.rule === "string" ? params.rule : null

  return (
    <PageBody width="wide">
      <PageHeader
        title="访问授权"
        description="按节点、标签或 * 决定端点之间能否访问。有序首条命中，默认动作可选允许或拒绝；路径由路由策略决定。"
        actions={
          access.ok ? (
            <span className="text-xs text-muted-foreground tabular-nums">读取于 {formatClock(access.fetchedAt)}</span>
          ) : null
        }
      />
      {!access.ok ? (
        <ControllerUnavailable url={controllerUrl()} error={access.error} />
      ) : !connections.ok ? (
        <ControllerUnavailable url={controllerUrl()} error={connections.error} />
      ) : !tags.ok ? (
        <ControllerUnavailable url={controllerUrl()} error={tags.error} />
      ) : (
        <AccessEditor
          snapshot={access.data}
          inventory={connections.data.nodes}
          tags={tags.data}
          focusRuleId={focusRuleId}
        />
      )}
    </PageBody>
  )
}
