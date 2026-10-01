import type { Metadata } from "next"

import { AccessEditor } from "@/components/access/access-editor"
import { ControllerUnavailable } from "@/components/common/controller-states"
import { PageBody, PageHeader } from "@/components/shell/page-header"
import { controllerUrl, fetchConnections, fetchMeshAccess, fetchTags } from "@/lib/controller"
import { requireNetwork } from "@/lib/network-scope"
import { formatClock } from "@/lib/time"

export const metadata: Metadata = { title: "访问授权" }

export default async function AccessPage({
  params,
  searchParams,
}: {
  params: Promise<{ network: string }>
  searchParams: Promise<Record<string, string | string[] | undefined>>
}) {
  const { networkId } = await requireNetwork(params)
  // The inventory shows which nodes have published keys (rules stay valid
  // without one; the data plane waits); tags give each rule's members.
  const [access, connections, tags, query] = await Promise.all([
    fetchMeshAccess(networkId),
    fetchConnections(networkId),
    fetchTags(networkId),
    searchParams,
  ])
  const focusRuleId = typeof query.rule === "string" ? query.rule : null
  const failed = !access.ok ? access.error : !connections.ok ? connections.error : !tags.ok ? tags.error : null

  return (
    <PageBody>
      <PageHeader
        title="访问授权"
        description="哪些节点之间可以访问。规则自上而下首条命中，其余流量走默认动作。"
        meta={access.ok ? <span>读取于 {formatClock(access.fetchedAt)}</span> : null}
      />
      {failed !== null || !access.ok || !connections.ok || !tags.ok ? (
        <ControllerUnavailable url={controllerUrl()} error={failed ?? ""} />
      ) : (
        <AccessEditor snapshot={access.data} inventory={connections.data.nodes} tags={tags.data} focusRuleId={focusRuleId} />
      )}
    </PageBody>
  )
}
