import { Button } from "@meshway/ui/button"

import { failureText } from "@/api/failure"
import { historyFailureText } from "@/api/queries/history"
import { itemCount, type useHistory } from "@/api/queries/use-history"
import {
  CoverageNotes,
  WindowChoices,
  WindowRange,
} from "@/components/history/window-control"
import { Skeleton } from "@/components/ui/skeleton"
import { dateTime } from "@/lib/format"
import type { HistoryWindow, WindowChoice } from "@/lib/history/window"
import { linkKey, type HistoryLink } from "@/lib/network-map"
import { transportText } from "@/lib/state-labels"
import { m } from "@/paraglide/messages.js"

import { Group } from "./-group"

export type HistoryLayerProps = {
  window: HistoryWindow | null
  onChoose: (choice: WindowChoice | null) => void
  history: ReturnType<typeof useHistory>
  links: readonly HistoryLink[] | null
  located: ReadonlySet<string>
  nameOf: (id: string) => string
}

/**
 * The history layer: off until a window is chosen. Then the links the
 * nodes recorded opening or closing in it, drawn faint between located
 * ends (a link that is current now is drawn as current only), with what
 * the records do not cover.
 */
export function HistoryLayer({
  window,
  onChoose,
  history,
  links,
  located,
  nameOf,
}: HistoryLayerProps) {
  const { query } = history
  return (
    <section className="flex flex-col gap-2 border-t border-divider pt-3">
      <div className="flex flex-wrap items-center gap-2">
        <h3 className="flex-1 text-meta font-medium text-muted-foreground">
          {m.overview_history()}
        </h3>
        <WindowChoices
          value={window?.choice ?? null}
          offLabel={m.overview_history_off()}
          onChange={onChoose}
        />
      </div>
      {window ? (
        <>
          <WindowRange
            window={window}
            busy={query.isFetching}
            onRefresh={() => onChoose(window.choice)}
          />
          <CoverageNotes
            coverage={history.coverage}
            window={window}
            nameOf={nameOf}
          />
          {query.data === undefined ? (
            query.isError ? (
              <p className="text-row text-muted-foreground">
                {failureText(query.error, historyFailureText) ??
                  m.history_failed()}
              </p>
            ) : (
              <Skeleton className="h-3 w-40 rounded-full" />
            )
          ) : links && links.length === 0 && history.complete ? (
            <p className="text-row text-muted-foreground">
              {m.overview_history_none()}
            </p>
          ) : links ? (
            <Group
              title={m.overview_history_seen({ count: String(links.length) })}
              defaultOpen={false}
            >
              <ul className="flex flex-col gap-0.5">
                {links.map((link) => {
                  return (
                    <li
                      key={linkKey(link)}
                      className="flex min-h-12 items-center gap-3 rounded-row px-2.5 py-1.5"
                    >
                      <span
                        aria-hidden="true"
                        className="h-0.5 w-3 shrink-0 rounded-full bg-[var(--globe-land)] opacity-50"
                      />
                      <span className="min-w-0 flex-1">
                        <span className="block truncate text-row font-medium">
                          {nameOf(link.a)} ↔ {nameOf(link.b)}
                        </span>
                        <span className="block truncate text-meta text-muted-foreground">
                          {transportText(link.transport)}
                        </span>
                        <span className="block truncate text-meta text-muted-foreground">
                          {link.closed
                            ? m.overview_history_last_closed()
                            : m.overview_history_last_opened()}{" "}
                          · {dateTime(link.lastEventAt)}
                        </span>
                      </span>
                      {located.has(link.a) && located.has(link.b) ? null : (
                        <span className="shrink-0 text-meta text-muted-foreground">
                          {m.overview_link_unlocated_end()}
                        </span>
                      )}
                    </li>
                  )
                })}
              </ul>
            </Group>
          ) : null}
          {query.isError && query.data !== undefined ? (
            <p className="text-row text-muted-foreground">
              {failureText(query.error, historyFailureText) ??
                m.history_failed()}
            </p>
          ) : null}
          {history.waiting ? (
            <div className="flex items-center gap-3 text-meta text-muted-foreground">
              <span>
                {m.history_partial({
                  count: String(itemCount(history.pages)),
                })}
              </span>
              <Button variant="outline" size="sm" onClick={history.more}>
                {m.history_more()}
              </Button>
            </div>
          ) : null}
        </>
      ) : null}
    </section>
  )
}
