import { PlusIcon } from "lucide-react"
import type { Metadata } from "next"
import Link from "next/link"

import {
  ControllerUnavailable,
  DevelopmentNotice,
  NoNodes,
} from "@/components/controller-states"
import { NodesTable } from "@/components/nodes-table"
import { PageHeader } from "@/components/page-header"
import { RefreshButton } from "@/components/refresh-button"
import { buttonVariants } from "@/components/ui/button"
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import { controllerUrl, fetchNodes } from "@/lib/controller"
import { byLastSeenDesc, formatClock } from "@/lib/time"

export const metadata: Metadata = { title: "节点" }

export default async function NodesPage() {
  const nodes = await fetchNodes()
  const revokedCount = nodes.ok
    ? nodes.data.nodes.filter((node) => node.revoked_at !== null).length
    : 0

  return (
    <>
      <PageHeader
        title="节点"
        description="Controller 持久保存的全部注册记录，按最后联系时间倒序排列。"
        actions={
          <>
            {nodes.ok ? (
              <span className="text-xs text-muted-foreground">
                读取于 {formatClock(nodes.fetchedAt)}
              </span>
            ) : null}
            <Link
              href="/enrollment"
              className={buttonVariants({ variant: "outline", size: "lg" })}
            >
              <PlusIcon data-icon="inline-start" />
              注册令牌
            </Link>
            <RefreshButton />
          </>
        }
      />

      {!nodes.ok ? (
        <ControllerUnavailable url={controllerUrl()} error={nodes.error} />
      ) : (
        <>
          <DevelopmentNotice />
          {nodes.data.nodes.length === 0 ? (
            <NoNodes />
          ) : (
            <Card className="[--card-spacing:--spacing(5)]">
              <CardHeader>
                <CardTitle className="text-base">
                  已注册节点
                  <span className="ml-2 font-mono text-sm font-normal text-muted-foreground tabular-nums">
                    {nodes.data.nodes.length}
                  </span>
                  {revokedCount > 0 ? (
                    <span className="ml-2 text-sm font-normal text-muted-foreground">
                      其中已撤销 {revokedCount}
                    </span>
                  ) : null}
                </CardTitle>
                <CardDescription className="text-sm">
                  「最后联系」是 controller 最近一次收到该节点注册或心跳请求的时间，相对时间以本次读取时刻计算。它不反映隧道状态或节点之间的连通性。
                </CardDescription>
              </CardHeader>
              <CardContent className="px-1">
                <NodesTable
                  nodes={[...nodes.data.nodes].sort(byLastSeenDesc)}
                  fetchedAt={nodes.fetchedAt}
                  manage
                />
              </CardContent>
            </Card>
          )}
        </>
      )}
    </>
  )
}
