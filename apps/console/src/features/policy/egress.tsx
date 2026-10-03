import type {
  ExitMatch,
  ExitRule,
  ProfileAssignment,
  ProfileFallback,
} from "@meshway/control-client"
import { cn } from "@/lib/utils"
import {
  ArrowRightIcon,
  AtSignIcon,
  ListIcon,
  LockIcon,
  NetworkIcon,
  type LucideIcon,
} from "lucide-react"

import { EmptyLine } from "@/components/chrome/section"
import { SegmentedControl } from "@/components/chrome/segmented-control"
import { m } from "@/paraglide/messages.js"

import { notesOn, type LayerProps } from "./layer"
import { HopChain } from "./mesh"
import type { PlacedDiagnostic } from "./document"
import {
  Fact,
  LayerFacts,
  NoteCounts,
  OrderedRules,
  RuleIdInput,
  RuleNotes,
  RuleRow,
} from "./rules"
import { SelectorChip, type SelectorNames } from "./selectors"
import { EGRESS_SECTIONS, type EgressSection } from "./url"

// Routing out of the mesh. The controller does not save these yet (a save
// with any of them is refused), so they are shown as the source holds them
// and changed only in the source.

export const egressLabel: Record<EgressSection, () => string> = {
  public: () => m.policy_egress_public(),
  profiles: () => m.policy_egress_profiles(),
  automatic: () => m.policy_egress_automatic(),
  assignments: () => m.policy_egress_assignments(),
}

const MATCH: Record<ExitMatch["kind"], LucideIcon> = {
  domain_suffix: AtSignIcon,
  cidr: NetworkIcon,
  rule_set: ListIcon,
}

function Destination({ match }: { match: ExitMatch }) {
  const Icon = MATCH[match.kind]
  return (
    <span className="inline-flex h-7 min-w-0 items-center gap-1.5 rounded-full border border-divider bg-fill-raised/60 px-2 text-meta">
      <Icon
        aria-hidden="true"
        className="size-3 shrink-0 text-muted-foreground"
      />
      <span className="truncate font-mono">
        {match.kind === "rule_set"
          ? m.policy_egress_rule_set({ name: match.value })
          : match.value}
      </span>
    </span>
  )
}

/** Exit candidates in order: the first is primary, the rest stand by. */
function Candidates({
  candidates,
  names,
}: {
  candidates: readonly string[]
  names: SelectorNames
}) {
  return (
    <span className="inline-flex min-w-0 flex-wrap items-center gap-1">
      {candidates.map((candidate, index) => (
        <span
          key={`${candidate}-${index}`}
          className="inline-flex items-center gap-1"
        >
          {index > 0 ? (
            <span className="text-meta text-muted-foreground">
              {m.policy_egress_then()}
            </span>
          ) : null}
          <SelectorChip selector={candidate} names={names} />
        </span>
      ))}
    </span>
  )
}

/** Where a matched flow goes. */
function Target({
  target,
  candidates,
  via,
  names,
}: {
  target: "nodes" | "direct" | "none"
  candidates: readonly string[]
  via: readonly string[]
  names: SelectorNames
}) {
  if (target === "direct")
    return (
      <span className="text-row font-medium">{m.policy_egress_direct()}</span>
    )
  if (target === "none")
    return (
      <span className="text-row font-medium">{m.policy_egress_none()}</span>
    )
  return (
    <span className="flex min-w-0 flex-col gap-1">
      <span className="flex min-w-0 flex-wrap items-center gap-1.5">
        <span className="text-meta text-muted-foreground">
          {m.policy_egress_exit()}
        </span>
        <Candidates candidates={candidates} names={names} />
      </span>
      {via.length > 0 ? <HopChain via={via} names={names} /> : null}
    </span>
  )
}

function fallbackTarget(fallback: ProfileFallback) {
  return fallback.kind === "nodes"
    ? {
        target: "nodes" as const,
        candidates: fallback.candidates,
        via: fallback.via,
      }
    : { target: fallback.kind, candidates: [], via: [] }
}

/** One exit rule: a rule row like every layer's, read-only (the controller does not take egress yet). */
function ExitCard({
  rule,
  names,
  notes,
}: {
  rule: ExitRule
  names: SelectorNames
  notes: readonly PlacedDiagnostic[]
}) {
  return (
    <RuleRow
      current={false}
      label={rule.id}
      notes={notes}
      names={names}
      end={
        <>
          <NoteCounts notes={notes} />
          <RuleIdInput
            value={rule.id}
            invalid={false}
            editable={false}
            onChange={() => undefined}
          />
        </>
      }
    >
      <SelectorChip selector={rule.src ?? "*"} names={names} className="h-7" />
      <ArrowRightIcon
        aria-hidden="true"
        className="size-3.5 shrink-0 text-muted-foreground"
      />
      {rule.destinations.map((match, index) => (
        <Destination
          key={`${match.kind}-${match.value}-${index}`}
          match={match}
        />
      ))}
      {/* Where it goes, on its own line under what it matches. */}
      <span className="basis-full">
        <Target
          target={rule.target}
          candidates={rule.candidates}
          via={rule.via}
          names={names}
        />
      </span>
    </RuleRow>
  )
}

export function profileName(id: string, names: SelectorNames): string {
  if (id === "mesh-only") return m.policy_profile_mesh_only()
  if (id === "auto") return m.policy_profile_auto()
  if (id === "global:*") return m.policy_profile_global_all()
  if (id.startsWith("global:")) {
    const node = names.node(id.slice(7))
    return m.policy_profile_global({ name: node?.name ?? id.slice(7, 15) })
  }
  return id
}

export function EgressLayer({
  section,
  onSection,
  ...props
}: LayerProps & {
  section: EgressSection
  onSection: (section: EgressSection) => void
}) {
  const { policy, names, diagnostics } = props
  const counts: Record<EgressSection, number> = {
    public: policy.public_rules.length,
    profiles: policy.profiles.length,
    automatic: Object.keys(policy.automatic?.regions ?? {}).length,
    assignments: policy.assignments.length,
  }
  return (
    <div className="flex flex-col gap-4">
      <SegmentedControl
        label={m.policy_layer_egress()}
        value={section}
        className="self-start overflow-x-auto"
        options={EGRESS_SECTIONS.map((value) => ({
          value,
          label: (
            <>
              {egressLabel[value]()}
              <span className="font-mono text-meta text-muted-foreground tabular-nums">
                {counts[value]}
              </span>
            </>
          ),
        }))}
        onChange={onSection}
      />
      <LayerFacts>
        <Fact>{m.policy_egress_not_saved()}</Fact>
        {section === "public" || section === "profiles" ? (
          <Fact>{m.policy_fact_first_match()}</Fact>
        ) : null}
        {section === "assignments" ? (
          <Fact>{m.policy_fact_specific()}</Fact>
        ) : null}
      </LayerFacts>
      <div key={section} className="swap-in flex flex-col gap-4">
        {section === "public" ? (
          <OrderedRules
            label={egressLabel.public()}
            end={m.policy_egress_public_end()}
            items={policy.public_rules.map((rule, index) => ({
              key: `${rule.id}-${index}`,
              node: (
                <ExitCard
                  rule={rule}
                  names={names}
                  notes={notesOn(diagnostics, "public_rules", rule.id)}
                />
              ),
            }))}
          />
        ) : section === "profiles" ? (
          policy.profiles.length === 0 ? (
            <EmptyLine>{m.policy_egress_profiles_none()}</EmptyLine>
          ) : (
            policy.profiles.map((profile, index) => (
              <section
                key={`${profile.id}-${index}`}
                className="flex flex-col gap-3 rounded-row border border-divider p-3"
              >
                <header className="flex min-w-0 items-center gap-2 px-0.5">
                  <h3 className="min-w-0 flex-1 truncate font-mono text-row font-medium">
                    {profile.id}
                  </h3>
                  <NoteCounts
                    notes={notesOn(diagnostics, "profiles", profile.id)}
                  />
                </header>
                <RuleNotes
                  notes={notesOn(diagnostics, "profiles", profile.id).filter(
                    (note) =>
                      note.placed.kind === "field" &&
                      note.placed.anchor[2] !== "rules"
                  )}
                  names={names}
                />
                <OrderedRules
                  label={profile.id}
                  end={
                    <span className="flex min-w-0 items-center gap-2">
                      <span className="shrink-0">
                        {m.policy_egress_fallback()}
                      </span>
                      <Target
                        {...fallbackTarget(profile.fallback)}
                        names={names}
                      />
                    </span>
                  }
                  items={profile.rules.map((rule, at) => ({
                    key: `${rule.id}-${at}`,
                    node: (
                      <ExitCard
                        rule={rule}
                        names={names}
                        notes={notesOn(
                          diagnostics,
                          "profiles",
                          profile.id,
                          "rules",
                          { id: rule.id }
                        )}
                      />
                    ),
                  }))}
                />
              </section>
            ))
          )
        ) : section === "automatic" ? (
          <Automatic {...props} />
        ) : (
          <Assignments assignments={policy.assignments} {...props} />
        )}
      </div>
    </div>
  )
}

function Automatic({ policy, names, diagnostics }: LayerProps) {
  const automatic = policy.automatic ?? { regions: {}, fallback: "mesh-only" }
  const regions = Object.entries(automatic.regions)
  const notes = diagnostics.filter(
    (diagnostic) =>
      diagnostic.placed.kind === "field" &&
      diagnostic.placed.anchor[0] === "automatic"
  )
  return (
    <div className="flex flex-col gap-2">
      <RuleNotes notes={notes} names={names} />
      {regions.length === 0 ? (
        <EmptyLine>{m.policy_egress_regions_none()}</EmptyLine>
      ) : null}
      <ul className="flex flex-col gap-1">
        {regions.map(([region, profile]) => (
          <li
            key={region}
            className="flex h-11 items-center gap-3 rounded-row bg-fill-subtle px-3.5"
          >
            <span className="w-8 font-mono text-row font-medium">{region}</span>
            <ArrowRightIcon
              aria-hidden="true"
              className="size-3.5 text-muted-foreground"
            />
            <span className="min-w-0 truncate text-row">
              {profileName(profile, names)}
            </span>
          </li>
        ))}
        <li className="flex h-11 items-center gap-3 rounded-row border border-dashed border-divider px-3.5 text-row text-muted-foreground">
          <span>{m.policy_egress_regions_other()}</span>
          <ArrowRightIcon aria-hidden="true" className="size-3.5" />
          <span className="min-w-0 truncate text-foreground">
            {profileName(automatic.fallback, names)}
          </span>
        </li>
      </ul>
    </div>
  )
}

function Assignments({
  assignments,
  names,
  diagnostics,
}: LayerProps & { assignments: readonly ProfileAssignment[] }) {
  if (assignments.length === 0)
    return <EmptyLine>{m.policy_egress_assignments_none()}</EmptyLine>
  return (
    <ul className="grid grid-cols-1 gap-2 @2xl/workspace:grid-cols-2">
      {assignments.map((assignment, index) => {
        const notes = diagnostics.filter(
          (diagnostic) =>
            diagnostic.placed.kind === "field" &&
            diagnostic.placed.anchor[0] === "assignments" &&
            diagnostic.placed.anchor[1] === index
        )
        return (
          <li
            key={`${assignment.src}-${index}`}
            className="flex flex-col gap-2 rounded-row bg-fill-subtle py-2.5 pr-3 pl-3.5"
          >
            <span className="flex min-w-0 items-center gap-2">
              <SelectorChip selector={assignment.src} names={names} />
              <span className="flex-1" />
              {assignment.locked ? (
                <span className="inline-flex items-center gap-1 text-meta text-muted-foreground">
                  <LockIcon aria-hidden="true" className="size-3" />
                  {m.policy_egress_locked()}
                </span>
              ) : null}
              <NoteCounts notes={notes} />
            </span>
            <span className="flex min-w-0 flex-wrap gap-1">
              {assignment.allowed.map((profile) => (
                <span
                  key={profile}
                  className={cn(
                    "inline-flex h-6 items-center rounded-full border px-2 text-meta",
                    profile === assignment.default_profile
                      ? "border-foreground/30 bg-fill-selected font-medium"
                      : "border-divider"
                  )}
                >
                  {profileName(profile, names)}
                </span>
              ))}
            </span>
            <span className="text-meta text-muted-foreground">
              {m.policy_egress_default({
                name: profileName(assignment.default_profile, names),
              })}
            </span>
            <RuleNotes notes={notes} names={names} />
          </li>
        )
      })}
    </ul>
  )
}
