import type { NetworkPolicy } from "@meshway/control-client"
import {
  ArrowLeftRightIcon,
  BracesIcon,
  KeyRoundIcon,
  LogOutIcon,
  WaypointsIcon,
  type LucideIcon,
} from "lucide-react"
import { Fragment } from "react"

import { DraftMark, RevisionLine } from "@/components/chrome/draft-mark"
import { ListRow, PanelList, useListKeys } from "@/components/chrome/list"
import { PanelHeader } from "@/components/chrome/panel-header"
import { Skeleton } from "@/components/ui/skeleton"
import { surfaceTitleId } from "@/features/shell/page-frame"
import { m } from "@/paraglide/messages.js"

import type { PlacedDiagnostic } from "./document"
import { NoteCounts } from "./rules"
import { POLICY_LAYERS, type PolicyLayer } from "./url"
import type { PolicyState } from "./use-policy"

export const layerLabel: Record<PolicyLayer, () => string> = {
  connections: () => m.policy_layer_connections(),
  grants: () => m.policy_layer_grants(),
  mesh: () => m.policy_layer_mesh(),
  egress: () => m.policy_layer_egress(),
  source: () => m.policy_layer_source(),
}

export const layerIcon: Record<PolicyLayer, LucideIcon> = {
  connections: ArrowLeftRightIcon,
  grants: KeyRoundIcon,
  mesh: WaypointsIcon,
  egress: LogOutIcon,
  source: BracesIcon,
}

const EGRESS_SECTIONS = new Set([
  "public_rules",
  "profiles",
  "automatic",
  "assignments",
])

/** The layer a controller diagnostic is about; the source holds the rest. */
function diagnosticLayer(diagnostic: PlacedDiagnostic): PolicyLayer {
  if (diagnostic.placed.kind === "path") return "mesh"
  if (diagnostic.placed.kind === "general") return "source"
  const section = diagnostic.placed.anchor[0]
  if (section === "connections" || section === "grants" || section === "mesh")
    return section
  if (typeof section === "string" && EGRESS_SECTIONS.has(section))
    return "egress"
  return "source"
}

function summary(layer: PolicyLayer, policy: NetworkPolicy): string {
  switch (layer) {
    case "connections":
      return m.policy_count_rules({ count: policy.connections.length })
    case "grants":
      return m.policy_count_rules({ count: policy.grants.length })
    case "mesh":
      return m.policy_count_rules({ count: policy.mesh.length })
    case "egress":
      return m.policy_egress_summary({
        public: policy.public_rules.length,
        profiles: policy.profiles.length,
        assignments: policy.assignments.length,
      })
    case "source":
      return ""
  }
}

const CHANGED: Record<PolicyLayer, readonly string[]> = {
  connections: ["connections"],
  grants: ["grants"],
  mesh: ["mesh"],
  egress: ["public_rules", "profiles", "automatic", "assignments"],
  source: [],
}

/** The policy page's index: one row per layer with its rules and the controller's notes. */
export function PolicyPanel({
  state,
  openLayer,
  onOpen,
}: {
  state: PolicyState
  openLayer: PolicyLayer | null
  onOpen: (layer: PolicyLayer) => void
}) {
  const { data, document, diagnostics } = state
  const lines = state.text === "" ? 0 : state.text.split("\n").length
  const { ref, onKeyDown } = useListKeys({
    ids: POLICY_LAYERS,
    openId: openLayer,
    onOpen: (id) => {
      const layer = POLICY_LAYERS.find((candidate) => candidate === id)
      if (layer) onOpen(layer)
    },
  })

  const sourceSummary = () => {
    if (!document) return ""
    const problems = document.problems.length
    return problems > 0
      ? `${m.policy_count_lines({ count: lines })} · ${m.policy_count_problems({ count: problems })}`
      : `${m.policy_count_lines({ count: lines })} · ${m.policy_source_valid()}`
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <PanelHeader page="policy" title={m.policy_nav()}>
        <RevisionLine
          revision={data?.revision ?? null}
          draftBase={null}
          facts={
            data && !data.valid
              ? [
                  <span key="invalid" className="text-destructive">
                    {m.policy_saved_invalid()}
                  </span>,
                ]
              : []
          }
        />
      </PanelHeader>
      <PanelList
        ref={ref}
        aria-labelledby={surfaceTitleId("policy", "panel")}
        onKeyDown={onKeyDown}
      >
        {POLICY_LAYERS.map((layer) => {
          const Icon = layerIcon[layer]
          // The controller's notes are about what is saved: while the draft's source is invalid they are set aside.
          const notes =
            document && !document.policy
              ? []
              : diagnostics.filter(
                  (diagnostic) => diagnosticLayer(diagnostic) === layer
                )
          const changed =
            layer === "source"
              ? state.draft.dirty
              : CHANGED[layer].some((section) => state.changed.has(section))
          const text = !data
            ? null
            : layer === "source"
              ? sourceSummary()
              : document?.policy
                ? summary(layer, document.policy)
                : document
                  ? m.policy_source_invalid_short()
                  : m.policy_source_none()
          return (
            <Fragment key={layer}>
              {layer === "source" ? (
                <li aria-hidden="true" className="mx-3 my-1 h-px bg-divider" />
              ) : null}
              <ListRow
                rowId={layer}
                current={layer === openLayer}
                onClick={() => onOpen(layer)}
                lead={
                  <Icon
                    aria-hidden="true"
                    className="size-4 text-muted-foreground"
                  />
                }
                title={
                  <>
                    <span className="truncate">{layerLabel[layer]()}</span>
                    {changed ? <DraftMark /> : null}
                  </>
                }
                subtitle={
                  text !== null ? (
                    <span className="truncate">{text}</span>
                  ) : (
                    <Skeleton className="h-2.5 w-24 rounded-full" />
                  )
                }
                trail={<NoteCounts notes={notes} />}
              />
            </Fragment>
          )
        })}
      </PanelList>
    </div>
  )
}
