import type { NodeSummary } from "@meshway/control-client"

import { NodeCountryButton } from "@/components/node-country-button"
import { RevokeNodeButton } from "@/components/revoke-node-button"
import { Badge } from "@/components/ui/badge"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { cn } from "@/lib/utils"
import { formatDateTime, formatRelative } from "@/lib/time"

function Timestamp({ value, fetchedAt }: { value: string; fetchedAt: number }) {
  return (
    <div className="flex flex-col">
      <span>{formatRelative(value, fetchedAt) ?? "—"}</span>
      <time
        dateTime={value}
        title={value}
        className="font-mono text-[0.6875rem] whitespace-nowrap text-muted-foreground"
      >
        {formatDateTime(value)}
      </time>
    </div>
  )
}

/**
 * Configuration receipt. An acknowledgement means the agent validated and
 * durably saved that revision; it does not mean networking was applied.
 */
export function ConfigurationStatus({ node }: { node: NodeSummary }) {
  if (node.revoked_at !== null) {
    return <span className="text-xs text-muted-foreground">已撤销，不再下发</span>
  }
  const desired = node.configuration_revision
  const acknowledged = node.acknowledged_revision
  const title = "「已校验保存」表示 agent 已校验并持久保存该版本配置，不代表网络已生效。"
  if (acknowledged === desired) {
    return (
      <div className="flex flex-col items-start gap-1" title={title}>
        <Badge variant="outline">已校验保存</Badge>
        <span className="font-mono text-[0.6875rem] whitespace-nowrap text-muted-foreground">
          版本 {acknowledged}
        </span>
      </div>
    )
  }
  return (
    <div className="flex flex-col items-start gap-1" title={title}>
      <Badge variant="secondary">未确认</Badge>
      <span className="font-mono text-[0.6875rem] whitespace-nowrap text-muted-foreground">
        {acknowledged === 0 ? "从未确认" : `已确认 ${acknowledged}`} · 目标 {desired}
      </span>
    </div>
  )
}

export function NodesTable({
  nodes,
  fetchedAt,
  manage = false,
}: {
  nodes: NodeSummary[]
  fetchedAt: number
  manage?: boolean
}) {
  return (
    <Table className="text-sm">
      <TableHeader>
        <TableRow className="hover:bg-transparent">
          <TableHead className="px-4">名称</TableHead>
          <TableHead className="px-4">状态</TableHead>
          <TableHead className="px-4">Mesh 地址</TableHead>
          <TableHead className="px-4">国家</TableHead>
          <TableHead className="px-4">配置</TableHead>
          <TableHead className="px-4">最后联系</TableHead>
          <TableHead className="px-4">注册于</TableHead>
          <TableHead className="px-4">Agent 版本</TableHead>
          <TableHead className="px-4">节点 ID</TableHead>
          {manage ? (
            <TableHead className="px-4 text-right">
              <span className="sr-only">操作</span>
            </TableHead>
          ) : null}
        </TableRow>
      </TableHeader>
      <TableBody>
        {nodes.map((node) => {
          const revoked = node.revoked_at !== null
          return (
            <TableRow key={node.id} className={cn(revoked && "text-muted-foreground")}>
              <TableCell className="px-4 py-3 font-medium">{node.name}</TableCell>
              <TableCell className="px-4 py-3">
                {node.revoked_at !== null ? (
                  <div className="flex flex-col items-start gap-1">
                    <Badge variant="destructive">已撤销</Badge>
                    <time
                      dateTime={node.revoked_at}
                      title={node.revoked_at}
                      className="font-mono text-[0.6875rem] whitespace-nowrap"
                    >
                      {formatDateTime(node.revoked_at)}
                    </time>
                  </div>
                ) : (
                  <Badge variant="outline">已注册</Badge>
                )}
              </TableCell>
              <TableCell className="px-4 py-3">
                <div className="flex flex-col font-mono text-xs whitespace-nowrap">
                  <span>{node.mesh_ipv4}</span>
                  <span className="text-[0.6875rem] text-muted-foreground">
                    {node.mesh_ipv6}
                  </span>
                </div>
              </TableCell>
              <TableCell className="px-4 py-3 font-mono text-xs">
                {manage && !revoked ? (
                  <NodeCountryButton
                    nodeId={node.id}
                    nodeName={node.name}
                    country={node.country}
                    expectedRevision={node.configuration_revision}
                  />
                ) : (
                  (node.country ?? "—")
                )}
              </TableCell>
              <TableCell className="px-4 py-3">
                <ConfigurationStatus node={node} />
              </TableCell>
              <TableCell className="px-4 py-3">
                <Timestamp value={node.last_seen} fetchedAt={fetchedAt} />
              </TableCell>
              <TableCell className="px-4 py-3">
                <Timestamp value={node.registered_at} fetchedAt={fetchedAt} />
              </TableCell>
              <TableCell className="px-4 py-3 font-mono text-xs">
                {node.agent_version}
              </TableCell>
              <TableCell className="px-4 py-3 font-mono text-xs text-muted-foreground">
                {node.id}
              </TableCell>
              {manage ? (
                <TableCell className="px-4 py-3 text-right">
                  {revoked ? null : (
                    <RevokeNodeButton nodeId={node.id} nodeName={node.name} />
                  )}
                </TableCell>
              ) : null}
            </TableRow>
          )
        })}
      </TableBody>
    </Table>
  )
}
