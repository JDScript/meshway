import type {
  RuleSet,
  RuleSetFormat,
  RuleSetView,
} from "@meshway/control-client"
import { CircleAlertIcon, CircleCheckIcon, RefreshCwIcon } from "lucide-react"
import type { ReactNode } from "react"

import { toastFailureWith } from "@/api/failure"
import { useRefreshRuleSet } from "@/api/queries/rule-sets"
import { BusyLabel } from "@/components/chrome/busy-label"
import { FieldOption, FieldSelect } from "@/components/chrome/field-select"
import { FieldInput } from "@/components/chrome/form"
import { Section } from "@/components/chrome/section"
import { SegmentedControl } from "@/components/chrome/segmented-control"
import { ChipSetField } from "@/components/configuration/chips"
import {
  NoneLine,
  Notes,
  notesWithin,
  refusedAt,
  RemoveButton,
  setRefusal,
  type Note,
} from "@/components/configuration/form-kit"
import { Button } from "@/components/ui/button"
import { useNow } from "@/hooks/use-now"
import { RESOURCE_PROBLEM } from "@/lib/configuration/diagnostics"
import { pointerOf } from "@/lib/configuration/jsonc"
import { withEntry, without } from "@/lib/configuration/keyed"
import {
  DEFAULT_INTERVAL,
  inlineOf,
  inlineParts,
  inlineValues,
  preparationOf,
  refreshable,
  RULE_SET_FORMATS,
  type Preparation,
  type RuleSetMap,
} from "@/lib/configuration/rule-sets"
import { agoTime, dateTime } from "@/lib/format"
import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"

import { AddKey, KeyInput, type KeyRule } from "./keyed-form"
import { destinationChips } from "./route-kit"

const FORMAT: Record<RuleSetFormat, () => string> = {
  "sing-box-source": () => m.configuration_ruleset_format_sing_box(),
  "mihomo-domain": () => m.configuration_ruleset_format_mihomo_domain(),
  "mihomo-ipcidr": () => m.configuration_ruleset_format_mihomo_ipcidr(),
}

const setKey = (): KeyRule => ({
  prefix: "",
  placeholder: m.configuration_ruleset_key_placeholder(),
})

/** The status the controller reports, read or not. */
export type RuleSetStatus =
  | { kind: "read"; views: ReadonlyMap<string, RuleSetView> }
  | { kind: "loading" }
  | { kind: "failed" }

/** Rule sets: inline content, or a remote source the controller downloads with the draft, on its interval and when refreshed. */
export function RuleSetsForm({
  networkId,
  sets,
  saved,
  status,
  referenced,
  editable,
  notes,
  onChange,
  onRename,
}: {
  networkId: string
  sets: RuleSetMap
  /** The sets at the controller's current revision, which its status is for. */
  saved: RuleSetMap
  status: RuleSetStatus
  /** The keys the draft's routing names. */
  referenced: ReadonlySet<string>
  editable: boolean
  notes: readonly Note[]
  onChange: (sets: RuleSetMap) => void
  /** A set renamed: its references follow. */
  onRename: (from: string, to: string) => void
}) {
  const entries = Object.entries(sets)
  const now = useNow(30_000)
  return (
    <div className="flex flex-col gap-4">
      <p className="text-meta text-muted-foreground">
        {m.configuration_ruleset_hint()}
      </p>
      <Section
        title={m.configuration_section_ruleSets()}
        count={entries.length}
      >
        <div className="flex flex-col gap-3">
          {entries.length === 0 ? (
            <NoneLine>{m.configuration_ruleset_none()}</NoneLine>
          ) : null}
          <ul
            aria-label={m.configuration_section_ruleSets()}
            className="flex flex-col gap-3"
          >
            {entries.map(([key, set], index) => (
              // A set is its place: renaming keeps it.
              <li key={index}>
                <RuleSetCard
                  networkId={networkId}
                  map={sets}
                  name={key}
                  set={set}
                  saved={saved[key]}
                  status={status}
                  referenced={referenced.has(key)}
                  editable={editable}
                  notes={notesWithin(notes, pointerOf(["ruleSets", key]))}
                  now={now}
                  onChange={(next) => onChange(withEntry(sets, key, next))}
                  onRename={(next) => onRename(key, next)}
                  onRemove={() => onChange(without(sets, key))}
                />
              </li>
            ))}
          </ul>
          {editable ? (
            <AddKey
              map={sets}
              rule={setKey()}
              label={m.configuration_ruleset_new()}
              addLabel={m.configuration_key_add()}
              onAdd={(key) =>
                onChange(
                  withEntry<RuleSet>(sets, key, {
                    type: "remote",
                    url: "",
                    format: "sing-box-source",
                  })
                )
              }
            />
          ) : null}
        </div>
      </Section>
    </div>
  )
}

function RuleSetCard({
  networkId,
  map,
  name,
  set,
  saved,
  status,
  referenced,
  editable,
  notes,
  now,
  onChange,
  onRename,
  onRemove,
}: {
  networkId: string
  map: RuleSetMap
  name: string
  set: RuleSet
  saved: RuleSet | undefined
  status: RuleSetStatus
  referenced: boolean
  editable: boolean
  notes: readonly Note[]
  now: number
  onChange: (set: RuleSet) => void
  onRename: (key: string) => void
  onRemove: () => void
}) {
  const at = (field?: string) =>
    pointerOf(field ? ["ruleSets", name, field] : ["ruleSets", name])
  const view = status.kind === "read" ? status.views.get(name) : undefined
  const preparation = preparationOf(set, saved, view)
  return (
    <div
      role="group"
      aria-label={name}
      className="group/rule flex flex-col gap-3 rounded-surface border border-divider p-3"
    >
      <div className="flex min-h-8 flex-wrap items-center gap-2">
        <KeyInput
          map={map}
          value={name}
          rule={setKey()}
          label={m.configuration_ruleset_key()}
          editable={editable}
          onRename={onRename}
          className="w-56 min-w-0"
        />
        <span className="flex-1" />
        {editable ? (
          <SegmentedControl
            size="sm"
            label={m.configuration_ruleset_type()}
            value={set.type}
            options={[
              { value: "remote", label: m.configuration_ruleset_remote() },
              { value: "inline", label: m.configuration_ruleset_inline() },
            ]}
            onChange={(type) => {
              if (type === set.type) return
              onChange(
                type === "inline"
                  ? { type: "inline" }
                  : { type: "remote", url: "", format: "sing-box-source" }
              )
            }}
          />
        ) : (
          <span className="inline-flex h-6 items-center rounded-full bg-fill-subtle px-2.5 text-meta font-medium">
            {set.type === "inline"
              ? m.configuration_ruleset_inline()
              : m.configuration_ruleset_remote()}
          </span>
        )}
        {editable ? (
          <RemoveButton
            label={m.configuration_key_remove({ name })}
            onRemove={onRemove}
          />
        ) : null}
      </div>
      {set.type === "inline" ? (
        <ChipSetField
          values={inlineValues(set)}
          kind={destinationChips([], false)}
          label={m.configuration_ruleset_entries()}
          addLabel={m.configuration_ruleset_entries_add()}
          editable={editable}
          invalid={setRefusal(notes, at(), inlineParts(set))}
          onChange={(values) => onChange(inlineOf(values))}
        />
      ) : (
        <div className="grid grid-cols-1 gap-2 sm:grid-cols-[minmax(0,1fr)_12rem_7rem]">
          <Labelled label={m.configuration_ruleset_url()}>
            <FieldInput
              aria-label={m.configuration_ruleset_url()}
              name="url"
              type="url"
              inputMode="url"
              autoComplete="off"
              autoCapitalize="none"
              spellCheck={false}
              placeholder={m.configuration_ruleset_url_placeholder()}
              readOnly={!editable}
              aria-invalid={refusedAt(notes, at("url")) || undefined}
              className="font-mono md:text-meta"
              value={set.url}
              onChange={(event) =>
                onChange({ ...set, url: event.target.value })
              }
            />
          </Labelled>
          <Labelled label={m.configuration_ruleset_format()}>
            <FieldSelect
              aria-label={m.configuration_ruleset_format()}
              name="format"
              disabled={!editable}
              value={set.format}
              onChange={(event) => {
                const format = RULE_SET_FORMATS.find(
                  (value) => value === event.target.value
                )
                if (format) onChange({ ...set, format })
              }}
              className="md:text-meta"
            >
              {RULE_SET_FORMATS.map((format) => (
                <FieldOption key={format} value={format}>
                  {FORMAT[format]()}
                </FieldOption>
              ))}
            </FieldSelect>
          </Labelled>
          <Labelled label={m.configuration_ruleset_interval()}>
            <FieldInput
              aria-label={m.configuration_ruleset_interval()}
              name="updateInterval"
              autoComplete="off"
              autoCapitalize="none"
              spellCheck={false}
              placeholder={DEFAULT_INTERVAL}
              readOnly={!editable}
              aria-invalid={refusedAt(notes, at("updateInterval")) || undefined}
              className="font-mono md:text-meta"
              value={set.updateInterval ?? ""}
              onChange={(event) => {
                const value = event.target.value.trim()
                const next = { ...set }
                if (value === "") delete next.updateInterval
                else next.updateInterval = value
                onChange(next)
              }}
            />
          </Labelled>
        </div>
      )}
      <Notes notes={notes} className="px-1.5" />
      <StatusBlock
        networkId={networkId}
        name={name}
        preparation={preparation}
        status={status}
        view={view}
        referenced={referenced}
        editable={editable}
        now={now}
      />
    </div>
  )
}

function Labelled({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex min-w-0 flex-col gap-1">
      <span aria-hidden="true" className="px-1 text-meta text-muted-foreground">
        {label}
      </span>
      {children}
    </div>
  )
}

/** Where the set's content stands and what comes next, then the controller's facts. */
function stepOf(preparation: Preparation): string {
  switch (preparation) {
    case "inline":
      return m.configuration_ruleset_step_inline()
    case "new":
      return m.configuration_ruleset_step_new()
    case "replaced":
      return m.configuration_ruleset_step_replaced()
    case "unread":
      return m.configuration_ruleset_step_unread()
    case "ready":
      return m.configuration_ruleset_step_ready()
    case "failing":
      return m.configuration_ruleset_step_failing()
    case "missing":
      return m.configuration_ruleset_step_missing()
  }
}

function StatusBlock({
  networkId,
  name,
  preparation,
  status,
  view,
  referenced,
  editable,
  now,
}: {
  networkId: string
  name: string
  preparation: Preparation
  status: RuleSetStatus
  view: RuleSetView | undefined
  referenced: boolean
  editable: boolean
  now: number
}) {
  const refresh = useRefreshRuleSet(networkId)
  const good = preparation === "ready" || preparation === "inline"
  const Icon = good ? CircleCheckIcon : CircleAlertIcon
  // A time read after the last tick reads as now, not as ahead of it.
  const when = (at: string) => agoTime(at, now) ?? dateTime(at) ?? at
  const facts: string[] = []
  if (view?.artifact)
    facts.push(
      m.configuration_ruleset_artifact({
        entries: view.artifact.entries,
        digest: view.artifact.digest.slice(0, 12),
        created: dateTime(view.artifact.createdAt) ?? view.artifact.createdAt,
      })
    )
  if (view) {
    facts.push(
      view.checkedAt
        ? m.configuration_ruleset_checked({ time: when(view.checkedAt) })
        : m.configuration_ruleset_never_checked()
    )
    if (view.lastSuccessAt)
      facts.push(
        m.configuration_ruleset_last_success({ time: when(view.lastSuccessAt) })
      )
    if (view.publication)
      facts.push(
        m.configuration_ruleset_publication({
          id: view.publication.slice(0, 8),
        })
      )
  }
  // A new source has no status of its own yet.
  const shownFacts = preparation === "new" ? [] : facts
  if (referenced) shownFacts.push(m.configuration_ruleset_referenced())
  return (
    <div className="flex flex-col gap-1 rounded-tile bg-fill-subtle px-3.5 py-2.5">
      <div className="flex flex-wrap items-start gap-x-3 gap-y-2">
        <p className="flex min-w-0 flex-[1_1_16rem] items-start gap-2 text-row">
          <Icon
            aria-hidden="true"
            className={cn(
              "mt-0.5 size-3.5 shrink-0",
              good ? "text-success" : "text-warning"
            )}
          />
          <span className="min-w-0 break-words">{stepOf(preparation)}</span>
        </p>
        {editable && refreshable(preparation) ? (
          <Button
            type="button"
            size="sm"
            variant="secondary"
            className="h-7 shrink-0 rounded-full px-3"
            disabled={refresh.isPending}
            onClick={() =>
              refresh.mutate(name, {
                onError: toastFailureWith({
                  state_conflict: () => m.configuration_ruleset_refresh_moved(),
                  state_limit: () => m.configuration_ruleset_refresh_busy(),
                  not_found: () => m.configuration_ruleset_refresh_gone(),
                }),
              })
            }
          >
            <BusyLabel busy={refresh.isPending}>
              <RefreshCwIcon />
              {m.configuration_ruleset_refresh()}
            </BusyLabel>
          </Button>
        ) : null}
      </div>
      {view?.problem && preparation !== "new" ? (
        <p
          className={cn(
            "pl-5.5 text-meta",
            view.problem === "not_ready"
              ? "text-muted-foreground"
              : "text-destructive"
          )}
        >
          {RESOURCE_PROBLEM[view.problem]()}
        </p>
      ) : null}
      {shownFacts.length > 0 ? (
        <p className="pl-5.5 text-meta text-muted-foreground tabular-nums">
          {shownFacts.join(" · ")}
        </p>
      ) : null}
      {status.kind === "failed" && preparation !== "inline" ? (
        <p className="pl-5.5 text-meta text-muted-foreground">
          {m.configuration_ruleset_status_failed()}
        </p>
      ) : null}
    </div>
  )
}
