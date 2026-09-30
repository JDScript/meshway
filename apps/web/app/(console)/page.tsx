import type { NodeSummary } from "@meshway/control-client"
import { ArrowRightIcon } from "lucide-react"
import Link from "next/link"

import {
  ControllerUnavailable,
  DevelopmentNotice,
  NoNodes,
} from "@/components/controller-states"
import { Milestones } from "@/components/milestones"
import { NodesTable } from "@/components/nodes-table"
import { PageHeader } from "@/components/page-header"
import { RefreshButton } from "@/components/refresh-button"
import { buttonVariants } from "@/components/ui/button"
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import { controllerUrl, fetchNodes, fetchOverview } from "@/lib/controller"
import { byLastSeenDesc, formatClock } from "@/lib/time"

const RECENT_LIMIT = 5

function deliverySummary(nodes: NodeSummary[]) {
  const active = nodes.filter((node) => node.revoked_at === null)
  const current = active.filter(
    (node) => node.acknowledged_revision === node.configuration_revision
  ).length
  const never = active.filter((node) => node.acknowledged_revision === 0).length
  return { active: active.length, current, never, pending: active.length - current - never }
}

export default async function OverviewPage() {
  const [overview, nodes] = await Promise.all([fetchOverview(), fetchNodes()])
  const revokedCount = nodes.ok
    ? nodes.data.nodes.filter((node) => node.revoked_at !== null).length
    : null
  const delivery = nodes.ok ? deliverySummary(nodes.data.nodes) : null

  return (
    <>
      <PageHeader
        title="概览"
        description="Controller 在本次请求时返回的状态，页面不缓存节点数据。"
        actions={
          <>
            {overview.ok ? (
              <span className="text-xs text-muted-foreground">
                读取于 {formatClock(overview.fetchedAt)}
              </span>
            ) : null}
            <RefreshButton />
          </>
        }
      />

      {overview.ok ? (
        <>
          <DevelopmentNotice />

          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
            <Card className="[--card-spacing:--spacing(5)]">
              <CardHeader>
                <CardDescription className="text-sm">Controller</CardDescription>
                <CardTitle className="text-xl font-semibold">
                  {overview.data.name}
                </CardTitle>
              </CardHeader>
              <CardContent>
                <dl className="grid grid-cols-[auto_1fr] gap-x-6 gap-y-1.5 text-sm">
                  <dt className="text-muted-foreground">版本</dt>
                  <dd className="font-mono text-xs leading-5">
                    {overview.data.version}
                  </dd>
                  <dt className="text-muted-foreground">地址</dt>
                  <dd className="font-mono text-xs leading-5 break-all">
                    {controllerUrl()}
                  </dd>
                </dl>
              </CardContent>
            </Card>

            <Card className="[--card-spacing:--spacing(5)]">
              <CardHeader>
                <CardDescription className="text-sm">已注册节点</CardDescription>
                <CardTitle className="font-mono text-3xl font-semibold tabular-nums">
                  {overview.data.node_count}
                </CardTitle>
              </CardHeader>
              <CardContent className="flex flex-col gap-1 text-sm text-muted-foreground">
                {revokedCount !== null ? (
                  <span>其中已撤销 {revokedCount} 个。</span>
                ) : null}
                <span>
                  Controller 中持久保存的注册记录总数（含已撤销），不是在线或已连通的节点数。
                </span>
              </CardContent>
            </Card>

            <Card className="[--card-spacing:--spacing(5)]">
              <CardHeader>
                <CardDescription className="text-sm">配置版本</CardDescription>
                <CardTitle className="font-mono text-3xl font-semibold tabular-nums">
                  {overview.data.configuration_revision}
                </CardTitle>
                <CardAction>
                  <Link
                    href="/routing"
                    className={buttonVariants({ variant: "ghost", size: "lg" })}
                  >
                    路由策略
                    <ArrowRightIcon data-icon="inline-end" />
                  </Link>
                </CardAction>
              </CardHeader>
              <CardContent className="flex flex-col gap-1 text-sm text-muted-foreground">
                {delivery !== null ? (
                  delivery.active === 0 ? (
                    <span>没有活动节点需要下发配置。</span>
                  ) : (
                    <span>
                      {delivery.active} 个活动节点中，{delivery.current} 个已校验保存当前版本
                      {delivery.pending > 0 ? `，${delivery.pending} 个仍停留在旧版本` : ""}
                      {delivery.never > 0 ? `，${delivery.never} 个从未确认` : ""}。
                    </span>
                  )
                ) : null}
                <span>节点清单、国家或策略变化时递增。确认只表示 agent 已校验并保存，不代表网络已生效。</span>
              </CardContent>
            </Card>
          </div>

          <section className="flex flex-col gap-4">
            {!nodes.ok ? (
              <ControllerUnavailable url={controllerUrl()} error={nodes.error} />
            ) : nodes.data.nodes.length === 0 ? (
              <NoNodes />
            ) : (
              <Card className="[--card-spacing:--spacing(5)]">
                <CardHeader>
                  <CardTitle className="text-base">最近联系</CardTitle>
                  <CardDescription className="text-sm">
                    按最后联系 controller 的时间排列的前 {RECENT_LIMIT} 个节点。
                  </CardDescription>
                  <CardAction>
                    <Link
                      href="/nodes"
                      className={buttonVariants({ variant: "ghost", size: "lg" })}
                    >
                      全部节点
                      <ArrowRightIcon data-icon="inline-end" />
                    </Link>
                  </CardAction>
                </CardHeader>
                <CardContent className="px-1">
                  <NodesTable
                    nodes={[...nodes.data.nodes]
                      .sort(byLastSeenDesc)
                      .slice(0, RECENT_LIMIT)}
                    fetchedAt={nodes.fetchedAt}
                  />
                </CardContent>
              </Card>
            )}
          </section>
        </>
      ) : (
        <ControllerUnavailable url={controllerUrl()} error={overview.error} />
      )}

      <Milestones />
    </>
  )
}
