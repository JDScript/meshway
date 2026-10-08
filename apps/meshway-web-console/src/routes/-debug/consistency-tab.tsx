import { Button } from "@meshway/ui/button"
import { cn } from "@meshway/ui/cn"
import { useQueryClient } from "@tanstack/react-query"
import { useState } from "react"
import { toast } from "sonner"

import { BusyLabel } from "@/components/chrome/busy-label"
import { EmptyLine, Section } from "@/components/chrome/section"
import { useNow } from "@/hooks/use-now"
import { agoTime } from "@/lib/format"
import { m } from "@/paraglide/messages.js"

import { checkConsistency, type Check, type Resource } from "./consistency"
import { brief } from "./format"

type Result = { networkId: string; at: number; checks: Check[] }

const resourceLabel = (resource: Resource) => {
  switch (resource.type) {
    case "network":
      return m.debug_network()
    case "nodes":
      return m.debug_resource_nodes()
    case "state":
      return m.debug_state()
    case "progress":
      return m.debug_resource_progress({ revision: resource.revision })
  }
}

function outcomeText(check: Check) {
  const { outcome } = check
  switch (outcome.type) {
    case "same":
      return m.debug_same()
    case "different":
      return m.debug_different({ count: outcome.differences.total })
    case "uncached":
      return m.debug_uncached()
    case "failed":
      return m.debug_read_failed({ code: outcome.code ?? "—" })
  }
}

/** The open network's streamed reads against a fresh GET of each, on request. */
export function ConsistencyTab({ networkId }: { networkId: string | null }) {
  if (!networkId) return <EmptyLine>{m.debug_no_network()}</EmptyLine>
  return <Consistency networkId={networkId} />
}

function Consistency({ networkId }: { networkId: string }) {
  const queryClient = useQueryClient()
  const now = useNow(1_000)
  const [result, setResult] = useState<Result | null>(null)
  const [running, setRunning] = useState(false)
  const [open, setOpen] = useState<number | null>(null)
  const shown = result?.networkId === networkId ? result : null

  const run = () => {
    setRunning(true)
    checkConsistency(queryClient, networkId)
      .then(
        (checks) => {
          setResult({ networkId, at: Date.now(), checks })
          setOpen(null)
        },
        () => toast.error(m.debug_check_failed())
      )
      .finally(() => setRunning(false))
  }

  return (
    <Section
      title={m.debug_consistency()}
      count={shown ? agoTime(shown.at, now) : null}
      action={
        <Button
          variant="secondary"
          size="sm"
          className="rounded-full px-3"
          disabled={running}
          aria-busy={running}
          onClick={run}
        >
          <BusyLabel busy={running}>{m.debug_check()}</BusyLabel>
        </Button>
      }
    >
      {shown ? (
        <ul className="flex flex-col gap-1">
          {shown.checks.map((check, index) => {
            const { outcome } = check
            const expandable = outcome.type === "different"
            return (
              <li key={index} className="rounded-tile bg-fill-subtle">
                <button
                  type="button"
                  disabled={!expandable}
                  aria-expanded={expandable ? open === index : undefined}
                  onClick={() => setOpen(open === index ? null : index)}
                  className="flex h-10 w-full items-center gap-3 rounded-tile px-3.5 text-left text-row outline-none focus-visible:ring-2 focus-visible:ring-ring enabled:hover:bg-fill-hover"
                >
                  <span className="w-40 shrink-0 font-medium">
                    {resourceLabel(check.resource)}
                  </span>
                  <span
                    className={cn(
                      "flex-1 truncate",
                      outcome.type === "different" && "text-warning",
                      outcome.type === "failed" && "text-destructive"
                    )}
                  >
                    {outcomeText(check)}
                  </span>
                  {check.sequences ? (
                    <span className="font-mono text-meta text-muted-foreground tabular-nums">
                      {m.debug_sequences(check.sequences)}
                    </span>
                  ) : null}
                </button>
                {outcome.type === "different" && open === index ? (
                  <table className="mx-3.5 mb-3 font-mono text-meta">
                    <thead className="text-muted-foreground">
                      <tr>
                        <th className="pr-4 text-left font-normal">
                          {m.debug_path()}
                        </th>
                        <th className="pr-4 text-left font-normal">
                          {m.debug_cached()}
                        </th>
                        <th className="text-left font-normal">GET</th>
                      </tr>
                    </thead>
                    <tbody>
                      {outcome.differences.found.map((difference) => (
                        <tr key={difference.path} className="align-top">
                          <td className="pr-4 break-all">
                            {difference.path || "/"}
                          </td>
                          <td className="pr-4 break-all">
                            {brief(difference.cached)}
                          </td>
                          <td className="break-all">
                            {brief(difference.read)}
                          </td>
                        </tr>
                      ))}
                      {outcome.differences.total >
                      outcome.differences.found.length ? (
                        <tr>
                          <td colSpan={3} className="text-muted-foreground">
                            {m.debug_more_differences({
                              count:
                                outcome.differences.total -
                                outcome.differences.found.length,
                            })}
                          </td>
                        </tr>
                      ) : null}
                    </tbody>
                  </table>
                ) : null}
              </li>
            )
          })}
        </ul>
      ) : (
        <EmptyLine>{m.debug_not_checked()}</EmptyLine>
      )}
    </Section>
  )
}
