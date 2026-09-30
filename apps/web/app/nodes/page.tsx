import type { Metadata } from "next"

import {
  ControllerUnavailable,
  DevelopmentNotice,
  NoNodes,
} from "@/components/controller-states"
import { NodesTable } from "@/components/nodes-table"
import { PageHeader } from "@/components/page-header"
import { RefreshButton } from "@/components/refresh-button"
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

  return (
    <>
      <PageHeader
        title="节点"
        description="Controller 记录的全部节点上报，按最后上报时间倒序排列。"
        actions={
          <>
            {nodes.ok ? (
              <span className="text-xs text-muted-foreground">
                读取于 {formatClock(nodes.fetchedAt)}
              </span>
            ) : null}
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
            <NoNodes url={controllerUrl()} />
          ) : (
            <Card className="[--card-spacing:--spacing(5)]">
              <CardHeader>
                <CardTitle className="text-base">
                  已记录节点
                  <span className="ml-2 font-mono text-sm font-normal text-muted-foreground tabular-nums">
                    {nodes.data.nodes.length}
                  </span>
                </CardTitle>
                <CardDescription className="text-sm">
                  「最后上报」是 controller 最近一次收到该节点 report
                  请求的时间，相对时间以本次读取时刻计算。它不反映隧道状态或节点之间的连通性。
                </CardDescription>
              </CardHeader>
              <CardContent className="px-1">
                <NodesTable
                  nodes={[...nodes.data.nodes].sort(byLastSeenDesc)}
                  fetchedAt={nodes.fetchedAt}
                />
              </CardContent>
            </Card>
          )}
        </>
      )}
    </>
  )
}
