import type { NodeSummary } from "@meshway/control-client"

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
