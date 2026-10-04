import { useState } from "react"

import { EmptyLine, Section } from "@/components/chrome/section"
import { SegmentedControl } from "@/components/chrome/segmented-control"
import { Tile } from "@/components/chrome/tile"
import { useNow } from "@/hooks/use-now"
import { agoTime } from "@/lib/format"
import { connectionWords } from "@/lib/live-labels"
import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"
import {
  useStreamTrace,
  type StreamControl,
  type StreamEntry,
} from "@/state/debug-trace"
import { useConnection } from "@/state/live"

import { byteSize, duration, keyLabel, traceTime } from "./format"
import { DataTree } from "./tree"

type Filter = "all" | "events" | "connection"

const controlLabel: Record<StreamControl, () => string> = {
  drop: m.debug_drop,
  resync: m.debug_resync,
  pause: m.debug_pause,
  resume: m.debug_resume,
}

const shows = (filter: Filter, entry: StreamEntry) =>
  filter === "all" || (filter === "events") === (entry.kind === "event")

/** The open network's stream: how it stands, and what it did, newest first. */
export function StreamTab({ networkId }: { networkId: string | null }) {
  if (!networkId) return <EmptyLine>{m.debug_no_network()}</EmptyLine>
  return <Stream networkId={networkId} />
}

function Stream({ networkId }: { networkId: string }) {
  const trace = useStreamTrace(networkId)
  const connection = useConnection(networkId)
  const now = useNow(1_000)
  const [filter, setFilter] = useState<Filter>("all")
  const [open, setOpen] = useState<ReadonlySet<number>>(() => new Set())
  const toggle = (seq: number) =>
    setOpen((current) => {
      const next = new Set(current)
      if (!next.delete(seq)) next.add(seq)
      return next
    })
  const entries = trace.entries
    .filter((entry) => shows(filter, entry))
    .toReversed()

  return (
    <div className="flex flex-col gap-6">
      <div className="grid grid-cols-2 gap-2 md:grid-cols-4 xl:grid-cols-7">
        <Tile label={m.debug_connection()}>
          {connectionWords(connection).title}
        </Tile>
        <Tile label={m.debug_cursor()}>
          <span className="font-mono font-normal">{trace.cursor ?? "—"}</span>
        </Tile>
        <Tile label={m.debug_view_revision()}>
          {trace.revision === null
            ? "—"
            : m.revision({ revision: trace.revision })}
        </Tile>
        <Tile label={m.debug_events()}>{trace.events}</Tile>
        <Tile label={m.debug_waits()}>{trace.waits}</Tile>
        <Tile label={m.debug_last_event()}>
          {trace.lastEventAt === null ? "—" : agoTime(trace.lastEventAt, now)}
        </Tile>
        <Tile label={m.debug_idle_bound()}>
          {trace.disconnectAfterMs === null
            ? "—"
            : duration(trace.disconnectAfterMs)}
        </Tile>
      </div>
      <Section
        title={m.debug_entries()}
        count={entries.length}
        action={
          <SegmentedControl
            size="sm"
            label={m.debug_entries()}
            value={filter}
            onChange={setFilter}
            options={[
              { value: "all", label: m.debug_filter_all() },
              { value: "events", label: m.debug_events() },
              { value: "connection", label: m.debug_connection() },
            ]}
          />
        }
      >
        {entries.length === 0 ? (
          <EmptyLine>{m.debug_nothing_recorded()}</EmptyLine>
        ) : (
          <ol className="flex flex-col font-mono text-meta">
            {entries.map((entry) => (
              <EntryRow
                key={entry.seq}
                entry={entry}
                open={open.has(entry.seq)}
                onToggle={() => toggle(entry.seq)}
              />
            ))}
          </ol>
        )}
      </Section>
    </div>
  )
}

const ROW =
  "grid w-full grid-cols-[7rem_10rem_minmax(0,1fr)_5rem] items-baseline gap-3 rounded-sm px-2 py-1 text-left"

function EntryRow({
  entry,
  open,
  onToggle,
}: {
  entry: StreamEntry
  open: boolean
  onToggle: () => void
}) {
  const time = (
    <span className="text-muted-foreground">{traceTime(entry.at)}</span>
  )
  switch (entry.kind) {
    case "event": {
      const { event } = entry
      const reason =
        event.type === "sync.reset" || event.type === "stream.closed"
          ? event.reason
          : null
      const writes = entry.touched.filter((t) => t.op === "write").length
      return (
        <li>
          <button
            type="button"
            aria-expanded={open}
            onClick={onToggle}
            className={cn(
              ROW,
              "outline-none hover:bg-fill-hover focus-visible:ring-2 focus-visible:ring-ring",
              open && "bg-fill-subtle"
            )}
          >
            {time}
            <span className="truncate">
              {reason ? `${event.type} · ${reason}` : event.type}
            </span>
            <span className="truncate text-muted-foreground">
              {entry.effects.length ? entry.effects.join(", ") : "—"}
              {entry.touched.length
                ? ` · ${m.debug_touched({
                    writes,
                    invalidations: entry.touched.length - writes,
                  })}`
                : ""}
            </span>
            <span className="text-right text-muted-foreground tabular-nums">
              {byteSize(entry.size)}
            </span>
          </button>
          {open ? (
            <div className="flex flex-col gap-3 py-2 pr-2 pl-[8.25rem]">
              {entry.id ? (
                <p className="break-all text-muted-foreground">
                  {m.debug_event_id({ id: entry.id })}
                </p>
              ) : null}
              {entry.touched.length ? (
                <ul className="flex flex-col gap-0.5">
                  {entry.touched.map((touch, index) => (
                    <li key={index} className="flex gap-2">
                      <span className="text-muted-foreground">
                        {touch.op === "write"
                          ? m.debug_write()
                          : m.debug_invalidate()}
                      </span>
                      <span className="break-all">{keyLabel(touch.key)}</span>
                    </li>
                  ))}
                </ul>
              ) : null}
              <DataTree value={event} />
            </div>
          ) : null}
        </li>
      )
    }
    case "connection":
      return (
        <li className={ROW}>
          {time}
          <span>{m.debug_connection()}</span>
          <span className="truncate">
            {connectionWords(entry.connection).title}
          </span>
        </li>
      )
    case "failure": {
      const { kind, status, code } = entry.failure
      return (
        <li className={cn(ROW, "text-destructive")}>
          {time}
          <span>{m.debug_filter_failed()}</span>
          <span className="truncate">
            {[kind, status, code].filter((part) => part !== null).join(" ")}
            {` → ${entry.action}`}
          </span>
        </li>
      )
    }
    case "wait":
      return (
        <li className={ROW}>
          {time}
          <span>{m.debug_kind_wait()}</span>
          <span className="truncate">
            {m.debug_wait({ time: duration(entry.ms) })}
          </span>
        </li>
      )
    case "control":
      return (
        <li className={cn(ROW, "text-warning")}>
          {time}
          <span>{m.debug_kind_control()}</span>
          <span className="truncate">{controlLabel[entry.control]()}</span>
        </li>
      )
  }
}
