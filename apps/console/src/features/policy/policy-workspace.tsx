import { CircleAlertIcon } from "lucide-react"
import { useState } from "react"

import { CloseButton } from "@/components/chrome/close-button"
import { RevisionLine } from "@/components/chrome/draft-mark"
import { SurfaceHeader } from "@/components/chrome/surface-header"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { useNodeNavigation } from "@/features/nodes/navigation"
import { surfaceTitleId } from "@/features/shell/page-frame"
import { PopupBoundary } from "@/features/shell/popup-boundary"
import { m } from "@/paraglide/messages.js"

import { cn } from "@/lib/utils"

import { ConnectionsLayer } from "./connections"
import { EgressLayer } from "./egress"
import { GrantsLayer } from "./grants"
import { lineColumn } from "./jsonc"
import type { LayerProps } from "./layer"
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
  const { data, document } = state
  const [body, setBody] = useState<HTMLDivElement | null>(null)

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
          <RevisionLine
            revision={data?.revision ?? null}
            draftBase={
              state.draft.dirty ? (state.draft.baseRevision ?? null) : null
            }
            facts={
              data && !state.canManage
                ? [<span key="read-only">{m.read_only()}</span>]
                : []
            }
          />
        }
        action={<CloseButton onClick={nav.closeLayer} />}
      />
      <PopupBoundary value={body}>
        <div
          ref={setBody}
          className={cn(
            "@container/workspace flex min-h-0 flex-1 flex-col gap-4 px-6",
            // The source editor ends above the commit bar, like every other layer's content.
            layer === "source"
              ? "pb-(--commit-space)"
              : "scroll-edge overflow-y-auto pt-1 pb-(--commit-space)"
          )}
        >
          {!data || !document ? (
            <Placeholder />
          ) : layer === "source" ? (
            <Source state={state} names={names} />
          ) : !document.policy ? (
            <InvalidSource
              text={document.text}
              problem={document.problems[0] ?? null}
              onOpenSource={() => nav.openLayer("source")}
            />
          ) : (
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
                  policy: document.policy,
                  editable: state.editable,
                  names,
                  diagnostics: state.diagnostics,
                  problems: document.problems,
                  rule: view.rule,
                  onRule: nav.setRule,
                  edit: state.edit,
                  move: state.move,
                  onOpenNode: (nodeId) => nodes.openNode(nodeId),
                }}
              />
            </form>
          )}
        </div>
      </PopupBoundary>
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
  if (!state.document) return null
  return (
    <SourceEditor
      document={state.document}
      diagnostics={state.diagnostics}
      names={names}
      editable={state.canManage}
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
