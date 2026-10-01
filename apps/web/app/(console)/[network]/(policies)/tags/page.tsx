import type { Metadata } from "next"

import { ControllerUnavailable } from "@/components/common/controller-states"
import { PageBody, PageHeader } from "@/components/shell/page-header"
import { TagsWorkspace } from "@/components/tags/tags-workspace"
import { controllerUrl, fetchConnections, fetchMeshAccess, fetchTags } from "@/lib/controller"
import { requireNetwork } from "@/lib/network-scope"

export const metadata: Metadata = { title: "标签" }

export default async function TagsPage({
  params,
  searchParams,
}: {
  params: Promise<{ network: string }>
  searchParams: Promise<Record<string, string | string[] | undefined>>
}) {
  const { networkId } = await requireNetwork(params)
  const [tags, connections, access, query] = await Promise.all([
    fetchTags(networkId),
    fetchConnections(networkId),
    fetchMeshAccess(networkId),
    searchParams,
  ])
  const focusNodeId = typeof query.node === "string" ? query.node : null

  return (
    <PageBody>
      <PageHeader
        title="标签"
        description="管理员维护的节点分组。访问授权以 tag:名称 引用；成员变化随网络配置版本生效。"
      />
      {!tags.ok ? (
        <ControllerUnavailable url={controllerUrl()} error={tags.error} />
      ) : !connections.ok ? (
        <ControllerUnavailable url={controllerUrl()} error={connections.error} />
      ) : (
        <TagsWorkspace
          key={tags.data.revision}
          snapshot={tags.data}
          inventory={connections.data.nodes}
          // Only used to explain why a referenced tag cannot be deleted.
          access={access.ok ? access.data : null}
          focusNodeId={focusNodeId && connections.data.nodes.some((entry) => entry.node.id === focusNodeId) ? focusNodeId : null}
        />
      )}
    </PageBody>
  )
}
