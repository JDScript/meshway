import { useState } from "react"

import { EmptyLine, Section } from "@/components/chrome/section"
import { SegmentedControl } from "@/components/chrome/segmented-control"
import { m } from "@/paraglide/messages.js"
import { useRequestTrace, type RequestEntry } from "@/state/debug-trace"

import { DataTable } from "./data-table"
import { milliseconds, shortIds, traceTime } from "./format"

type Filter = "all" | "failed"

const failed = (request: RequestEntry) =>
  request.status === null || request.status >= 400

const outcome = (request: RequestEntry) => {
  if (request.status !== null) return String(request.status)
  return request.failed === "cancelled"
    ? m.debug_request_cancelled()
    : m.debug_request_network()
}

/** The requests made while recording, newest first: never a header or a body. */
export function RequestsTab() {
  const requests = useRequestTrace()
  const [filter, setFilter] = useState<Filter>("all")
  const shown = requests
    .filter((request) => filter === "all" || failed(request))
    .toReversed()

  return (
    <Section
      title={m.debug_requests()}
      count={shown.length}
      action={
        <SegmentedControl
          size="sm"
          label={m.debug_requests()}
          value={filter}
          onChange={setFilter}
          options={[
            { value: "all", label: m.debug_filter_all() },
            { value: "failed", label: m.debug_filter_failed() },
          ]}
        />
      }
    >
      {shown.length === 0 ? (
        <EmptyLine>{m.debug_nothing_recorded()}</EmptyLine>
      ) : (
        <DataTable
          columns={[
            { label: m.debug_time() },
            { label: m.debug_method() },
            { label: m.debug_path() },
            { label: m.debug_status() },
            { label: m.debug_duration(), numeric: true },
            { label: m.debug_problem() },
          ]}
          rows={shown.map((request) => ({
            key: request.seq,
            tone: failed(request) ? "destructive" : undefined,
            cells: [
              traceTime(request.at),
              request.stream ? `${request.method} SSE` : request.method,
              shortIds(request.path),
              outcome(request),
              milliseconds(request.ms),
              request.code ?? "",
            ],
          }))}
        />
      )}
    </Section>
  )
}
