import type { NodeSummary } from "@meshway/control-client"

import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { formatRelative } from "@/lib/time"

export function NodesTable({
  nodes,
  fetchedAt,
}: {
  nodes: NodeSummary[]
  fetchedAt: number
}) {
  return (
    <Table className="text-sm">
      <TableHeader>
        <TableRow className="hover:bg-transparent">
          <TableHead className="px-4">名称</TableHead>
          <TableHead className="px-4">节点 ID</TableHead>
          <TableHead className="px-4">Agent 版本</TableHead>
          <TableHead className="px-4">最后上报</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {nodes.map((node) => (
          <TableRow key={node.id}>
            <TableCell className="px-4 py-3 font-medium">{node.name}</TableCell>
            <TableCell className="px-4 py-3 font-mono text-xs text-muted-foreground">
              {node.id}
            </TableCell>
            <TableCell className="px-4 py-3 font-mono text-xs">
              {node.agent_version}
            </TableCell>
            <TableCell className="px-4 py-3">
              <div className="flex flex-col">
                <span>{formatRelative(node.last_seen, fetchedAt) ?? "—"}</span>
                <time
                  dateTime={node.last_seen}
                  className="font-mono text-[0.6875rem] text-muted-foreground"
                >
                  {node.last_seen}
                </time>
              </div>
            </TableCell>
          </TableRow>
        ))}
      </TableBody>
    </Table>
  )
}
