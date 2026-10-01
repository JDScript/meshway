"use client"

import { useRoutes } from "@/components/shell/network-context"
import type { EndpointDiscoveriesSnapshot, NetworkSnapshot, NodeList, NodeSummary } from "@meshway/control-client"
import Link from "next/link"
import { useSearchParams } from "next/navigation"
import type { ReactNode } from "react"

import { DnsCommand } from "@/components/network/dns-guidance"
import { DiagnosticDetails } from "@/components/common/diagnostic-details"
import { FailureNotice } from "@/components/common/failure-notice"
import { useLocationHash } from "@/components/common/use-location-hash"
import { EndpointDiscoveryPanel } from "@/components/connections/endpoint-discovery-panel"
import { Badge } from "@/components/ui/badge"
import { buttonVariants } from "@/components/ui/button"
import { LIVE_STATUS_COMMAND, NETCHECK_COMMAND, NODE_STATE_ARGUMENT, peerDiagnosticCommands } from "@/lib/connectivity-guidance"
import { docsUrl } from "@/lib/config"
import type { ControllerResult } from "@/lib/controller"
import { canonicalNodeDnsName } from "@/lib/dns-guidance"
import { topologyFlowHref } from "@/lib/flow-navigation"
import { shellQuote } from "@/lib/shell"

export function ConnectivityWorkbench({ nodes, network, discoveries }: {
  nodes: ControllerResult<NodeList>
  network: ControllerResult<NetworkSnapshot>
  discoveries: ControllerResult<EndpointDiscoveriesSnapshot>
}) {
  const params = useSearchParams()
  const routes = useRoutes()
  const section = useLocationHash()
  const sourceId = params.get("source") ?? params.get("node") ?? ""
  const targetId = params.get("target") ?? ""
  const inventory = nodes.ok ? nodes.data.nodes.filter((node) => node.revoked_at === null) : []
  const source = inventory.find((node) => node.id === sourceId)
  const target = inventory.find((node) => node.id === targetId)
  const commands = peerDiagnosticCommands(source?.id, target?.id)
  const name = target && network.ok && network.data.dns.enabled ? canonicalNodeDnsName(target.id, network.data.dns.zone) : null

  const selectNode = (key: "source" | "target", id: string) => {
    const url = new URL(window.location.href)
    url.searchParams.delete("node")
    if (sourceId) url.searchParams.set("source", sourceId)
    if (id) url.searchParams.set(key, id)
    else url.searchParams.delete(key)
    window.history.replaceState(null, "", url)
  }

  return (
    <section aria-label="连通性诊断工作台" className="flex min-w-0 flex-col gap-4">
      <div className="grid gap-3 sm:grid-cols-2">
        <NodeSelect label="源节点（执行命令的环境）" id="diagnostic-source" value={sourceId} nodes={inventory} onChange={(id) => selectNode("source", id)} />
        <NodeSelect label="目标节点（独立于源节点）" id="diagnostic-target" value={targetId} nodes={inventory} exclude={source?.id} onChange={(id) => selectNode("target", id)} />
      </div>
      {!nodes.ok ? <FailureNotice title="节点清单读取失败" nextStep="刷新页面重试；恢复前可使用独立 netcheck 模板。" error={nodes.error} /> : inventory.length === 0 ? <p className="text-xs text-muted-foreground">没有可用的活动节点；netcheck 仍可在待测环境独立执行。</p> : null}
      {(sourceId && !source) || (targetId && !target) ? <p role="status" className="text-xs text-warning">链接中的节点已不可用，未替换成其他节点。请选择活动节点。</p> : null}
      {source && target && source.id === target.id ? <p role="status" className="text-xs text-warning">源与目标相同，请另选目标；不会生成自 ping 命令。</p> : null}
      <p className="text-xs text-muted-foreground">命令在源节点所在环境执行；live / DNS 模板复用其现有 state 路径。本地 CLI 输出未上报到 Web。</p>

      <div className="flex min-w-0 flex-col gap-3">
        <CheckSection id="saved-flow" title="已保存 flow" source="配置预览" open={!section || section === "saved-flow"}>
          <p>检查已保存的访问授权与路由选择。结果是规划路径，不是实际 trace。</p>
          <Link href={topologyFlowHref(routes, { sourceNodeId: source?.id, targetNodeId: source && target && source.id !== target.id ? target.id : undefined })} className={buttonVariants({ variant: "outline", size: "sm" })}>在拓扑中检查已保存 flow</Link>
        </CheckSection>

        <CheckSection id="live-adjacency" title="邻接与身份" source="Agent 运行观测" open={section === "live-adjacency"}>
          <div className="flex flex-wrap gap-2">
            {source ? <Link href={routes.machine(source.id, "connections")} className={buttonVariants({ variant: "outline", size: "sm" })}>查看源节点连接运行状态</Link> : <Link href={routes.machines()} className={buttonVariants({ variant: "outline", size: "sm" })}>选择节点查看运行状态</Link>}
            <Link href={source ? routes.topology({ node: source.id }) : routes.topology()} className={buttonVariants({ variant: "ghost", size: "sm" })}>查看已上报邻接</Link>
          </div>
          <p>status --live 读取正在运行的 managed daemon；whois 解析它的可信清单。ping 只探测已认证直连邻居的 RTT，不是 ICMP、端到端业务延迟或访问授权。</p>
          <Command value={LIVE_STATUS_COMMAND} kind="live-status" label="复制 live status 命令" />
          {commands ? <>
            <Command value={commands.whois} kind="whois" label="复制目标 whois 命令" />
            <Command value={commands.ping} kind="ping" label="复制目标 ping 命令" />
          </> : <p>选择源节点与不同的目标节点后生成 whois / ping 命令；使用完整节点 ID，不使用显示名称。</p>}
        </CheckSection>

        <CheckSection id="dns" title="DNS" source="本地执行 · 未上报" open={section === "dns"}>
          <p>status 读取源节点本地 DNS listener 元数据；query 经正在运行的 daemon 查询它实际的 loopback listener，需先显式启用 --dns-listen。解析成功不证明隧道连通。</p>
          <Command value={`meshway-agent dns status ${NODE_STATE_ARGUMENT} --json`} kind="dns-status" label="复制诊断 DNS status 命令" />
          {!network.ok ? <p role="status" className="text-destructive">网络设置读取失败，无法生成查询：{network.error}</p> : <p>已保存 DNS：{network.data.dns.enabled ? "启用" : "关闭"} · r{network.data.revision}；本地 listener 与系统 resolver 状态未知。</p>}
          {source && commands && name ? <Command value={`meshway-agent dns query ${shellQuote(name)} ${NODE_STATE_ARGUMENT} --type A --json`} kind="dns-query" label="复制目标 DNS 查询命令" /> : <p>已保存 DNS 启用且选定不同的活动目标后，提供查询命令。</p>}
          <Link href={routes.dns("dns-diagnostics")} className={buttonVariants({ variant: "outline", size: "sm" })}>DNS 配置与诊断</Link>
        </CheckSection>

        <CheckSection id="netcheck" title="底层网络 · netcheck" source="carrier 自报 / 独立探测" open={section === "netcheck"}>
          <EndpointDiscoveryPanel initial={discoveries} nodeId={source?.id} />
          <p className="font-medium text-foreground">独立探测 · 未上报</p>
          <p>在想测的节点网络 namespace 中执行：当前进程用独立临时 UDP socket 观测底层网络，每次执行后关闭；无需 state 或 daemon，不改路由、不注册候选。</p>
          <Command value={NETCHECK_COMMAND} kind="netcheck" label="复制 netcheck 命令模板" />
          <p>替换为两个你指定的不同 numeric IPv4 地址；同 IP 的不同端口不能比较映射随目的 IP 的变化。来源为 independent probe，不代表节点 listener endpoint、打洞、peer 身份或已安装候选。</p>
          <a href={`${docsUrl}/netcheck`} target="_blank" rel="noreferrer" className="w-fit text-foreground underline underline-offset-4">netcheck 使用说明</a>
        </CheckSection>
      </div>
      <p className="text-xs text-muted-foreground">未测量：Mesh 流量、吞吐、端到端路径延迟与真实 trace。目前没有这些指标的上报数据。</p>
    </section>
  )
}

function NodeSelect({ label, id, value, nodes, exclude, onChange }: {
  label: string; id: string; value: string; nodes: NodeSummary[]; exclude?: string; onChange: (id: string) => void
}) {
  return (
    <div className="flex min-w-0 flex-col gap-1.5">
      <label htmlFor={id} className="text-xs font-medium">{label}</label>
      <select id={id} value={value} onChange={(event) => onChange(event.target.value)} className="h-8 w-full min-w-0 rounded-md border border-input bg-card px-2 text-[0.8125rem]">
        <option value="">请选择节点</option>
        {value && !nodes.some((node) => node.id === value) ? <option value={value}>不可用节点（来自链接）</option> : null}
        {nodes.map((node) => <option key={node.id} value={node.id} disabled={node.id === exclude}>{node.name} · {node.id.slice(0, 8)}</option>)}
      </select>
    </div>
  )
}

function CheckSection({ id, title, source, open, children }: { id: string; title: string; source: string; open?: boolean; children: ReactNode }) {
  return (
    <DiagnosticDetails id={id} open={open} title={title} className="scroll-mt-20">
      <Badge variant="outline" className="w-fit font-normal text-muted-foreground">{source}</Badge>
      <div className="flex min-w-0 flex-col items-start gap-3 text-xs text-muted-foreground [&>div]:w-full [&>p]:leading-relaxed">{children}</div>
    </DiagnosticDetails>
  )
}

function Command({ value, kind, label }: { value: string; kind: string; label: string }) {
  return <DnsCommand value={value} label={label} testId={`diagnostic-${kind}-command`} />
}
