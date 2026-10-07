import type {
  ConfigurationRevisionSummary,
  NetworkSummary,
} from "@meshway/control-client"
import { useInfiniteQuery, useQuery } from "@tanstack/react-query"
import { ArrowLeftIcon, RotateCcwIcon } from "lucide-react"
import { useMemo, useState } from "react"

import { sessionQuery } from "@/api/queries/auth"
import {
  comparisonQuery,
  isDenied,
  revisionQuery,
  revisionsQuery,
} from "@/api/queries/configuration"
import { isNotFound } from "@/api/queries/network"
import { CloseButton } from "@/components/chrome/close-button"
import { RevisionLine } from "@/components/chrome/draft-mark"
import { FieldOption, FieldSelect } from "@/components/chrome/field-select"
import {
  ListNote,
  ListPlaceholders,
  ListRow,
  PanelList,
} from "@/components/chrome/list"
import { EmptyLine, Section } from "@/components/chrome/section"
import { SurfaceHeader } from "@/components/chrome/surface-header"
import { Tile } from "@/components/chrome/tile"
import { ChangeList, JsonBlock } from "@/components/configuration/change-list"
import { causeLabel } from "@/components/configuration/labels"
import { surfaceTitleId } from "@/components/surface/page-frame"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { Spinner } from "@/components/ui/spinner"
import { useIsMobile } from "@/hooks/use-media-query"
import { ACTOR_WORDS, actorText } from "@/lib/activity"
import { dateTime } from "@/lib/format"
import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"

import type { HistorySelection } from "../../-navigation"
import { ApplyProgress } from "./progress-section"

/** A revision's author by the email recorded with it (else their ID), or the system. */
/**
 * The configuration's revisions (Owner, Admin): who made each and why, one
 * of them compared with any other as the controller computes it, its
 * configuration, and restoring it, which goes through the same review as
 * any change.
 */
export function HistoryWorkspace({
  networkId,
  network,
  selection,
  checking,
  onSelect,
  onRestore,
  onClose,
}: {
  networkId: string
  /** null until read. */
  network: NetworkSummary | null
  selection: HistorySelection
  /** A restore is being checked. */
  checking: boolean
  onSelect: (selection: HistorySelection) => void
  onRestore: (revision: number, current: number) => void
  onClose: () => void
}) {
  const mobile = useIsMobile()
  const editable = network?.actions.editConfiguration ?? false
  const revisions = useInfiniteQuery({
    ...revisionsQuery(networkId),
    enabled: editable,
  })
  const session = useQuery(sessionQuery())
  const items = useMemo(
    () => (revisions.data?.pages ?? []).flatMap((page) => page.items),
    [revisions.data]
  )
  const current =
    revisions.data?.pages[0]?.currentRevision ??
    network?.currentRevision ??
    null
  const selected = selection.revision ?? (mobile ? null : current)
  const actor = (summary: ConfigurationRevisionSummary) =>
    actorText(summary.createdBy, session.data?.user.id ?? null, ACTOR_WORDS)

  const list = (
    <PanelList
      aria-labelledby={surfaceTitleId("configuration", "workspace")}
      className="px-0"
    >
      {revisions.isPending ? (
        <ListPlaceholders />
      ) : revisions.isError && items.length === 0 ? (
        <ListNote>
          {isDenied(revisions.error)
            ? m.configuration_read_denied()
            : m.configuration_read_failed()}
        </ListNote>
      ) : items.length === 0 ? (
        <ListNote>{m.configuration_history_empty()}</ListNote>
      ) : (
        items.map((summary) => (
          <ListRow
            key={summary.revision}
            rowId={String(summary.revision)}
            current={summary.revision === selected}
            onClick={() =>
              onSelect({ revision: summary.revision, against: null })
            }
            lead={
              <span
                className={cn(
                  "size-2 rounded-full",
                  summary.current
                    ? "bg-foreground"
                    : "border border-muted-foreground"
                )}
              />
            }
            title={
              <>
                <span className="shrink-0 font-mono tabular-nums">
                  {m.revision({ revision: summary.revision })}
                </span>
                <span className="truncate text-muted-foreground">
                  {summary.rollbackFrom !== null
                    ? m.configuration_history_rollback({
                        revision: summary.rollbackFrom,
                      })
                    : causeLabel[summary.cause]()}
                </span>
              </>
            }
            subtitle={
              <span className="truncate">
                {dateTime(summary.createdAt) ?? "—"} · {actor(summary)}
              </span>
            }
            trail={
              summary.current ? m.configuration_history_current() : undefined
            }
          />
        ))
      )}
      {revisions.hasNextPage ? (
        <li className="flex justify-center py-2">
          <Button
            variant="ghost"
            size="sm"
            className="min-w-28 rounded-full"
            disabled={revisions.isFetchingNextPage}
            aria-busy={revisions.isFetchingNextPage}
            onClick={() => void revisions.fetchNextPage()}
          >
            {revisions.isFetchingNextPage ? <Spinner /> : null}
            {m.configuration_history_more()}
          </Button>
        </li>
      ) : null}
    </PanelList>
  )

  const summary = items.find((item) => item.revision === selected) ?? null
  const detail =
    selected === null ? null : (
      <RevisionDetail
        key={selected}
        networkId={networkId}
        revision={selected}
        summary={summary}
        actor={summary ? actor(summary) : null}
        items={items}
        against={selection.against}
        current={current}
        checking={checking}
        onAgainst={(against) => onSelect({ revision: selected, against })}
        onRestore={() => {
          if (current !== null) onRestore(selected, current)
        }}
      />
    )

  return (
    <div className="swap-in flex min-h-0 flex-1 flex-col">
      <SurfaceHeader
        size="lg"
        className="px-6 pt-6 pb-4"
        title={(className) => (
          <h2
            id={surfaceTitleId("configuration", "workspace")}
            className={className}
          >
            {m.configuration_history()}
          </h2>
        )}
        action={<CloseButton onClick={onClose} />}
        meta={
          <RevisionLine
            revision={current}
            draftBase={null}
            facts={
              revisions.data
                ? [m.configuration_history_count({ count: items.length })]
                : []
            }
          />
        }
      />
      {!editable ? (
        <div className="px-6 pb-6">
          <EmptyLine>{m.configuration_read_denied()}</EmptyLine>
        </div>
      ) : mobile ? (
        selected === null ? (
          <div className="flex min-h-0 flex-1 flex-col px-4 pb-4">{list}</div>
        ) : (
          <div className="scroll-edge min-h-0 flex-1 overflow-y-auto px-6 pt-1 pb-6">
            <Button
              variant="ghost"
              size="sm"
              className="mb-3 -ml-2 rounded-full"
              onClick={() => onSelect({ revision: null, against: null })}
            >
              <ArrowLeftIcon />
              {m.configuration_history_all()}
            </Button>
            {detail}
          </div>
        )
      ) : (
        <div className="grid min-h-0 flex-1 grid-cols-[18rem_minmax(0,1fr)] gap-4 px-4 pb-4">
          <div className="flex min-h-0 flex-col">{list}</div>
          <div className="scroll-edge min-h-0 overflow-y-auto px-2 pt-1 pb-2">
            {detail ?? (
              <EmptyLine>{m.configuration_history_choose()}</EmptyLine>
            )}
          </div>
        </div>
      )}
    </div>
  )
}

function RevisionDetail({
  networkId,
  revision,
  summary,
  actor,
  items,
  against,
  current,
  checking,
  onAgainst,
  onRestore,
}: {
  networkId: string
  revision: number
  /** null while that page of the list is unread. */
  summary: ConfigurationRevisionSummary | null
  actor: string | null
  /** The revisions read so far. */
  items: readonly ConfigurationRevisionSummary[]
  /** The revision to compare with; null for the one before it. */
  against: number | null
  current: number | null
  checking: boolean
  onAgainst: (against: number | null) => void
  onRestore: () => void
}) {
  const [showConfiguration, setShowConfiguration] = useState(false)
  const previous = revision > 1 ? revision - 1 : null
  const other = against ?? previous
  const view = useQuery(revisionQuery(networkId, revision))
  const otherView = useQuery({
    ...revisionQuery(networkId, other ?? 1),
    enabled: other !== null,
  })
  const comparison = useQuery({
    ...comparisonQuery(networkId, other ?? 1, revision),
    enabled: other !== null,
  })
  const options = items.filter((item) => item.revision !== revision)
  const isCurrent = summary?.current ?? revision === current
  const yesNo = (changed: boolean) =>
    changed ? m.configuration_history_changed() : m.configuration_history_same()

  // A revision the network does not have (an address typed or kept from elsewhere).
  if (isNotFound(view.error))
    return (
      <EmptyLine>{m.configuration_history_missing({ revision })}</EmptyLine>
    )

  return (
    <div className="flex flex-col gap-8">
      <div className="grid gap-2 sm:grid-cols-2">
        <Tile label={m.configuration_history_revision()}>
          <span className="font-mono">{m.revision({ revision })}</span>
          {isCurrent ? ` · ${m.configuration_history_current()}` : ""}
        </Tile>
        <Tile label={m.configuration_history_cause()}>
          {summary === null
            ? "—"
            : summary.rollbackFrom !== null
              ? m.configuration_history_rollback({
                  revision: summary.rollbackFrom,
                })
              : causeLabel[summary.cause]()}
        </Tile>
        <Tile label={m.configuration_history_time()}>
          {summary ? (dateTime(summary.createdAt) ?? "—") : "—"}
        </Tile>
        <Tile label={m.configuration_history_actor()}>{actor ?? "—"}</Tile>
      </div>

      <div className="flex flex-wrap gap-2">
        <Button
          variant="secondary"
          className="h-9 rounded-full px-4"
          onClick={() => setShowConfiguration((shown) => !shown)}
          aria-expanded={showConfiguration}
        >
          {showConfiguration
            ? m.configuration_history_hide()
            : m.configuration_history_view()}
        </Button>
        <Button
          variant="secondary"
          className="h-9 min-w-36 rounded-full px-4"
          disabled={isCurrent || current === null || checking}
          aria-busy={checking}
          onClick={onRestore}
        >
          {checking ? <Spinner /> : <RotateCcwIcon />}
          {m.configuration_history_restore()}
        </Button>
      </div>

      <ApplyProgress
        networkId={networkId}
        revision={revision}
        current={isCurrent}
      />

      {showConfiguration ? (
        view.data ? (
          <JsonBlock value={view.data.configuration} className="max-h-96" />
        ) : (
          <Skeleton className="h-40 w-full rounded-tile" />
        )
      ) : null}

      <Section
        title={m.configuration_history_against()}
        action={
          <div className="w-40">
            <FieldSelect
              name="against"
              aria-label={m.configuration_history_against()}
              value={other === null ? "" : String(other)}
              onChange={(event) =>
                onAgainst(
                  event.target.value === "" ? null : Number(event.target.value)
                )
              }
              className="h-8 text-row"
            >
              {other === null ? (
                <FieldOption value="">
                  {m.configuration_history_against_none()}
                </FieldOption>
              ) : null}
              {other !== null &&
              !options.some((item) => item.revision === other) ? (
                <FieldOption value={String(other)}>
                  {m.revision({ revision: other })}
                </FieldOption>
              ) : null}
              {options.map((item) => (
                <FieldOption key={item.revision} value={String(item.revision)}>
                  {m.revision({ revision: item.revision })}
                </FieldOption>
              ))}
            </FieldSelect>
          </div>
        }
      >
        {other === null ? (
          <EmptyLine>{m.configuration_history_no_changes()}</EmptyLine>
        ) : comparison.isError ? (
          <EmptyLine>
            {isNotFound(otherView.error)
              ? m.configuration_history_missing({ revision: other })
              : m.configuration_history_compare_failed()}
          </EmptyLine>
        ) : !comparison.data ? (
          <Skeleton className="h-12 w-full rounded-tile" />
        ) : (
          <div className="flex flex-col gap-4">
            <div className="grid gap-2 sm:grid-cols-3">
              <Tile label={m.configuration_history_configuration_changed()}>
                {yesNo(comparison.data.configurationChanged)}
              </Tile>
              <Tile label={m.configuration_review_source()}>
                {yesNo(comparison.data.sourceChanged)}
              </Tile>
              <Tile label={m.configuration_history_bindings_changed()}>
                {yesNo(comparison.data.bindingsChanged)}
              </Tile>
            </div>
            {comparison.data.changes.length === 0 ? (
              <EmptyLine>{m.configuration_history_no_changes()}</EmptyLine>
            ) : (
              <ChangeList
                changes={comparison.data.changes}
                before={otherView.data?.configuration}
                after={view.data?.configuration}
                labels={{
                  before: m.revision({ revision: other }),
                  after: m.revision({ revision }),
                }}
              />
            )}
          </div>
        )}
      </Section>
    </div>
  )
}
