import type { Metadata } from "next"
import Link from "next/link"

import { ControllerUnavailable } from "@/components/common/controller-states"
import { DiagnosticDetails } from "@/components/common/diagnostic-details"
import { PolicyEditor } from "@/components/routing/policy-editor"
import { PageBody, PageHeader } from "@/components/shell/page-header"
import { controllerUrl, fetchNodes, fetchPolicy } from "@/lib/controller"
import { requireNetwork } from "@/lib/network-scope"
import { formatClock } from "@/lib/time"

export const metadata: Metadata = { title: "路由" }

export default async function RoutingPage({
  params,
  searchParams,
}: {
  params: Promise<{ network: string }>
  searchParams: Promise<Record<string, string | string[] | undefined>>
}) {
  const { networkId, routes } = await requireNetwork(params)
  const [policy, nodes, query] = await Promise.all([fetchPolicy(networkId), fetchNodes(networkId), searchParams])
  // Deep link from a plan or trace explanation: highlight that rule.
  const focusRuleId = typeof query.rule === "string" ? query.rule : null

  return (
    <PageBody>
      <PageHeader
        title="路由"
        description="流量走哪条路径、经由哪些节点。规则自上而下首条命中。"
        meta={policy.ok ? <span>读取于 {formatClock(policy.fetchedAt)}</span> : null}
      />
      <DiagnosticDetails title="执行范围：Mesh IPv4，互联网出口与 IPv6 尚未启用" className="bg-card shadow-xs">
        <div data-testid="routing-execution-scope" className="flex flex-col gap-2 leading-relaxed text-muted-foreground">
          <p>
            以 <code className="font-mono text-foreground">--apply-dataplane</code> 运行的 Linux agent 执行这里的 Mesh IPv4
            意图：来源节点 / 国家、目标节点 / CIDR、首条命中、经由节点、直连 / 拒绝。端点之间能否访问由
            <Link href={routes.access()} className="mx-0.5 text-foreground underline underline-offset-4">访问授权</Link>
            单独决定；允许不会绕过这里的拒绝或经由节点。
          </p>
          <p>域名后缀需要 DNS；报文可能命中域名规则时按拒绝处理，不会跳过该规则。</p>
        </div>
      </DiagnosticDetails>
      {!policy.ok ? (
        <ControllerUnavailable url={controllerUrl()} error={policy.error} />
      ) : !nodes.ok ? (
        <ControllerUnavailable url={controllerUrl()} error={nodes.error} />
      ) : (
        <PolicyEditor snapshot={policy.data} nodes={nodes.data.nodes} focusRuleId={focusRuleId} />
      )}
    </PageBody>
  )
}
