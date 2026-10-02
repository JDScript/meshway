import type { NetworkPolicy } from "@meshway/control-client"
import { cn } from "@/lib/utils"
import {
  ArrowLeftRightIcon,
  BracesIcon,
  ChevronRightIcon,
  KeyRoundIcon,
  LogOutIcon,
  WaypointsIcon,
  type LucideIcon,
} from "lucide-react"

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

const layerIcon: Record<PolicyLayer, LucideIcon> = {
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

  const sourceSummary = () => {
    if (!document) return ""
    const problems = document.problems.length
    return problems > 0
      ? `${m.policy_count_lines({ count: lines })} · ${m.policy_count_problems({ count: problems })}`
      : `${m.policy_count_lines({ count: lines })} · ${m.policy_source_valid()}`
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex flex-col gap-1 px-4 pt-4 pb-3">
        <h2
          id={surfaceTitleId("policy", "panel")}
          className="text-base font-semibold tracking-tight"
        >
          {m.policy_nav()}
        </h2>
        <p className="flex h-4 items-center gap-1.5 text-meta text-muted-foreground">
          {data ? (
            <>
              <span className="font-mono tabular-nums">
                {m.policy_revision({ revision: data.revision })}
              </span>
              {!data.valid ? (
                <>
                  <span aria-hidden="true">·</span>
                  <span className="text-destructive">
                    {m.policy_saved_invalid()}
                  </span>
                </>
              ) : null}
            </>
          ) : (
            <Skeleton className="h-2.5 w-28 rounded-full" />
          )}
        </p>
      </header>
      <ul
        aria-labelledby={surfaceTitleId("policy", "panel")}
        className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto px-2 pb-2"
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
          const text =
            layer === "source"
              ? data
                ? sourceSummary()
                : null
              : document?.policy
                ? summary(layer, document.policy)
                : data
                  ? document
                    ? m.policy_source_invalid_short()
                    : m.policy_source_none()
                  : null
          return (
            <li
              key={layer}
              className={cn(
                layer === "source" && "mt-1.5 border-t border-divider pt-1.5"
              )}
            >
              <button
                type="button"
                aria-current={layer === openLayer ? "true" : undefined}
                onClick={() => onOpen(layer)}
                className="list-row h-14 gap-3 px-3"
              >
                <span className="flex size-8 shrink-0 items-center justify-center rounded-full border border-divider bg-fill-raised/50">
                  <Icon aria-hidden="true" className="size-3.5" />
                </span>
                <span className="min-w-0 flex-1">
                  <span className="flex items-center gap-1.5 text-row font-medium">
                    <span className="truncate">{layerLabel[layer]()}</span>
                    {changed ? (
                      <span
                        className="size-1.5 shrink-0 rounded-full bg-warning"
                        aria-label={m.policy_changed()}
                      />
                    ) : null}
                  </span>
                  {text !== null ? (
                    <span className="block truncate text-meta text-muted-foreground">
                      {text}
                    </span>
                  ) : (
                    <Skeleton className="mt-1.5 h-2.5 w-24 rounded-full" />
                  )}
                </span>
                <NoteCounts notes={notes} />
                <ChevronRightIcon className="size-4 shrink-0 text-muted-foreground" />
              </button>
            </li>
          )
        })}
      </ul>
    </div>
  )
}
