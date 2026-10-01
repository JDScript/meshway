"use client"

import type { NodeSummary, PolicySnapshot } from "@meshway/control-client"
import { ArrowRightIcon, RefreshCwIcon } from "lucide-react"
import Link from "next/link"
import { useRouter } from "next/navigation"
import { useRef, useState, useTransition } from "react"

import { Panel } from "@/components/common/panel"
import { LastContact, ReceiptBadge } from "@/components/nodes/node-status"
import { planSteps } from "@/components/routing/path-trace"
import {
  externalLabel,
  FlowForm,
  PreviewButton,
  PreviewOutcome,
} from "@/components/routing/route-preview"
import { MeshMap, type MapPlan } from "@/components/topology/mesh-map"
import { Badge } from "@/components/ui/badge"
import { Button, buttonVariants } from "@/components/ui/button"
import type { PreviewResult } from "@/lib/action-state"
import { activeNodes } from "@/lib/node-status"
import { previewPolicyAction } from "@/lib/policy-actions"
import { emptyFlow, toFlow, validateFlow, type FlowDraft } from "@/lib/policy-model"
import { cn } from "@/lib/utils"

/** A result with the exact flow it answered; the form may have moved on. */
type Plan = { result: PreviewResult; key: string; flow: FlowDraft }

/**
 * Logical map over the real inventory. Paths come only from the controller
 * compiling the *saved* policy snapshot this page was rendered with; the view
 * never draws links of its own and never labels a plan compiled for an older
 * snapshot as the live configuration.
 */
export function TopologyView({
  snapshot,
  nodes,
  fetchedAt,
  initialNodeId,
}: {
  snapshot: PolicySnapshot
  nodes: NodeSummary[]
  fetchedAt: number
  initialNodeId: string | null
}) {
  const router = useRouter()
  const active = activeNodes(nodes)
  const revokedCount = nodes.length - active.length
  const [selectedId, setSelectedId] = useState<string | null>(
    initialNodeId && active.some((node) => node.id === initialNodeId)
      ? initialNodeId
      : (active[0]?.id ?? null)
  )
  const [flow, setFlow] = useState<FlowDraft>(() => ({
    ...emptyFlow,
    sourceNodeId: initialNodeId && active.some((node) => node.id === initialNodeId) ? initialNodeId : "",
  }))
  const [plan, setPlan] = useState<Plan | null>(null)
  const [pending, startPlan] = useTransition()
  const [refreshing, startRefresh] = useTransition()
  const seq = useRef(0)

  const flowKey = JSON.stringify(toFlow(flow))
  const flowError = validateFlow(flow)
  const response = plan?.result.status === "previewed" ? plan.result.response : null
  const flowChanged = plan !== null && plan.key !== flowKey
  const revisionMoved = response !== null && response.revision !== snapshot.revision
  const stale = flowChanged || revisionMoved

  const mapPlan: MapPlan | null =
    response && response.decision.path.length > 0
      ? {
          path: response.decision.path,
          external: planSteps(response.decision, externalLabel(plan?.flow ?? flow)).some(
            (step) => step.kind === "external"
          ),
          stale,
        }
      : null

  const runPlan = () => {
    const current = ++seq.current
    const key = flowKey
    const query = toFlow(flow)
    const submittedFlow = flow
    startPlan(async () => {
      const result = await previewPolicyAction(snapshot.policy, query)
      if (current !== seq.current) return
      setPlan({ result, key, flow: submittedFlow })
    })
  }

  const selected = selectedId ? active.find((node) => node.id === selectedId) : undefined

  return (
    <div className="grid items-start gap-4 xl:grid-cols-[minmax(0,1fr)_22rem]">
      <div className="flex min-w-0 flex-col gap-4">
        <Panel
          title={
            <span className="flex flex-wrap items-center gap-2">
              逻辑拓扑
              <Badge variant="outline">已保存策略 r{snapshot.revision}</Badge>
              {!snapshot.valid ? <Badge variant="destructive">策略无效</Badge> : null}
            </span>
          }
          description="节点按名称环形排列，不代表物理位置；只有规划路径会显示连线。"
        >
          {active.length === 0 ? (
            <p className="px-4 py-16 text-center text-sm text-muted-foreground">
              没有活动节点。
              <Link href="/enrollment" className="ml-1 underline underline-offset-4">
                注册节点
              </Link>
            </p>
          ) : (
            <div className="bg-grid-dots rounded-b-xl bg-canvas">
              <MeshMap
                nodes={active}
                selectedId={selectedId}
                sourceId={flow.sourceNodeId}
                targetId={flow.targetNodeId}
                plan={mapPlan}
                onSelect={setSelectedId}
              />
            </div>
          )}
          <div className="flex flex-wrap items-center gap-x-4 gap-y-1 border-t px-4 py-2 text-[0.6875rem] text-muted-foreground">
            <Legend className="bg-brand" label="规划路径上的节点" />
            <span>链路延迟：— 未测量</span>
            {revokedCount > 0 ? <span>已撤销 {revokedCount} 个节点未显示</span> : null}
          </div>
        </Panel>

        <Panel title="节点列表" description="拓扑图的文字版本；选择节点查看详情。">
          <ul className="flex flex-wrap gap-2 p-4">
            {active.map((node) => (
              <li key={node.id}>
                <button
                  type="button"
                  aria-pressed={selectedId === node.id}
                  onClick={() => setSelectedId(node.id)}
                  className={cn(
                    "flex max-w-60 min-w-0 flex-col rounded-lg border px-3 py-1.5 text-left transition-colors hover:bg-muted/50 focus-visible:ring-2 focus-visible:ring-ring/30 focus-visible:outline-none",
                    selectedId === node.id && "border-brand/50 bg-brand/5",
                    (flow.sourceNodeId === node.id || flow.targetNodeId === node.id) && "border-brand"
                  )}
                >
                  <span className="truncate text-xs font-medium">
                    {node.name}
                    {flow.sourceNodeId === node.id ? " · 来源" : ""}
                    {flow.targetNodeId === node.id ? " · 目标" : ""}
                  </span>
                  <span className="truncate font-mono text-[0.6875rem] text-muted-foreground">
                    {node.mesh_ipv4}
                    {node.country ? ` · ${node.country}` : ""}
                  </span>
                  <span className="text-[0.6875rem] text-muted-foreground">
                    控制面联系 <LastContact node={node} fetchedAt={fetchedAt} />
                  </span>
                </button>
              </li>
            ))}
          </ul>
        </Panel>
      </div>

      <div className="flex min-w-0 flex-col gap-4 xl:sticky xl:top-16">
        <Panel title="规划路径" description="按已保存策略编译一条假设流量；不是实际 trace。" bodyClassName="flex flex-col gap-4 p-4">
          <FlowForm nodes={nodes} flow={flow} onFlowChange={setFlow} disabled={pending} />
          <PreviewButton
            onClick={runPlan}
            disabled={pending || active.length === 0}
            pending={pending}
            blockedReason={flowError}
            label="规划路径"
          />
          {revisionMoved && response ? (
            <div className="flex flex-col items-start gap-2 rounded-lg border border-warning/40 bg-warning/5 px-3 py-2 text-xs">
              <span>
                Controller 配置已变为 r{response.revision}，此规划基于页面读取的 r{snapshot.revision} 策略，可能不是当前生效配置。
              </span>
              <Button
                variant="outline"
                size="sm"
                disabled={refreshing}
                onClick={() => startRefresh(() => router.refresh())}
              >
                <RefreshCwIcon data-icon="inline-start" className={refreshing ? "animate-spin" : undefined} />
                读取最新策略
              </Button>
            </div>
          ) : null}
          <div className="border-t pt-4">
            <PreviewOutcome
              result={plan?.result ?? null}
              nodes={nodes}
              flow={plan?.flow ?? flow}
              stale={stale}
              pending={pending}
              staleMessage={flowChanged ? "流量已修改，以下是上一次的规划。" : "基于较旧的策略版本。"}
              emptyMessage="选择源节点与目标，查看已保存策略为这条流量规划的逻辑路径。"
            />
          </div>
        </Panel>

        {selected ? (
          <Panel
            title={selected.name}
            description={<span className="block truncate font-mono" title={selected.id}>{selected.id}</span>}
            bodyClassName="flex flex-col gap-3 p-4"
          >
            <dl className="grid grid-cols-[5.5rem_1fr] gap-x-3 gap-y-2 text-xs">
              <dt className="text-muted-foreground">Mesh IPv4</dt>
              <dd className="font-mono">{selected.mesh_ipv4}</dd>
              <dt className="text-muted-foreground">Mesh IPv6</dt>
              <dd className="font-mono break-all">{selected.mesh_ipv6}</dd>
              <dt className="text-muted-foreground">国家</dt>
              <dd className="font-mono">{selected.country ?? "未设置"}</dd>
              <dt className="text-muted-foreground">控制面联系</dt>
              <dd>
                <LastContact node={selected} fetchedAt={fetchedAt} />
              </dd>
              <dt className="text-muted-foreground">配置确认</dt>
              <dd>
                <ReceiptBadge node={selected} />
              </dd>
              <dt className="text-muted-foreground">RTT</dt>
              <dd className="text-muted-foreground">— 未测量</dd>
            </dl>
            <div className="flex flex-wrap gap-2">
              <Button
                variant="outline"
                size="sm"
                onClick={() => setFlow({ ...flow, sourceNodeId: selected.id })}
                disabled={flow.sourceNodeId === selected.id}
              >
                设为来源
              </Button>
              <Button
                variant="outline"
                size="sm"
                onClick={() =>
                  setFlow({ ...flow, targetNodeId: selected.id, domain: "", ip: "", country: "" })
                }
                disabled={flow.targetNodeId === selected.id}
              >
                设为目标
              </Button>
              <Link
                href={`/nodes/${encodeURIComponent(selected.id)}`}
                className={buttonVariants({ variant: "ghost", size: "sm" })}
              >
                节点详情
                <ArrowRightIcon data-icon="inline-end" />
              </Link>
            </div>
          </Panel>
        ) : null}
      </div>
    </div>
  )
}

function Legend({ className, label }: { className: string; label: string }) {
  return (
    <span className="flex items-center gap-1.5">
      <span aria-hidden="true" className={cn("size-2 rounded-full", className)} />
      {label}
    </span>
  )
}
