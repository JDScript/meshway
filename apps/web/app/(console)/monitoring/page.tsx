import type { NodeSummary } from "@meshway/control-client"
import { ArrowLeftRightIcon, GaugeIcon } from "lucide-react"
import type { Metadata } from "next"
import type { ReactNode } from "react"

import { ControllerUnavailable, NoNodes } from "@/components/common/controller-states"
import { Panel } from "@/components/common/panel"
import { RefreshButton } from "@/components/common/refresh-button"
import { ContactDot, LastContact } from "@/components/nodes/node-status"
import { PageBody, PageHeader } from "@/components/shell/page-header"
import { Badge } from "@/components/ui/badge"
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty"
import { controllerUrl, fetchNodes } from "@/lib/controller"
import { activeNodes, contactBucket, contactLabels, type ContactBucket } from "@/lib/node-status"
import { byLastSeenDesc, formatClock } from "@/lib/time"

export const metadata: Metadata = { title: "监控" }

const MATRIX_LIMIT = 8
const buckets: ContactBucket[] = ["recent", "hour", "stale"]

export default async function MonitoringPage() {
  const nodes = await fetchNodes()

  return (
    <PageBody>
      <PageHeader
        title="监控"
        description="控制面联系来自真实心跳；延迟、流量与路径测量需要数据面，目前尚未上报。"
        actions={
          nodes.ok ? (
            <>
              <span className="text-xs text-muted-foreground tabular-nums">
                读取于 {formatClock(nodes.fetchedAt)}
              </span>
              <RefreshButton />
            </>
          ) : null
        }
      />
      {!nodes.ok ? (
        <ControllerUnavailable url={controllerUrl()} error={nodes.error} />
      ) : nodes.data.nodes.length === 0 ? (
        <NoNodes />
      ) : (
        <Monitoring nodes={nodes.data.nodes} fetchedAt={nodes.fetchedAt} />
      )}
    </PageBody>
  )
}

function Monitoring({
  nodes: all,
  fetchedAt,
}: {
  nodes: NodeSummary[]
  fetchedAt: number
}) {
  const nodes = activeNodes(all).sort(byLastSeenDesc)
  const matrix = nodes.slice(0, MATRIX_LIMIT)
  const counts = buckets.map((bucket) => ({
    bucket,
    count: nodes.filter((node) => contactBucket(node, fetchedAt) === bucket).length,
  }))

  return (
    <>
      <Panel
        title="控制面联系"
        description="Controller 最近一次接受注册或心跳的时间。不代表在线、隧道或连通性。"
      >
        <div className="grid gap-px border-b bg-border sm:grid-cols-3">
          {counts.map(({ bucket, count }) => (
            <div key={bucket} className="flex items-center gap-3 bg-card px-4 py-3">
              <ContactDot bucket={bucket} className="size-2" />
              <span className="text-xs text-muted-foreground">{contactLabels[bucket]}</span>
              <span className="ml-auto text-lg font-semibold tabular-nums">{count}</span>
            </div>
          ))}
        </div>
        <ul className="divide-y">
          {nodes.map((node) => (
            <li key={node.id} className="flex items-center gap-3 px-4 py-2 text-xs">
              <span className="min-w-0 flex-1 truncate text-sm font-medium">{node.name}</span>
              <span className="hidden font-mono text-muted-foreground sm:inline">{node.agent_version}</span>
              <span className="w-28 text-right">
                <LastContact node={node} fetchedAt={fetchedAt} />
              </span>
            </li>
          ))}
        </ul>
      </Panel>

      <Panel
        title={
          <span className="flex items-center gap-2">
            节点间延迟
            <Badge variant="outline">未测量</Badge>
          </span>
        }
        description={
          nodes.length > MATRIX_LIMIT
            ? `显示最近联系的 ${MATRIX_LIMIT} 个节点。数据面上报 RTT 后填充。`
            : "数据面上报 RTT 后填充；— 表示没有测量数据。"
        }
      >
        <div className="overflow-x-auto p-4">
          <table className="border-separate border-spacing-1 text-[0.6875rem]">
            <thead>
              <tr>
                <th scope="col" className="sr-only">
                  源 \ 目标
                </th>
                {matrix.map((node) => (
                  <th
                    key={node.id}
                    scope="col"
                    className="max-w-20 truncate px-1 pb-1 text-left font-medium text-muted-foreground"
                  >
                    {node.name}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {matrix.map((row) => (
                <tr key={row.id}>
                  <th
                    scope="row"
                    className="max-w-28 truncate pr-2 text-right font-medium text-muted-foreground"
                  >
                    {row.name}
                  </th>
                  {matrix.map((column) => (
                    <td
                      key={column.id}
                      className={
                        row.id === column.id
                          ? "size-11 rounded-md bg-muted/40"
                          : "size-11 rounded-md border border-dashed text-center font-mono text-muted-foreground"
                      }
                    >
                      {row.id === column.id ? (
                        <span className="sr-only">同一节点</span>
                      ) : (
                        <>
                          —<span className="sr-only">未测量</span>
                        </>
                      )}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </Panel>

      <div className="grid gap-6 md:grid-cols-2">
        <FutureMetric
          icon={<ArrowLeftRightIcon />}
          title="流量与吞吐"
          description="按节点与规则统计的流量需要数据面转发；实现前不会显示任何序列。"
        />
        <FutureMetric
          icon={<GaugeIcon />}
          title="Mesh trace"
          description="实际转发路径测量（区别于逻辑规划路径），用于验证必经节点与出口。"
        />
      </div>
    </>
  )
}

function FutureMetric({
  icon,
  title,
  description,
}: {
  icon: ReactNode
  title: string
  description: string
}) {
  return (
    <Empty className="border border-dashed">
      <EmptyHeader>
        <EmptyMedia variant="icon">{icon}</EmptyMedia>
        <EmptyTitle>{title}</EmptyTitle>
        <EmptyDescription>{description}</EmptyDescription>
      </EmptyHeader>
    </Empty>
  )
}
