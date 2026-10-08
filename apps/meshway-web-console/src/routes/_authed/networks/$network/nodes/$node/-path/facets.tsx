import type { Evaluation } from "@meshway/control-client"
import { cn } from "@meshway/ui/cn"
import type { ReactNode } from "react"

import { profileText, type NodeNamer } from "@/lib/configuration/diagnostics"
import {
  ADDRESS_CLASS,
  ruleText,
  TARGET_PROBLEM,
  UNAVAILABLE_REASON,
} from "@/lib/configuration/evaluation"
import { canonical } from "@/lib/configuration/operations"
import { m } from "@/paraglide/messages.js"

// One evaluation, part by part, for the path tab's rows. Each part is
// drawn from the controller's answer alone; rules are named from the
// configuration it was asked over.

export type FacetContext = {
  /** A Node ID as its hostname. */
  nameOf: NodeNamer
  /** The configuration asked over, for naming its rules. */
  rules: unknown
  /** Opens a rule in the source; null where the source is not this configuration. */
  openRule: ((pointer: string) => void) | null
}

type Facet = {
  key:
    | "outcome"
    | "destination"
    | "profile"
    | "access"
    | "route"
    | "targets"
    | "paths"
  label: () => string
  /** What two evaluations compare by. */
  of: (evaluation: Evaluation) => unknown
  render: (evaluation: Evaluation, context: FacetContext) => ReactNode
}

const muted = (text: string) => (
  <span className="text-muted-foreground">{text}</span>
)

function Rule({
  pointer,
  context,
}: {
  pointer: string
  context: FacetContext
}) {
  const { name, remark } = ruleText(pointer, context.rules, context.nameOf)
  const label = (
    <>
      {name}
      {remark ? (
        <span className="font-normal text-muted-foreground"> · {remark}</span>
      ) : null}
    </>
  )
  const open = context.openRule
  return open ? (
    <button
      type="button"
      className="cursor-pointer text-left font-medium underline-offset-2 hover:underline"
      onClick={() => open(pointer)}
    >
      {label}
    </button>
  ) : (
    <span className="font-medium">{label}</span>
  )
}

const names = (ids: readonly string[], context: FacetContext) =>
  ids.map(context.nameOf).join(", ")

function Note({ children }: { children: ReactNode }) {
  return <p className="text-meta text-muted-foreground">{children}</p>
}

export const FACETS: readonly Facet[] = [
  {
    key: "outcome",
    label: () => m.nodes_path_outcome(),
    of: (evaluation) => evaluation.outcome,
    render: ({ outcome }) => {
      switch (outcome.type) {
        case "permitted":
          return (
            <span className="font-medium text-success">
              {m.nodes_path_outcome_permitted()}
            </span>
          )
        case "direct":
          return (
            <span className="font-medium">{m.nodes_path_outcome_direct()}</span>
          )
        case "unmanaged":
          return (
            <span className="font-medium">
              {m.nodes_path_outcome_unmanaged()}
            </span>
          )
        case "unavailable":
          return (
            <span className="font-medium text-destructive">
              {m.nodes_path_outcome_unavailable({
                reason: UNAVAILABLE_REASON[outcome.reason](),
              })}
            </span>
          )
      }
    },
  },
  {
    key: "destination",
    label: () => m.nodes_path_destination(),
    of: (evaluation) => evaluation.destination,
    render: ({ destination }, context) => {
      switch (destination.type) {
        case "node":
          return context.nameOf(destination.nodeId)
        case "address":
          return (
            <>
              <span className="font-mono">{destination.address}</span>
              {muted(` · ${ADDRESS_CLASS[destination.class]()}`)}
            </>
          )
        case "domain":
          return <span className="font-mono">{destination.name}</span>
        case "internet":
          return m.nodes_path_internet()
      }
    },
  },
  {
    key: "profile",
    label: () => m.nodes_path_profile(),
    of: (evaluation) => evaluation.profile,
    render: ({ profile }, context) => (
      <div className="flex flex-col gap-0.5">
        <span className="font-medium">
          {profileText(profile.name, context.nameOf)}
        </span>
        <Note>
          {profile.assignment ? (
            <>
              {m.nodes_path_set_by()}{" "}
              <Rule pointer={profile.assignment} context={context} />
            </>
          ) : (
            m.nodes_path_profile_default()
          )}
          {profile.locked ? ` · ${m.nodes_path_profile_locked()}` : null}
          {profile.requested ? ` · ${m.nodes_path_profile_requested()}` : null}
        </Note>
      </div>
    ),
  },
  {
    key: "access",
    label: () => m.nodes_path_access(),
    of: (evaluation) => evaluation.access,
    render: ({ access }, context) => (
      <div className="flex flex-col gap-1.5">
        <span
          className={cn(
            "font-medium",
            access.allowed ? "text-success" : "text-destructive"
          )}
        >
          {access.allowed
            ? m.nodes_path_access_allowed()
            : access.grants.length === 0
              ? m.nodes_path_access_default()
              : m.nodes_path_access_resolution()}
        </span>
        {access.grants.length > 0 ? (
          <ul className="flex flex-col gap-1">
            {access.grants.map((grant) => (
              <li key={grant.rule} className="flex flex-col">
                <Rule pointer={grant.rule} context={context} />
                <Note>
                  <span className="font-mono">{grant.traffic.join(", ")}</span>
                  {grant.reverse ? ` · ${m.nodes_path_reverse()}` : null}
                  {grant.resolutionDependent
                    ? ` · ${m.nodes_path_resolution()}`
                    : null}
                </Note>
              </li>
            ))}
          </ul>
        ) : null}
      </div>
    ),
  },
  {
    key: "route",
    label: () => m.nodes_path_route(),
    of: (evaluation) => evaluation.route,
    render: ({ route }, context) => {
      if (!route) return muted(m.nodes_path_route_none())
      const profile = route.profile
        ? profileText(route.profile, context.nameOf)
        : null
      return (
        <div className="flex flex-col gap-1">
          <span className="font-medium">
            {{
              mesh: m.nodes_path_route_mesh,
              exit: m.nodes_path_route_exit,
              direct: m.nodes_path_route_direct,
              unmanaged: m.nodes_path_route_unmanaged,
            }[route.kind]()}
          </span>
          <Note>
            {route.rule ? (
              <>
                {m.nodes_path_set_by()}{" "}
                <Rule pointer={route.rule} context={context} />
              </>
            ) : route.kind === "mesh" ? (
              m.nodes_path_route_automatic()
            ) : profile ? (
              m.nodes_path_route_by_profile({ profile })
            ) : null}
          </Note>
          {route.ruleSet ? (
            <Note>
              {m.nodes_path_route_rule_set({
                key: route.ruleSet.key,
                entry: route.ruleSet.entry,
              })}
            </Note>
          ) : null}
          {route.kind === "exit" && route.targets.length > 0 ? (
            <Note>
              {m.nodes_path_route_exits({
                nodes: names(route.targets, context),
              })}
            </Note>
          ) : null}
          {route.via.length > 0 ? (
            <Note>
              {m.nodes_path_route_via({ nodes: names(route.via, context) })}
            </Note>
          ) : null}
          {route.resolutionDependent.length > 0 ? (
            <div className="flex flex-col gap-0.5">
              <Note>{m.nodes_path_route_skipped()}</Note>
              <ul className="flex flex-col gap-0.5 pl-3">
                {route.resolutionDependent.map((pointer) => (
                  <li key={pointer} className="text-meta">
                    <Rule pointer={pointer} context={context} />
                  </li>
                ))}
              </ul>
            </div>
          ) : null}
        </div>
      )
    },
  },
  {
    key: "targets",
    label: () => m.nodes_path_targets(),
    of: (evaluation) => evaluation.targets,
    render: ({ targets }, context) =>
      targets.length === 0 ? (
        muted(m.nodes_path_targets_none())
      ) : (
        <ul className="flex flex-col gap-0.5">
          {targets.map((target) => (
            <li key={target.nodeId} className="flex items-baseline gap-2">
              <span className="font-medium">
                {context.nameOf(target.nodeId)}
              </span>
              {target.problem ? (
                <span className="text-meta text-destructive">
                  {TARGET_PROBLEM[target.problem]()}
                </span>
              ) : (
                <span className="text-meta text-muted-foreground">
                  {m.nodes_path_target_paths({ count: target.paths })}
                </span>
              )}
            </li>
          ))}
        </ul>
      ),
  },
  {
    key: "paths",
    label: () => m.nodes_path_paths(),
    of: (evaluation) => evaluation.paths,
    render: ({ paths }, context) =>
      paths.length === 0 ? (
        muted(m.nodes_path_paths_none())
      ) : (
        <ol className="flex flex-col gap-2">
          {paths.map((path, index) => (
            <li key={path.nodes.join(">")} className="flex flex-col gap-0.5">
              <span className="text-meta text-muted-foreground">
                {index === 0
                  ? m.nodes_path_primary()
                  : m.nodes_path_backup({ index })}
              </span>
              <span className="font-medium">
                {path.nodes.map(context.nameOf).join(" → ")}
              </span>
              {path.edges.map((edge) => (
                <Note key={`${edge.a}>${edge.b}`}>
                  {context.nameOf(edge.a)} → {context.nameOf(edge.b)}
                  {": "}
                  {edge.topology ? (
                    <Rule pointer={edge.topology} context={context} />
                  ) : (
                    m.nodes_path_edge_default()
                  )}
                </Note>
              ))}
            </li>
          ))}
        </ol>
      ),
  },
]

/** Whether two evaluations differ in `facet`. */
export const differs = (facet: Facet, a: Evaluation, b: Evaluation) =>
  canonical(facet.of(a)) !== canonical(facet.of(b))
