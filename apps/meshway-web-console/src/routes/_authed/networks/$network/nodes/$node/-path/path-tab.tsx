import {
  evaluateConfiguration,
  type EvaluationConfiguration,
  type EvaluationResponse,
  type EvaluationTarget,
  type NetworkSummary,
  type NodeInventoryResponse,
} from "@meshway/control-client"
import { useQuery } from "@tanstack/react-query"
import { useNavigate } from "@tanstack/react-router"
import { Fragment, useId, useRef, useState, type FormEvent } from "react"
import { toast } from "sonner"

import { authed, client } from "@/api/client"
import { failureText, type FailureText } from "@/api/failure"
import { configurationQuery } from "@/api/queries/configuration"
import { BusyLabel } from "@/components/chrome/busy-label"
import { FieldInput } from "@/components/chrome/form"
import { Section } from "@/components/chrome/section"
import { SegmentedControl } from "@/components/chrome/segmented-control"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { profileText } from "@/lib/configuration/diagnostics"
import { isDirty, type DraftInput } from "@/lib/configuration/draft-state"
import {
  draftConfiguration,
  draftRules,
  targetOf,
} from "@/lib/configuration/evaluation"
import { shortId } from "@/lib/format"
import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"
import { useConfigurationDraft } from "@/state/configuration-draft"

import { differs, FACETS, type FacetContext } from "./facets"

const TOAST = "path-check"

/** What the controller's refusals of a check mean here. */
const CHECK_FAILURE: FailureText = {
  invalid_request: () => m.nodes_path_refused(),
  revision_conflict: () => m.nodes_path_draft_outdated(),
  configuration_invalid: () => m.nodes_path_draft_invalid(),
  state_limit: () => m.nodes_path_limit(),
}

type Column =
  | { status: "pending" }
  | { status: "done"; response: EvaluationResponse }
  | { status: "failed"; text: string }

/** One check: what was asked, over the current version and, with a draft, over it too. */
type Check = {
  run: number
  target: EvaluationTarget
  profile: string | null
  /** The draft it was asked over; null without one. */
  draft: { input: DraftInput; baseRevision: number } | null
  current: Column
  over: Column | null
}

/**
 * The path tab (D6): from this node to a destination typed as a node's
 * hostname, an IP address or a domain, or a public destination no rule
 * matches. The controller plans it over the current version and, while a
 * draft holds edits, over the draft too, side by side with the parts that
 * differ marked. A plan, not a measurement; protocols and ports are not
 * asked. Owners and admins only (`readConfiguration`).
 */
export function PathTab({
  networkId,
  nodeId,
  network,
  nodes,
}: {
  networkId: string
  nodeId: string
  network: NetworkSummary | null
  nodes: NodeInventoryResponse | undefined
}) {
  const id = useId()
  const navigate = useNavigate()
  const configuration = useQuery(configurationQuery(networkId))
  const { entry } = useConfigurationDraft(networkId)
  const [typed, setTyped] = useState("")
  const [check, setCheck] = useState<Check | null>(null)
  const runs = useRef(0)
  const listed = nodes?.items ?? []
  const nameOf = (value: string) =>
    listed.find((node) => node.id === value)?.hostname ?? shortId(value)
  const currentRevision =
    configuration.data?.revision ?? network?.currentRevision ?? null

  const ask = (
    configuration: EvaluationConfiguration,
    target: EvaluationTarget,
    profile: string | null
  ) =>
    authed((token) =>
      evaluateConfiguration(
        client,
        { networkId },
        {
          configuration,
          source: nodeId,
          target,
          ...(profile === null ? {} : { profile }),
        },
        { token }
      )
    )

  const start = (target: EvaluationTarget, profile: string | null) => {
    runs.current += 1
    const run = runs.current
    const draft =
      entry && isDirty(entry)
        ? { input: entry.input, baseRevision: entry.baseRevision }
        : null
    setCheck({
      run,
      target,
      profile,
      draft,
      current: { status: "pending" },
      over: draft ? { status: "pending" } : null,
    })
    const settle = (
      column: "current" | "over",
      promise: Promise<EvaluationResponse>
    ) =>
      promise
        .then(
          (response) => ({ status: "done" as const, response }),
          (error: unknown) => ({
            status: "failed" as const,
            text: failureText(error, CHECK_FAILURE) ?? m.nodes_path_failed(),
          })
        )
        .then((settled) =>
          setCheck((now) =>
            now && now.run === run ? { ...now, [column]: settled } : now
          )
        )
    void settle("current", ask({ type: "current" }, target, profile))
    if (draft)
      void settle(
        "over",
        ask(
          draftConfiguration(draft.baseRevision, draft.input),
          target,
          profile
        )
      )
  }

  const submit = (event: FormEvent) => {
    event.preventDefault()
    const target = targetOf(typed, listed)
    if (!target) {
      toast.error(m.nodes_path_destination_unknown(), { id: TOAST })
      return
    }
    start(target, null)
  }

  const busy =
    check !== null &&
    (check.current.status === "pending" || check.over?.status === "pending")
  const shown = check?.current.status === "done" ? check.current.response : null
  const openRule = (pointer: string) =>
    void navigate({
      to: "/networks/$network/configuration/$view",
      params: { network: networkId, view: "source" },
      search: { at: pointer },
    })
  const contexts: FacetContext[] = [
    {
      nameOf,
      rules: configuration.data?.configuration ?? null,
      openRule,
    },
    ...(check?.draft
      ? [{ nameOf, rules: draftRules(check.draft.input), openRule: null }]
      : []),
  ]
  const columns: (Column | null)[] = check
    ? [check.current, ...(check.over ? [check.over] : [])]
    : []
  const evaluations = columns.map((column) =>
    column?.status === "done" ? column.response.evaluation : null
  )
  const [base, other] = evaluations
  // Profiles are offered once the controller has said which are allowed.
  const allowed = shown?.evaluation.profile.allowed ?? []

  return (
    <div className="flex flex-col gap-8">
      <Section title={m.nodes_path_destination()}>
        <form className="flex flex-col gap-3" onSubmit={submit}>
          <div className="flex flex-wrap items-center gap-2">
            <FieldInput
              aria-label={m.nodes_path_destination()}
              name="destination"
              list={`${id}-nodes`}
              autoComplete="off"
              spellCheck={false}
              placeholder={m.nodes_path_destination_placeholder()}
              value={typed}
              onChange={(event) => setTyped(event.target.value)}
              className="min-w-0 flex-[1_1_16rem]"
            />
            <datalist id={`${id}-nodes`}>
              {listed
                .filter((node) => node.id !== nodeId)
                .map((node) => (
                  <option key={node.id} value={node.hostname} />
                ))}
            </datalist>
            <Button type="submit" disabled={busy}>
              <BusyLabel busy={busy}>{m.nodes_path_check()}</BusyLabel>
            </Button>
          </div>
          <div className="flex flex-wrap items-center justify-between gap-3">
            <p className="max-w-prose text-meta text-muted-foreground">
              {m.nodes_path_hint()}
            </p>
            <Button
              type="button"
              variant="secondary"
              size="sm"
              disabled={busy}
              onClick={() => start({ type: "internet" }, null)}
            >
              {m.nodes_path_internet()}
            </Button>
          </div>
        </form>
      </Section>

      {check ? (
        <Section
          title={m.nodes_path_result()}
          action={
            allowed.length > 1 ? (
              <SegmentedControl
                size="sm"
                label={m.nodes_path_profile()}
                value={shown?.evaluation.profile.name ?? ""}
                options={allowed.map((name) => ({
                  value: name,
                  label: profileText(name, nameOf),
                }))}
                onChange={(name) => start(check.target, name)}
              />
            ) : null
          }
        >
          <div
            className={cn(
              "grid items-start gap-x-2 gap-y-2",
              columns.length > 1
                ? "grid-cols-[6rem_minmax(0,1fr)_minmax(0,1fr)]"
                : "grid-cols-[6rem_minmax(0,1fr)]"
            )}
          >
            <span />
            <ColumnHead
              title={m.nodes_path_current()}
              column={check.current}
              note={
                check.current.status === "done" &&
                currentRevision !== null &&
                check.current.response.revision !== currentRevision
                  ? m.nodes_path_stale_revision({
                      revision: check.current.response.revision,
                    })
                  : m.nodes_path_at({
                      revision:
                        check.current.status === "done"
                          ? check.current.response.revision
                          : (currentRevision ?? 0),
                    })
              }
            />
            {check.over ? (
              <ColumnHead
                title={m.nodes_path_draft()}
                column={check.over}
                note={
                  entry?.input !== check.draft?.input
                    ? m.nodes_path_stale_draft()
                    : m.nodes_path_draft_base({
                        revision: check.draft?.baseRevision ?? 0,
                      })
                }
              />
            ) : null}
            {FACETS.map((facet) => (
              <Fragment key={facet.key}>
                <span className="pt-2.5 text-meta text-muted-foreground">
                  {facet.label()}
                </span>
                {columns.map((column, index) => {
                  const evaluation = evaluations[index] ?? null
                  const context = contexts[index]
                  const changed =
                    index === 1 &&
                    base != null &&
                    other != null &&
                    differs(facet, base, other)
                  return (
                    <div
                      key={index}
                      className={cn(
                        "min-w-0 rounded-tile bg-fill-subtle px-3.5 py-2.5 text-row break-words",
                        changed && "ring-1 ring-warning/60"
                      )}
                    >
                      {evaluation && context ? (
                        facet.render(evaluation, context)
                      ) : column?.status === "pending" ? (
                        <Skeleton className="h-4 w-2/3 rounded-full" />
                      ) : (
                        <span className="text-muted-foreground">—</span>
                      )}
                      {changed ? (
                        <p className="mt-1 text-meta text-warning">
                          {m.nodes_path_differs()}
                        </p>
                      ) : null}
                    </div>
                  )
                })}
              </Fragment>
            ))}
          </div>
          {check.current.status === "done" || check.over?.status === "done" ? (
            <div className="flex justify-end">
              <Button
                type="button"
                variant="ghost"
                size="sm"
                disabled={busy}
                onClick={() => start(check.target, check.profile)}
              >
                {m.nodes_path_again()}
              </Button>
            </div>
          ) : null}
        </Section>
      ) : null}
    </div>
  )
}

function ColumnHead({
  title,
  column,
  note,
}: {
  title: string
  column: Column
  note: string
}) {
  return (
    <div className="flex min-w-0 flex-col px-1">
      <span className="text-row font-medium">{title}</span>
      {column.status === "failed" ? (
        <span className="text-meta text-destructive">{column.text}</span>
      ) : (
        <span className="text-meta text-muted-foreground">{note}</span>
      )}
    </div>
  )
}
