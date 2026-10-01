"use client"

import type { NetworkSnapshot, NodeSummary } from "@meshway/control-client"
import Link from "next/link"

import { CopyButton } from "@/components/common/copy-button"
import { Panel } from "@/components/common/panel"
import { StatusBadge } from "@/components/common/status-badge"
import { useRoutes } from "@/components/shell/network-context"
import { canonicalNodeDnsName } from "@/lib/dns-guidance"

/**
 * Each active node's stable canonical name under the saved zone. Readable
 * aliases are decided by the controller across all nodes and are not
 * guessed here from display names.
 */
export function DnsNames({ network, nodes }: { network: NetworkSnapshot; nodes: NodeSummary[] }) {
  const routes = useRoutes()
  const active = nodes.filter((node) => node.revoked_at === null)
  return (
    <Panel
      title="节点名称"
      description={
        network.dns.enabled
          ? `zone ${network.dns.zone} · 完整名称由节点 ID 推导，改名不变。`
          : "DNS 已关闭，不发布记录；下列为开启后的完整名称。"
      }
      actions={<StatusBadge tone={network.dns.enabled ? "info" : "neutral"}>{network.dns.enabled ? "发布意图：开启" : "未发布"}</StatusBadge>}
    >
      {active.length === 0 ? (
        <p className="px-5 py-6 text-center text-xs text-muted-foreground">这张网络还没有活动节点。</p>
      ) : (
        <ul className="divide-y">
          {active.map((node) => {
            const name = canonicalNodeDnsName(node.id, network.dns.zone)
            return (
              <li key={node.id} className="grid gap-1 px-4 py-2.5 sm:grid-cols-[12rem_minmax(0,1fr)_auto] sm:items-center sm:gap-4 sm:px-5">
                <Link href={routes.machine(node.id)} className="truncate text-[0.8125rem] font-medium hover:underline">
                  {node.name}
                </Link>
                <code className="min-w-0 font-mono text-xs break-all text-muted-foreground">{name ?? "无法从已保存 zone 推导"}</code>
                {name ? <CopyButton value={name} label={`复制 ${node.name} 的完整名称`} iconOnly variant="ghost" /> : <span />}
              </li>
            )
          })}
        </ul>
      )}
    </Panel>
  )
}
