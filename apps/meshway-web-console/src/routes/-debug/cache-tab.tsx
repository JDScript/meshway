import { matchQuery, useQueryClient } from "@tanstack/react-query"
import { useState } from "react"

import { networkKey } from "@/api/queries/network"
import { EmptyLine, Section } from "@/components/chrome/section"
import { SegmentedControl } from "@/components/chrome/segmented-control"
import { useNow } from "@/hooks/use-now"
import { agoTime } from "@/lib/format"
import { m } from "@/paraglide/messages.js"
import { useCacheTrace, type CacheTrace } from "@/state/debug-trace"

import { DataTable } from "./data-table"
import { keyLabel } from "./format"

type Scope = "network" | "all"

const sourceLabel = (source: CacheTrace["lastUpdate"] | undefined) => {
  switch (source) {
    case "fetch":
      return "GET"
    case "stream":
      return "SSE"
    case "write":
      return m.debug_source_write()
    default:
      return "—"
  }
}

/**
 * The query cache as it stands, read again each second: each read's state,
 * where its data last came from and, while recording, how often it was
 * fetched, written and marked stale.
 */
export function CacheTab({ networkId }: { networkId: string | null }) {
  const queryClient = useQueryClient()
  const trace = useCacheTrace()
  const now = useNow(1_000)
  const [scope, setScope] = useState<Scope>("network")
  const queries = queryClient
    .getQueryCache()
    .getAll()
    .filter(
      (query) =>
        scope === "all" ||
        (networkId !== null &&
          matchQuery({ queryKey: networkKey(networkId) }, query))
    )
    .sort((a, b) => a.queryHash.localeCompare(b.queryHash))

  return (
    <Section
      title={m.debug_queries()}
      count={queries.length}
      action={
        <SegmentedControl
          size="sm"
          label={m.debug_queries()}
          value={scope}
          onChange={setScope}
          options={[
            { value: "network", label: m.debug_scope_network() },
            { value: "all", label: m.debug_filter_all() },
          ]}
        />
      }
    >
      {queries.length === 0 ? (
        <EmptyLine>{m.debug_no_queries()}</EmptyLine>
      ) : (
        <DataTable
          columns={[
            { label: m.debug_key(), long: true },
            { label: m.debug_status() },
            { label: m.debug_source() },
            { label: m.debug_updated() },
            { label: "GET", numeric: true },
            { label: "SSE", numeric: true },
            { label: m.debug_source_write(), numeric: true },
            { label: m.debug_invalidations(), numeric: true },
            { label: m.debug_observers(), numeric: true },
          ]}
          rows={queries.map((query) => {
            const recorded = trace[query.queryHash]
            const { status, fetchStatus, dataUpdatedAt } = query.state
            return {
              key: query.queryHash,
              tone: status === "error" ? "destructive" : undefined,
              cells: [
                keyLabel(query.queryKey),
                [status, fetchStatus, query.isStale() ? "stale" : null]
                  .filter((part) => part !== null)
                  .join(" · "),
                sourceLabel(recorded?.lastUpdate),
                dataUpdatedAt ? agoTime(dataUpdatedAt, now) : "—",
                recorded?.fetches ?? 0,
                recorded?.streamWrites ?? 0,
                recorded?.writes ?? 0,
                recorded?.invalidations ?? 0,
                query.getObserversCount(),
              ],
            }
          })}
        />
      )}
    </Section>
  )
}
