"use client"

import type { NodeSummary } from "@meshway/control-client"
import { ArrowUpRightIcon } from "lucide-react"
import Link from "next/link"
import { useId, useState } from "react"

import { Panel } from "@/components/common/panel"
import { NodePicker } from "@/components/routing/node-picker"
import { Badge } from "@/components/ui/badge"
import { buttonVariants } from "@/components/ui/button"
import { Label } from "@/components/ui/label"
import { topologyFlowHref } from "@/lib/flow-navigation"

/** Lightweight handoff to the one saved-flow workspace. Draft evaluation stays in its editor. */
export function FlowCheck({ nodes, baseRevision, dirty }: {
  nodes: NodeSummary[]
  baseRevision: number
  dirty: boolean
}) {
  const id = useId()
  const [sourceNodeId, setSource] = useState("")
  const [targetNodeId, setTarget] = useState("")

  return (
    <Panel
      data-testid="access-flow-check"
      title={<span className="flex flex-wrap items-center gap-2">检查已保存的授权<Badge variant="outline">编辑基础 r{baseRevision}</Badge></span>}
      description="在网络拓扑中统一查看端点授权、路由选择、逐跳邻接与执行条件。"
      bodyClassName="flex flex-col gap-4 p-4"
    >
      <div className="grid gap-3 sm:grid-cols-2">
        <div className="flex min-w-0 flex-col gap-1.5">
          <Label htmlFor={`${id}-source`}>源节点</Label>
          <NodePicker id={`${id}-source`} nodes={nodes} value={sourceNodeId} onValueChange={setSource} placeholder="可预填来源" className="w-full" />
        </div>
        <div className="flex min-w-0 flex-col gap-1.5">
          <Label htmlFor={`${id}-target`}>目标节点</Label>
          <NodePicker id={`${id}-target`} nodes={nodes} value={targetNodeId} onValueChange={setTarget} placeholder="可预填目标" className="w-full" />
        </div>
      </div>
      <Link href={topologyFlowHref({ sourceNodeId, targetNodeId })} className={buttonVariants({ variant: "outline", size: "sm" })}>
        在拓扑中检查流量<ArrowUpRightIcon data-icon="inline-end" />
      </Link>
      <p className={dirty ? "text-xs text-warning" : "text-xs text-muted-foreground"}>
        {dirty ? "未保存的草稿不参与检查；草稿会保留，返回后可继续编辑。" : "检查使用 controller 当前已保存的配置，是计划，不是 trace。"}
      </p>
    </Panel>
  )
}
