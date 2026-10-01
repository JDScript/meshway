import type { NetworkSnapshot, NodeSummary } from "@meshway/control-client"
import Link from "next/link"

import { CopyButton } from "@/components/common/copy-button"
import { Panel } from "@/components/common/panel"
import { DnsCommand, DnsDocsLinks } from "@/components/network/dns-guidance"
import { Badge } from "@/components/ui/badge"
import { buttonVariants } from "@/components/ui/button"
import { canonicalNodeDnsName } from "@/lib/dns-guidance"

export type NodeDnsNetwork = { ok: true; data: NetworkSnapshot } | { ok: false; error: string }

/** Identity-derived guidance, never an observation of the local agent. */
export function NodeDnsPanel({ node, network }: { node: NodeSummary; network: NodeDnsNetwork }) {
  const name = network.ok ? canonicalNodeDnsName(node.id, network.data.dns.zone) : null
  const publishedIntent = network.ok && network.data.dns.enabled && node.revoked_at === null
  const dig = name ? `dig @127.0.0.1 -p 5353 ${name}. A` : null

  return (
    <Panel title="节点 DNS" actions={<Link href="/network#dns-diagnostics" className={buttonVariants({ variant: "ghost", size: "sm" })}>配置与诊断</Link>} bodyClassName="flex min-w-0 flex-col gap-3 px-4 py-3 text-xs">
      {!network.ok ? <p role="status" className="text-destructive">网络设置读取失败，无法推导 DNS 名称：{network.error}</p> : (
        <>
          <div className="flex flex-wrap items-center gap-2">
            <Badge variant="outline">已保存意图：{network.data.dns.enabled ? "启用" : "关闭"}</Badge>
            <span className="text-muted-foreground">r{network.data.revision}</span>
          </div>
          {!network.data.dns.enabled ? <p data-testid="node-dns-disabled" className="text-muted-foreground">DNS 已关闭，不发布节点记录；以下仅是身份推导名称。</p> : node.revoked_at !== null ? <p className="text-muted-foreground">节点已撤销，不发布其 DNS 记录；以下仅是身份推导名称。</p> : null}
          {name ? (
            <div className="flex min-w-0 flex-col gap-1.5">
              <span className="text-muted-foreground">完整 ID canonical FQDN</span>
              <div className="flex min-w-0 items-start gap-2">
                <code data-testid="node-dns-canonical" className="min-w-0 flex-1 font-mono break-all">{name}</code>
                <CopyButton value={name} label="复制完整 DNS FQDN" iconOnly variant="ghost" />
              </div>
            </div>
          ) : <p className="text-destructive">节点 ID 或已保存 zone 不满足 DNS 名称格式，未生成查询命令。</p>}
          <p className="text-muted-foreground">可读 alias 需 controller 对全部活动节点做 IDNA 规范化与唯一性检查；本页只显示完整身份名，不推断 alias。</p>
          {publishedIntent && dig ? <DnsCommand value={dig} label="复制节点 DNS 查询命令" testId="node-dns-query-command" /> : null}
        </>
      )}
      <p className="text-muted-foreground">本地 DNS listener 与系统 resolver 状态未知。查询示例需 agent 显式配置 <code className="font-mono">--dns-listen 127.0.0.1:5353</code>，在其所在环境执行；DNS 答复不证明隧道连通或访问授权。</p>
      <DnsDocsLinks />
    </Panel>
  )
}
