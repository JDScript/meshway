import type { Metadata } from "next"

import { ControllerUnavailable } from "@/components/common/controller-states"
import { PageBody, PageHeader } from "@/components/shell/page-header"
import { TagsWorkspace } from "@/components/tags/tags-workspace"
import { controllerUrl, fetchConnections, fetchMeshAccess, fetchTags } from "@/lib/controller"

export const metadata: Metadata = { title: "标签" }

export default async function TagsPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>
}) {
  const [tags, connections, access, params] = await Promise.all([
    fetchTags(),
    fetchConnections(),
    fetchMeshAccess(),
    searchParams,
  ])
  const focusNodeId = typeof params.node === "string" ? params.node : null

  return (
    <PageBody width="wide">
      <PageHeader
        title="标签"
        description="管理员维护的节点分组。访问规则以 tag:名称 引用；成员变化随全局配置版本生效。"
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
