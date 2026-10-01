import type { Metadata } from "next"
import Link from "next/link"
import { redirect } from "next/navigation"

import { ControllerUnavailable } from "@/components/common/controller-states"
import { ConnectionsWorkspace } from "@/components/connections/connections-workspace"
import { PageBody, PageHeader } from "@/components/shell/page-header"
import { buttonVariants } from "@/components/ui/button"
import { controllerUrl, fetchConnectionRuntimes, fetchConnections } from "@/lib/controller"
import { formatClock } from "@/lib/time"

export const metadata: Metadata = { title: "高级连接" }

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
        title="高级连接"
        description="全网络连接配置与审计。日常监听、主动连接请从节点详情配置；这里适合跨节点检查与批量调整。认证通道不等于访问授权。"
        actions={
          connections.ok ? (
            <span className="text-xs text-muted-foreground tabular-nums">
              读取于 {formatClock(connections.fetchedAt)}
            </span>
          ) : null
        }
      />
      <div className="flex flex-wrap items-center gap-3 text-xs text-muted-foreground">
        <Link href="/nodes" className={buttonVariants({ variant: "outline", size: "sm" })}>选择节点配置连接</Link>
        <Link href="/topology" className="underline underline-offset-4">查看真实邻接与检查流量</Link>
      </div>
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
