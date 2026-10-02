import type { Metadata } from "next"
import Link from "next/link"
import { redirect } from "next/navigation"

import { ControllerUnavailable } from "@/components/common/controller-states"
import { ConnectionsWorkspace } from "@/components/connections/connections-workspace"
import { PageBody, PageHeader } from "@/components/shell/page-header"
import { buttonVariants } from "@/components/ui/button"
import { controllerUrl, fetchConnectionRuntimes, fetchConnections, fetchManagedListeners } from "@/lib/controller"
import { requireNetwork } from "@/lib/network-scope"
import { formatClock } from "@/lib/time"

export const metadata: Metadata = { title: "手工连接" }

export default async function ConnectionsPage({
  params,
  searchParams,
}: {
  params: Promise<{ network: string }>
  searchParams: Promise<Record<string, string | string[] | undefined>>
}) {
  const [{ networkId, routes }, query] = await Promise.all([requireNetwork(params), searchParams])
  // Per-node editing lives on the node page.
  if (typeof query.node === "string" && query.node !== "" && query.node.length <= 128) {
    redirect(routes.machine(query.node, "connections"))
  }
  const [connections, runtimes, managed] = await Promise.all([
    fetchConnections(networkId),
    fetchConnectionRuntimes(networkId),
    fetchManagedListeners(networkId),
  ])

  return (
    <PageBody>
      <PageHeader
        title="手工连接"
        description="跨节点检查监听与主动连接；日常修改在节点「配置」中完成。"
        meta={connections.ok ? <span>读取于 {formatClock(connections.fetchedAt)}</span> : null}
        actions={
          <>
            <Link href={routes.topology()} className={buttonVariants({ variant: "ghost" })}>查看已上报邻接</Link>
            <Link href={routes.machines()} className={buttonVariants({ variant: "outline" })}>按节点配置</Link>
          </>
        }
      />
      {connections.ok ? (
        <ConnectionsWorkspace
          snapshot={connections.data}
          initialRuntimes={runtimes}
          managedListenerIds={managed.ok ? managed.data.listeners.map((listener) => listener.settings.id) : null}
        />
      ) : (
        <ControllerUnavailable url={controllerUrl()} error={connections.error} />
      )}
    </PageBody>
  )
}
