import type {
  AutoProfile,
  EgressRouting,
  EgressRule,
  MeshRoute,
  ProfileAssignment,
  RouteAction,
  RoutingConfiguration,
  RoutingProfile,
} from "@meshway/control-client"
import { ArrowRightIcon, ListIcon, LockIcon, LockOpenIcon } from "lucide-react"
import { useId, type ReactNode } from "react"

import { SwitchRow } from "@/components/chrome/form"
import { Section } from "@/components/chrome/section"
import { ChipField, ChipSetField } from "@/components/configuration/chips"
import {
  AddButton,
  NoneLine,
  Notes,
  notesWithin,
  refusedAt,
  RemarkInput,
  RemoveButton,
  RuleRow,
  selectorChips,
  type Names,
  type Note,
} from "@/components/configuration/form-kit"
import { Button } from "@/components/ui/button"
import {
  pointerOf,
  pointerSegments,
  type JsonPath,
} from "@/lib/configuration/jsonc"
import { without } from "@/lib/configuration/keyed"
import {
  matchOf,
  matchValues,
  moved,
  profileChoices,
  renameProfile,
  type ProfileChoice,
} from "@/lib/configuration/routing"
import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"

import { AddKey, KeyInput, type KeyRule } from "./keyed-form"
import { MoveButtons, moveKeys, OrderedRules, useRowKeys } from "./ordered"
import {
  ActionField,
  countryChips,
  destinationChips,
  nodeChips,
  profileChips,
  profileLabel,
} from "./route-kit"

const replaceAt = <T,>(items: readonly T[], index: number, next: T): T[] =>
  items.map((item, at) => (at === index ? next : item))

/** A rule set a rule names that the draft does not have, as a note under it (the controller has the last word). */
function setNotes(
  rule: EgressRule,
  path: string,
  ruleSets: readonly string[]
): Note[] {
  return (rule.match.ruleSets ?? [])
    .filter((key) => !ruleSets.includes(key))
    .map((key) => ({
      path,
      message: m.configuration_routing_set_note_unknown({ key }),
      origin: "local" as const,
    }))
}

/** Traffic routing: Mesh routes, then egress (public rules, profiles, the automatic profile, assignments). */
export function RoutingForm({
  routing,
  editable,
  names,
  notes,
  exits,
  ruleSets,
  onChange,
  onOpenRuleSets,
}: {
  routing: RoutingConfiguration
  editable: boolean
  names: Names
  notes: readonly Note[]
  /** Node IDs enabled as exits in the configuration. */
  exits: ReadonlySet<string>
  /** The draft's rule set keys. */
  ruleSets: readonly string[]
  onChange: (routing: RoutingConfiguration) => void
  onOpenRuleSets: () => void
}) {
  const egress = routing.egress ?? {}
  const setEgress = (next: EgressRouting) =>
    onChange({ ...routing, egress: next })
  const common = { editable, names, notes, exits, ruleSets }
  // A node as a whole (its assigned egress against its access) or a rule set's content.
  const outside = notes
    .filter(
      (note) => !note.path.startsWith("/routing/") && note.path !== "/routing"
    )
    .map((note) => {
      const [section, key] = pointerSegments(note.path) ?? []
      if (section === "nodes" && key !== undefined)
        return {
          ...note,
          message: m.configuration_routing_node_note({
            node: names.nodes.get(key) ?? key.slice(0, 8),
            message: note.message,
          }),
        }
      if (section === "ruleSets" && key !== undefined)
        return {
          ...note,
          message: m.configuration_routing_set_note({
            key,
            message: note.message,
          }),
        }
      return note
    })

  return (
    <div className="flex flex-col gap-8">
      {outside.length > 0 ? (
        <div className="flex flex-col gap-2 rounded-tile bg-fill-subtle px-3.5 py-3">
          <Notes notes={outside} />
          {outside.some((note) => note.path.startsWith("/ruleSets/")) ? (
            <Button
              type="button"
              size="sm"
              variant="secondary"
              className="self-start rounded-full"
              onClick={onOpenRuleSets}
            >
              <ListIcon />
              {m.configuration_routing_open_rule_sets()}
            </Button>
          ) : null}
        </div>
      ) : null}
      <MeshRoutes
        rules={routing.mesh ?? []}
        editable={editable}
        names={names}
        notes={notes}
        onChange={(mesh) => onChange({ ...routing, mesh })}
      />
      <Section
        title={m.configuration_routing_public()}
        count={(egress.rules ?? []).length}
      >
        <Hint>{m.configuration_routing_public_hint()}</Hint>
        <EgressRules
          {...common}
          path={["routing", "egress", "rules"]}
          rules={egress.rules ?? []}
          end={m.configuration_routing_public_end()}
          onChange={(rules) => setEgress({ ...egress, rules })}
        />
      </Section>
      <Profiles {...common} routing={routing} onChange={onChange} />
      <Auto
        auto={egress.auto ?? null}
        choices={profileChoices(routing, [...exits], false)}
        editable={editable}
        names={names}
        notes={notes}
        onChange={(auto) => {
          const next = { ...egress }
          if (auto) next.auto = auto
          else delete next.auto
          setEgress(next)
        }}
      />
      <Assignments
        assignments={egress.assignments ?? []}
        choices={profileChoices(routing, [...exits], true)}
        editable={editable}
        names={names}
        notes={notes}
        onChange={(assignments) => setEgress({ ...egress, assignments })}
      />
    </div>
  )
}

function Hint({ children }: { children: ReactNode }) {
  return <p className="text-meta text-muted-foreground">{children}</p>
}

/** Mesh routes: the first rule matching source and destination decides the nodes traffic must pass, in order. */
function MeshRoutes({
  rules,
  editable,
  names,
  notes,
  onChange,
}: {
  rules: readonly MeshRoute[]
  editable: boolean
  names: Names
  notes: readonly Note[]
  onChange: (rules: MeshRoute[]) => void
}) {
  const rows = useRowKeys(rules.length)
  const ends = selectorChips(names, false)
  const hops = nodeChips(names, null)
  const update = (index: number, next: MeshRoute) =>
    onChange(replaceAt(rules, index, next))

  return (
    <Section title={m.configuration_routing_mesh()} count={rules.length}>
      <Hint>{m.configuration_routing_mesh_hint()}</Hint>
      <OrderedRules
        label={m.configuration_routing_mesh()}
        end={m.configuration_routing_mesh_end()}
        items={rules.map((rule, index) => {
          const at = (field: string) =>
            pointerOf(["routing", "mesh", index, field])
          const label = m.configuration_rule_label({ index: index + 1 })
          const onMove = (delta: -1 | 1) => {
            rows.moved(index, delta)
            onChange(moved(rules, index, delta))
          }
          return {
            key: rows.key(index),
            node: (
              <div
                onKeyDown={
                  editable ? moveKeys(index, rules.length, onMove) : undefined
                }
              >
                <RuleRow
                  label={label}
                  notes={notesWithin(
                    notes,
                    pointerOf(["routing", "mesh", index])
                  )}
                  end={
                    <RowEnd
                      remark={rule.remark ?? ""}
                      editable={editable}
                      index={index}
                      count={rules.length}
                      label={label}
                      onRemark={(remark) => update(index, { ...rule, remark })}
                      onMove={onMove}
                      onRemove={() => {
                        rows.removed(index)
                        onChange(rules.filter((_, at) => at !== index))
                      }}
                    />
                  }
                >
                  <ChipSetField
                    values={rule.src}
                    kind={ends}
                    label={m.configuration_routing_src()}
                    addLabel={m.configuration_routing_src_add()}
                    editable={editable}
                    invalid={refusedAt(notes, at("src"))}
                    onChange={(src) => update(index, { ...rule, src })}
                  />
                  <ArrowRightIcon
                    role="img"
                    aria-label={m.configuration_routing_to()}
                    className="size-3.5 shrink-0 text-muted-foreground"
                  />
                  <ChipSetField
                    values={rule.dst}
                    kind={ends}
                    label={m.configuration_routing_dst()}
                    addLabel={m.configuration_routing_dst_add()}
                    editable={editable}
                    invalid={refusedAt(notes, at("dst"))}
                    onChange={(dst) => update(index, { ...rule, dst })}
                  />
                  <span className="pl-1 text-meta text-muted-foreground">
                    {m.configuration_routing_via()}
                  </span>
                  <ChipSetField
                    values={rule.via}
                    kind={hops}
                    label={m.configuration_routing_via_label()}
                    addLabel={m.configuration_routing_via_add()}
                    editable={editable}
                    invalid={refusedAt(notes, at("via"))}
                    onChange={(via) => update(index, { ...rule, via })}
                  />
                </RuleRow>
              </div>
            ),
          }
        })}
        add={
          editable ? (
            <AddButton
              label={m.configuration_routing_mesh_add()}
              onAdd={() =>
                onChange([...rules, { src: ["*"], dst: [], via: [] }])
              }
            />
          ) : undefined
        }
      />
    </Section>
  )
}

/** A rule's end: its remark, moving it, removing it. */
function RowEnd({
  remark,
  editable,
  index,
  count,
  label,
  onRemark,
  onMove,
  onRemove,
}: {
  remark: string
  editable: boolean
  index: number
  count: number
  label: string
  onRemark: (remark: string) => void
  onMove: (delta: -1 | 1) => void
  onRemove: () => void
}) {
  return (
    <>
      <RemarkInput value={remark} editable={editable} onChange={onRemark} />
      {editable ? (
        <>
          <MoveButtons
            index={index}
            count={count}
            label={label}
            onMove={onMove}
          />
          <RemoveButton
            label={m.configuration_rule_remove({ index: index + 1 })}
            onRemove={onRemove}
          />
        </>
      ) : null}
    </>
  )
}

type Common = {
  editable: boolean
  names: Names
  notes: readonly Note[]
  exits: ReadonlySet<string>
  ruleSets: readonly string[]
}

/** Egress rules, top to bottom, ending in `end` (or a profile's fallback). */
function EgressRules({
  path,
  rules,
  end,
  editable,
  names,
  notes,
  exits,
  ruleSets,
  onChange,
}: Common & {
  path: JsonPath
  rules: readonly EgressRule[]
  end: ReactNode
  onChange: (rules: EgressRule[]) => void
}) {
  const rows = useRowKeys(rules.length)
  const destinations = destinationChips(ruleSets, true)
  const update = (index: number, next: EgressRule) =>
    onChange(replaceAt(rules, index, next))

  return (
    <OrderedRules
      label={m.configuration_routing_rules()}
      end={end}
      items={rules.map((rule, index) => {
        const at = pointerOf([...path, index])
        const label = m.configuration_rule_label({ index: index + 1 })
        const onMove = (delta: -1 | 1) => {
          rows.moved(index, delta)
          onChange(moved(rules, index, delta))
        }
        return {
          key: rows.key(index),
          node: (
            <div
              onKeyDown={
                editable ? moveKeys(index, rules.length, onMove) : undefined
              }
            >
              <RuleRow
                label={label}
                notes={[
                  ...notesWithin(notes, at),
                  ...setNotes(
                    rule,
                    pointerOf([...path, index, "match"]),
                    ruleSets
                  ),
                ]}
                end={
                  <RowEnd
                    remark={rule.remark ?? ""}
                    editable={editable}
                    index={index}
                    count={rules.length}
                    label={label}
                    onRemark={(remark) => update(index, { ...rule, remark })}
                    onMove={onMove}
                    onRemove={() => {
                      rows.removed(index)
                      onChange(rules.filter((_, at) => at !== index))
                    }}
                  />
                }
              >
                <ChipSetField
                  values={matchValues(rule.match)}
                  kind={destinations}
                  label={m.configuration_routing_match()}
                  addLabel={m.configuration_routing_match_add()}
                  editable={editable}
                  invalid={refusedAt(
                    notes,
                    pointerOf([...path, index, "match"])
                  )}
                  onChange={(values) =>
                    update(index, { ...rule, match: matchOf(values) })
                  }
                />
                <ArrowRightIcon
                  role="img"
                  aria-label={m.configuration_routing_to()}
                  className="size-3.5 shrink-0 text-muted-foreground"
                />
                <ActionField
                  action={rule.action}
                  path={[...path, index, "action"]}
                  fallback={false}
                  editable={editable}
                  names={names}
                  exits={exits}
                  notes={notes}
                  onChange={(action) => update(index, { ...rule, action })}
                />
              </RuleRow>
            </div>
          ),
        }
      })}
      add={
        editable ? (
          <AddButton
            label={m.configuration_routing_rule_add()}
            onAdd={() =>
              onChange([
                ...rules,
                { match: {}, action: { type: "exit", nodes: [] } },
              ])
            }
          />
        ) : undefined
      }
    />
  )
}

const profileKey = (): KeyRule => ({
  prefix: "",
  placeholder: m.configuration_routing_profile_name_placeholder(),
})

/** The configuration's profiles: each its ordered rules and its fallback; the built-ins beside them. */
function Profiles({
  routing,
  onChange,
  ...common
}: Common & {
  routing: RoutingConfiguration
  onChange: (routing: RoutingConfiguration) => void
}) {
  const { editable, names, notes, exits } = common
  const egress = routing.egress ?? {}
  const profiles = egress.profiles ?? {}
  const entries = Object.entries(profiles)
  const setProfiles = (next: Record<string, RoutingProfile>) =>
    onChange({ ...routing, egress: { ...egress, profiles: next } })
  const builtins = profileChoices(routing, [...exits], true).filter(
    (choice) => choice.kind !== "custom"
  )

  return (
    <Section title={m.configuration_routing_profiles()} count={entries.length}>
      <Hint>{m.configuration_routing_profiles_hint()}</Hint>
      <div className="flex flex-wrap items-center gap-1.5 rounded-tile bg-fill-subtle px-3.5 py-2.5">
        <span className="pr-1 text-meta text-muted-foreground">
          {m.configuration_routing_builtin()}
        </span>
        {builtins.map((choice) => (
          <ChipField
            key={choice.value}
            value={choice.value}
            kind={profileChips(builtins, names)}
            label={profileLabel(choice.value, names)}
            editable={false}
            onChange={() => undefined}
          />
        ))}
        {exits.size === 0 ? (
          <span className="text-meta text-muted-foreground">
            {m.configuration_routing_no_exits()}
          </span>
        ) : null}
      </div>
      {entries.length === 0 ? (
        <NoneLine>{m.configuration_routing_profiles_none()}</NoneLine>
      ) : null}
      <ul
        aria-label={m.configuration_routing_profiles()}
        className="flex flex-col gap-3"
      >
        {entries.map(([key, profile], index) => {
          const path = ["routing", "egress", "profiles", key]
          const own = [
            ...notes.filter((note) => note.path === pointerOf(path)),
            ...notesWithin(notes, pointerOf([...path, "fallback"])),
          ]
          return (
            // A profile is its place: renaming keeps it.
            <li
              key={index}
              className="group/rule flex flex-col gap-3 rounded-surface border border-divider p-3"
            >
              <div className="flex min-h-8 items-center gap-2">
                <KeyInput
                  map={profiles}
                  value={key}
                  rule={profileKey()}
                  label={m.configuration_routing_profile_name()}
                  editable={editable}
                  invalid={refusedAt(notes, pointerOf(path))}
                  onRename={(next) =>
                    onChange(renameProfile(routing, key, next))
                  }
                  className="w-56 min-w-0 text-row"
                />
                <span className="flex-1" />
                {editable ? (
                  <RemoveButton
                    label={m.configuration_key_remove({ name: key })}
                    onRemove={() => setProfiles(without(profiles, key))}
                  />
                ) : null}
              </div>
              <EgressRules
                {...common}
                path={[...path, "rules"]}
                rules={profile.rules ?? []}
                end={
                  <>
                    <span className="shrink-0">
                      {m.configuration_routing_fallback()}
                    </span>
                    <ActionField
                      action={profile.fallback}
                      path={[...path, "fallback"]}
                      fallback
                      editable={editable}
                      names={names}
                      exits={exits}
                      notes={notes}
                      onChange={(fallback: RouteAction) =>
                        setProfiles({
                          ...profiles,
                          [key]: { ...profile, fallback },
                        })
                      }
                    />
                  </>
                }
                onChange={(rules) =>
                  setProfiles({ ...profiles, [key]: { ...profile, rules } })
                }
              />
              <Notes notes={own} className="px-1.5" />
            </li>
          )
        })}
      </ul>
      {editable ? (
        <AddKey
          map={profiles}
          rule={profileKey()}
          label={m.configuration_routing_profile_new()}
          addLabel={m.configuration_key_add()}
          onAdd={(key) =>
            setProfiles({
              ...profiles,
              [key]: { rules: [], fallback: { type: "unmanaged" } },
            })
          }
        />
      ) : null}
    </Section>
  )
}

/** The automatic profile: countries and regions to profiles, in order, and the profile when the region is not known. */
function Auto({
  auto,
  choices,
  editable,
  names,
  notes,
  onChange,
}: {
  auto: AutoProfile | null
  /** What it may choose: not itself. */
  choices: readonly ProfileChoice[]
  editable: boolean
  names: Names
  notes: readonly Note[]
  onChange: (auto: AutoProfile | null) => void
}) {
  const id = useId()
  const rows = useRowKeys(auto?.rules.length ?? 0)
  const profiles = profileChips(choices, names)
  const countries = countryChips()
  const path = ["routing", "egress", "auto"]
  const own = notes.filter(
    (note) =>
      note.path === pointerOf(path) ||
      note.path === pointerOf([...path, "rules"])
  )

  return (
    <Section title={m.configuration_routing_auto()}>
      <Hint>{m.configuration_routing_auto_hint()}</Hint>
      <SwitchRow
        id={`${id}-auto`}
        label={m.configuration_routing_auto_enabled()}
        checked={auto !== null}
        disabled={!editable}
        onCheckedChange={(on) =>
          onChange(on ? { rules: [], fallback: "mesh" } : null)
        }
      />
      {auto ? (
        <>
          <OrderedRules
            label={m.configuration_routing_auto()}
            end={
              <>
                <span className="shrink-0">
                  {m.configuration_routing_auto_fallback()}
                </span>
                <ChipField
                  value={auto.fallback}
                  kind={profiles}
                  label={m.configuration_routing_auto_fallback()}
                  editable={editable}
                  invalid={refusedAt(notes, pointerOf(path))}
                  onChange={(fallback) => onChange({ ...auto, fallback })}
                />
              </>
            }
            items={auto.rules.map((rule, index) => {
              const label = m.configuration_rule_label({ index: index + 1 })
              const onMove = (delta: -1 | 1) => {
                rows.moved(index, delta)
                onChange({ ...auto, rules: moved(auto.rules, index, delta) })
              }
              const update = (next: typeof rule) =>
                onChange({ ...auto, rules: replaceAt(auto.rules, index, next) })
              return {
                key: rows.key(index),
                node: (
                  <div
                    onKeyDown={
                      editable
                        ? moveKeys(index, auto.rules.length, onMove)
                        : undefined
                    }
                  >
                    <RuleRow
                      label={label}
                      notes={[]}
                      end={
                        editable ? (
                          <>
                            <MoveButtons
                              index={index}
                              count={auto.rules.length}
                              label={label}
                              onMove={onMove}
                            />
                            <RemoveButton
                              label={m.configuration_rule_remove({
                                index: index + 1,
                              })}
                              onRemove={() => {
                                rows.removed(index)
                                onChange({
                                  ...auto,
                                  rules: auto.rules.filter(
                                    (_, at) => at !== index
                                  ),
                                })
                              }}
                            />
                          </>
                        ) : null
                      }
                    >
                      <ChipSetField
                        values={rule.countries}
                        kind={countries}
                        label={m.configuration_routing_countries()}
                        addLabel={m.configuration_routing_countries_add()}
                        editable={editable}
                        onChange={(next) =>
                          update({ ...rule, countries: next })
                        }
                      />
                      <ArrowRightIcon
                        role="img"
                        aria-label={m.configuration_routing_to()}
                        className="size-3.5 shrink-0 text-muted-foreground"
                      />
                      <ChipField
                        value={rule.profile}
                        kind={profiles}
                        label={m.configuration_routing_profile()}
                        editable={editable}
                        onChange={(profile) => update({ ...rule, profile })}
                      />
                    </RuleRow>
                  </div>
                ),
              }
            })}
            add={
              editable ? (
                <AddButton
                  label={m.configuration_routing_auto_add()}
                  onAdd={() =>
                    onChange({
                      ...auto,
                      rules: [
                        ...auto.rules,
                        { countries: [], profile: "mesh" },
                      ],
                    })
                  }
                />
              ) : undefined
            }
          />
          <Notes notes={own} className="px-1.5" />
        </>
      ) : null}
    </Section>
  )
}

/** Who may use which profiles: the allowed sets add up; overlapping assignments must agree on the default and the lock. */
function Assignments({
  assignments,
  choices,
  editable,
  names,
  notes,
  onChange,
}: {
  assignments: readonly ProfileAssignment[]
  choices: readonly ProfileChoice[]
  editable: boolean
  names: Names
  notes: readonly Note[]
  onChange: (assignments: ProfileAssignment[]) => void
}) {
  const targets = selectorChips(names, false)
  const profiles = profileChips(choices, names)
  const update = (index: number, next: ProfileAssignment) =>
    onChange(replaceAt(assignments, index, next))

  return (
    <Section
      title={m.configuration_routing_assignments()}
      count={assignments.length}
    >
      <Hint>{m.configuration_routing_assignments_hint()}</Hint>
      {assignments.length === 0 ? (
        <NoneLine>{m.configuration_routing_assignments_none()}</NoneLine>
      ) : null}
      <ul
        aria-label={m.configuration_routing_assignments()}
        className="flex flex-col gap-2"
      >
        {assignments.map((assignment, index) => {
          const path = pointerOf(["routing", "egress", "assignments", index])
          const locked = assignment.locked ?? false
          const LockGlyph = locked ? LockIcon : LockOpenIcon
          return (
            // Assignments have no IDs: a row is its position.
            <li key={index}>
              <RuleRow
                label={m.configuration_routing_assignment_label({
                  index: index + 1,
                })}
                notes={notesWithin(notes, path)}
                end={
                  <>
                    {editable ? (
                      <Button
                        type="button"
                        variant="ghost"
                        size="sm"
                        aria-pressed={locked}
                        onClick={() =>
                          update(index, { ...assignment, locked: !locked })
                        }
                        className={cn(
                          "h-7 rounded-full px-2.5 text-meta",
                          locked ? "text-foreground" : "text-muted-foreground"
                        )}
                      >
                        <LockGlyph />
                        {locked
                          ? m.configuration_routing_locked()
                          : m.configuration_routing_unlocked()}
                      </Button>
                    ) : (
                      <span className="inline-flex h-7 items-center gap-1.5 px-2.5 text-meta text-muted-foreground">
                        <LockGlyph className="size-3.5" />
                        {locked
                          ? m.configuration_routing_locked()
                          : m.configuration_routing_unlocked()}
                      </span>
                    )}
                    {editable ? (
                      <RemoveButton
                        label={m.configuration_routing_assignment_remove({
                          index: index + 1,
                        })}
                        onRemove={() =>
                          onChange(assignments.filter((_, at) => at !== index))
                        }
                      />
                    ) : null}
                  </>
                }
              >
                <ChipSetField
                  values={assignment.target}
                  kind={targets}
                  label={m.configuration_routing_target()}
                  addLabel={m.configuration_routing_target_add()}
                  editable={editable}
                  invalid={refusedAt(notes, `${path}/target`)}
                  onChange={(target) =>
                    update(index, { ...assignment, target })
                  }
                />
                <span className="pl-1 text-meta text-muted-foreground">
                  {m.configuration_routing_allowed()}
                </span>
                <ChipSetField
                  values={assignment.allowed}
                  kind={profiles}
                  label={m.configuration_routing_allowed_label()}
                  addLabel={m.configuration_routing_allowed_add()}
                  editable={editable}
                  onChange={(allowed) =>
                    update(index, { ...assignment, allowed })
                  }
                />
                <span className="pl-1 text-meta text-muted-foreground">
                  {m.configuration_routing_default()}
                </span>
                <ChipField
                  value={assignment.default}
                  kind={profiles}
                  label={m.configuration_routing_default_label()}
                  editable={editable}
                  onChange={(value) =>
                    update(index, { ...assignment, default: value })
                  }
                />
              </RuleRow>
            </li>
          )
        })}
      </ul>
      {editable ? (
        <AddButton
          label={m.configuration_routing_assignment_add()}
          onAdd={() =>
            onChange([
              ...assignments,
              { target: [], allowed: ["mesh"], default: "mesh" },
            ])
          }
        />
      ) : null}
    </Section>
  )
}
