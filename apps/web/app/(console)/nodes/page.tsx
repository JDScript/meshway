import { PlusIcon } from "lucide-react"
import type { Metadata } from "next"
import Link from "next/link"

import { ControllerUnavailable, NoNodes } from "@/components/common/controller-states"
import { RefreshButton } from "@/components/common/refresh-button"
import { NodesView } from "@/components/nodes/nodes-view"
import { PageBody, PageHeader } from "@/components/shell/page-header"
import { buttonVariants } from "@/components/ui/button"
import { controllerUrl, fetchNodes } from "@/lib/controller"
import { formatClock } from "@/lib/time"

export const metadata: Metadata = { title: "节点" }

export default async function NodesPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>
}) {
  const [nodes, params] = await Promise.all([fetchNodes(), searchParams])
  const query = typeof params.q === "string" ? params.q : ""

  return (
    <PageBody>
      <PageHeader
        title="节点"
        description="在 controller 注册的节点身份、Mesh 地址与配置确认。"
        actions={
          <>
            {nodes.ok ? (
              <>
                <span className="text-xs text-muted-foreground tabular-nums">
                  读取于 {formatClock(nodes.fetchedAt)}
                </span>
                <RefreshButton />
              </>
            ) : null}
            <Link href="/enrollment" className={buttonVariants({ size: "lg" })}>
              <PlusIcon data-icon="inline-start" />
              添加节点
            </Link>
          </>
        }
      />
      {!nodes.ok ? (
        <ControllerUnavailable url={controllerUrl()} error={nodes.error} />
      ) : nodes.data.nodes.length === 0 ? (
        <NoNodes />
      ) : (
        <NodesView
          key={query}
          nodes={nodes.data.nodes}
          fetchedAt={nodes.fetchedAt}
          initialQuery={query}
        />
      )}
    </PageBody>
  )
}
