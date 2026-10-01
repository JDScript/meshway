import { InfoIcon } from "lucide-react"
import type { Metadata } from "next"
import Link from "next/link"

import { ControllerUnavailable } from "@/components/common/controller-states"
import { PolicyEditor } from "@/components/routing/policy-editor"
import { PageBody, PageHeader } from "@/components/shell/page-header"
import { Alert, AlertDescription } from "@/components/ui/alert"
import { controllerUrl, fetchNodes, fetchPolicy } from "@/lib/controller"
import { formatClock } from "@/lib/time"

export const metadata: Metadata = { title: "路由策略" }

export default async function RoutingPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>
}) {
  const [policy, nodes, params] = await Promise.all([fetchPolicy(), fetchNodes(), searchParams])
  // Deep link from a plan or trace explanation: highlight that rule.
  const focusRuleId = typeof params.rule === "string" ? params.rule : null

  return (
    <PageBody width="wide">
      <PageHeader
        title="路由策略"
        description="按来源、目标域名后缀 / CIDR / 节点 / 国家匹配流量，规划有序经由节点与互联网出口。"
        actions={
          policy.ok ? (
            <span className="text-xs text-muted-foreground tabular-nums">
              读取于 {formatClock(policy.fetchedAt)}
            </span>
          ) : null
        }
      />
      <Alert data-testid="routing-execution-scope">
        <InfoIcon />
        <AlertDescription className="flex flex-col gap-1">
          <p>
            以 <code className="font-mono">--apply-dataplane</code> 运行的 Linux agent 会执行这里的 Mesh IPv4
            意图：来源节点 / 国家、目标节点 / CIDR、首条命中、经由节点、直连 / 拒绝；编译出的路径还必须有完全一致的
            <Link href="/access" className="mx-0.5 underline underline-offset-4">
              访问授权
            </Link>
            。
          </p>
          <p>
            域名后缀需要 DNS，互联网出口与 IPv6 尚未启用；报文可能命中域名规则时按拒绝处理，不会跳过该规则。
          </p>
        </AlertDescription>
      </Alert>
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
