import { useNavigate } from "@tanstack/react-router"

import { Skeleton } from "@/components/ui/skeleton"
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table"
import { useNetworkView } from "@/features/network/use-network-view"
import { freshnessLabel, vitalityColor, vitalityLabel } from "@/features/overview/labels"
import { vitalityOf } from "@/features/overview/model"
import { useReadFailureToast } from "@/features/overview/network-overview"
import { runtimeFor, runtimeFreshness } from "@/features/topology/runtime"
import { relativeTime } from "@/lib/format"
import { m } from "@/paraglide/messages.js"

const PLACEHOLDER_ROWS = 4

/** Every active node of the network; a row opens it on the globe. */
export function NodesPage({ networkId }: { networkId: string }) {
  const view = useNetworkView(networkId)
  const navigate = useNavigate()
  useReadFailureToast("nodes", view.nodes.isError, m.overview_nodes_failed())

  const nodes = view.inventory.filter((node) => node.revoked_at === null).sort((a, b) => a.name.localeCompare(b.name))
  const runtimes = view.runtimes.data ?? null
  const open = (nodeId: string) =>
    void navigate({ to: "/networks/$network/overview", params: { network: networkId }, search: { node: nodeId } })

  return (
    <div className="px-4 pt-2 pb-8 md:px-6">
      <div className="flex items-baseline gap-3">
        <h1 className="text-2xl font-semibold tracking-tight">{m.nav_nodes()}</h1>
        <span className="text-sm text-muted-foreground tabular-nums">{view.nodeCount ?? "—"}</span>
      </div>

      <Table className="mt-6">
        <TableHeader>
          <TableRow className="hover:bg-transparent">
            <TableHead className="w-[28%]">{m.nodes_col_node()}</TableHead>
            <TableHead>{m.node_mesh_address()}</TableHead>
            <TableHead>{m.node_report()}</TableHead>
            <TableHead>{m.node_contact()}</TableHead>
            <TableHead>{m.node_receipt()}</TableHead>
            <TableHead className="text-right">{m.node_version()}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {view.nodes.data === undefined
            ? Array.from({ length: PLACEHOLDER_ROWS }, (_, index) => (
                <TableRow key={index} className="h-12 hover:bg-transparent">
                  {Array.from({ length: 6 }, (_, cell) => (
                    <TableCell key={cell}>
                      <Skeleton className="h-3.5 w-24 rounded-full" />
                    </TableCell>
                  ))}
                </TableRow>
              ))
            : nodes.length === 0
              ? (
                  <TableRow className="h-12 hover:bg-transparent">
                    <TableCell colSpan={6} className="text-muted-foreground">
                      {m.nodes_empty()}
                    </TableCell>
                  </TableRow>
                )
              : nodes.map((node) => {
                  const vitality = vitalityOf(node, runtimes, view.now)
                  const report = runtimeFor(runtimes, node.id)
                  const reportedAt = report?.reported_at ? relativeTime(report.reported_at, view.now) : null
                  const pending = node.acknowledged_revision < node.configuration_revision
                  return (
                    <TableRow
                      key={node.id}
                      tabIndex={0}
                      onClick={() => open(node.id)}
                      onKeyDown={(event) => {
                        if (event.key === "Enter") open(node.id)
                      }}
                      className="h-12 cursor-pointer outline-none focus-visible:bg-foreground/5"
                    >
                      <TableCell>
                        <span className="flex items-center gap-2.5">
                          <span
                            aria-label={vitalityLabel[vitality]()}
                            className="size-2 shrink-0 rounded-full"
                            style={{
                              background: vitalityColor[vitality],
                              boxShadow: vitality === "live" ? `0 0 8px 1px ${vitalityColor[vitality]}` : undefined,
                            }}
                          />
                          <span className="truncate font-medium">{node.name}</span>
                        </span>
                      </TableCell>
                      <TableCell className="font-mono text-[13px]">{node.mesh_ipv4}</TableCell>
                      <TableCell>
                        {freshnessLabel[runtimeFreshness(report, view.now)]()}
                        {reportedAt ? <span className="text-muted-foreground"> · {reportedAt}</span> : null}
                      </TableCell>
                      <TableCell>{relativeTime(node.last_seen, view.now) ?? m.node_never()}</TableCell>
                      <TableCell className={pending ? "font-mono text-[13px] text-warning" : "font-mono text-[13px]"}>
                        {pending
                          ? m.node_receipt_pending({ target: String(node.configuration_revision), acknowledged: String(node.acknowledged_revision) })
                          : m.node_receipt_current({ revision: String(node.acknowledged_revision) })}
                      </TableCell>
                      <TableCell className="text-right font-mono text-[13px] text-muted-foreground">{node.agent_version}</TableCell>
                    </TableRow>
                  )
                })}
        </TableBody>
      </Table>
    </div>
  )
}
