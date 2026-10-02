import { cn } from "@/lib/utils"
import { CircleAlertIcon, RouteIcon } from "lucide-react"
import { useMemo } from "react"

import { CloseButton } from "@/components/chrome/close-button"
import { SurfaceHeader } from "@/components/chrome/surface-header"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { useNodeNavigation } from "@/features/nodes/navigation"
import { surfaceTitleId } from "@/features/shell/page-frame"
import { m } from "@/paraglide/messages.js"

import { ConnectionsLayer } from "./connections"
import { policyText, readDocument } from "./document"
import { EgressLayer } from "./egress"
import { FlowPreview } from "./flow-preview"
import { GrantsLayer } from "./grants"
import { lineColumn } from "./jsonc"
import type { LayerProps } from "./layer"
import { LegacyNotice } from "./legacy"
import { MeshLayer } from "./mesh"
import type { PolicyNavigation } from "./navigation"
import { layerLabel } from "./policy-panel"
import { problemReason } from "./problems"
import { useSelectorNames } from "./selectors"
import { SourceEditor } from "./source"
import type { PolicyLayer, PolicyWorkspaceView } from "./url"
import { usePolicy, usePolicyCommit } from "./use-policy"

/**
 * One layer of the policy, or its source, edited in place. Every layer
 * edits the same draft (the JSONC text) against one revision, so a change
 * made in one is pending in all, and one commit bar saves it.
 */
export function PolicyWorkspace({
  networkId,
  layer,
  view,
  nav,
}: {
  networkId: string
  layer: PolicyLayer
  view: PolicyWorkspaceView
  nav: PolicyNavigation
}) {
  const state = usePolicy(networkId)
  const commit = usePolicyCommit(networkId, state)
  const names = useSelectorNames(networkId)
  const nodes = useNodeNavigation(networkId)
  const { data, document, shown, legacy } = state

  const origin =
    shown.origin === "proposal"
      ? m.policy_origin_proposal()
      : state.draft.dirty
        ? m.policy_origin_draft({ revision: state.draft.baseRevision ?? 0 })
        : data
          ? m.policy_revision({ revision: data.revision })
          : null

  return (
    <div className="swap-in flex min-h-0 flex-1 flex-col" key={layer}>
      <SurfaceHeader
        size="lg"
        className="px-6 pt-6 pb-4"
        title={(className) => (
          <h2 id={surfaceTitleId("policy", "workspace")} className={className}>
            {layerLabel[layer]()}
          </h2>
        )}
        meta={
          <p className="flex h-4 items-center gap-1.5 text-meta text-muted-foreground">
            {origin ? (
              <span
                className={cn(
                  "font-mono tabular-nums",
                  shown.origin === "proposal" && "text-warning"
                )}
              >
                {origin}
              </span>
            ) : (
              <Skeleton className="h-2.5 w-24 rounded-full" />
            )}
            {data ? (
              <>
                <span aria-hidden="true">·</span>
                <span>{legacy ? m.policy_mode_v2() : m.policy_mode_v3()}</span>
                {!state.canManage ? (
                  <>
                    <span aria-hidden="true">·</span>
                    <span>{m.policy_read_only()}</span>
                  </>
                ) : null}
              </>
            ) : null}
          </p>
        }
        action={
          <span className="flex items-center gap-1">
            {legacy ? (
              <Button
                type="button"
                variant="ghost"
                size="sm"
                aria-pressed={view.flow}
                className={cn(
                  "h-8 rounded-full px-3",
                  view.flow && "bg-fill-selected"
                )}
                onClick={() => nav.setFlow(!view.flow)}
              >
                <RouteIcon />
                {m.policy_flow_title()}
              </Button>
            ) : null}
            <CloseButton onClick={nav.closeLayer} />
          </span>
        }
      />
      <div
        className={cn(
          "@container/workspace flex min-h-0 flex-1 flex-col gap-4 px-6",
          layer === "source" ? "pb-6" : "overflow-y-auto pb-(--commit-space)"
        )}
      >
        {legacy && data ? (
          <LegacyNotice
            proposal={data.migration_proposal}
            diagnostics={data.diagnostics}
            started={document !== null}
            canManage={state.canManage}
            names={names}
            onStart={state.start}
          />
        ) : null}
        {legacy && view.flow ? (
          <FlowPreview networkId={networkId} names={names} />
        ) : null}
        {!data ? (
          <Placeholder />
        ) : layer === "source" ? (
          <Source state={state} names={names} />
        ) : document && !document.policy ? (
          <InvalidSource
            text={document.text}
            problem={document.problems[0] ?? null}
            onOpenSource={() => nav.openLayer("source")}
          />
        ) : shown.policy ? (
          <form
            className="flex flex-col"
            onSubmit={(event) => {
              event.preventDefault()
              commit.submit()
            }}
          >
            <button type="submit" hidden />
            <Layer
              layer={layer}
              view={view}
              nav={nav}
              props={{
                policy: shown.policy,
                editable: state.editable,
                names,
                diagnostics: state.diagnostics,
                problems: document?.problems ?? [],
                rule: view.rule,
                onRule: nav.setRule,
                edit: state.edit,
                move: state.move,
                onOpenNode: (nodeId) => nodes.openNode(nodeId),
              }}
            />
          </form>
        ) : null}
      </div>
    </div>
  )
}

function Layer({
  layer,
  view,
  nav,
  props,
}: {
  layer: Exclude<PolicyLayer, "source">
  view: PolicyWorkspaceView
  nav: PolicyNavigation
  props: LayerProps
}) {
  switch (layer) {
    case "connections":
      return <ConnectionsLayer {...props} />
    case "grants":
      return <GrantsLayer {...props} />
    case "mesh":
      return <MeshLayer {...props} />
    case "egress":
      return (
        <EgressLayer
          {...props}
          section={view.section}
          onSection={nav.setSection}
        />
      )
  }
}

function Source({
  state,
  names,
}: {
  state: ReturnType<typeof usePolicy>
  names: ReturnType<typeof useSelectorNames>
}) {
  const proposal = state.data?.migration_proposal ?? null
  // A v2 network shows the proposal's text, read-only, until it is put into the draft.
  const proposalDocument = useMemo(
    () => (proposal ? readDocument(policyText(proposal)) : null),
    [proposal]
  )
  const document = state.document ?? (state.legacy ? proposalDocument : null)
  if (!document) return null
  return (
    <SourceEditor
      key={state.document ? "draft" : "proposal"}
      document={document}
      diagnostics={state.document ? state.diagnostics : []}
      names={names}
      editable={state.canManage && state.document !== null}
      onChange={state.setText}
    />
  )
}

/** The text does not parse or does not match the schema: the graphical views wait for it to be fixed. */
function InvalidSource({
  text,
  problem,
  onOpenSource,
}: {
  text: string
  problem: Parameters<typeof problemReason>[0] | null
  onOpenSource: () => void
}) {
  return (
    <div className="flex min-h-14 items-center gap-3 rounded-row border border-destructive/30 bg-destructive/6 py-2.5 pr-2.5 pl-3.5">
      <CircleAlertIcon
        aria-hidden="true"
        className="size-4 shrink-0 text-destructive"
      />
      <span className="min-w-0 flex-1">
        <span className="block text-row font-medium">
          {m.policy_source_invalid()}
        </span>
        {problem ? (
          <span className="block truncate text-meta text-muted-foreground">
            {m.policy_problem_at({
              line: lineColumn(text, problem.from).line,
              reason: problemReason(problem),
            })}
          </span>
        ) : null}
      </span>
      <Button
        type="button"
        size="sm"
        variant="secondary"
        className="h-8 shrink-0 rounded-full px-3.5"
        onClick={onOpenSource}
      >
        {m.policy_source_open()}
      </Button>
    </div>
  )
}

function Placeholder() {
  return (
    <div className="flex flex-col gap-2">
      <Skeleton className="h-6 w-48 rounded-full" />
      <Skeleton className="h-14 w-full rounded-row" />
      <Skeleton className="h-14 w-full rounded-row" />
    </div>
  )
}
