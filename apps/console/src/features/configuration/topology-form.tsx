import type {
  TopologyAction,
  TopologyDefault,
  TopologyRule,
} from "@meshway/control-client"
import { ArrowLeftRightIcon } from "lucide-react"

import { Section } from "@/components/chrome/section"
import { SegmentedControl } from "@/components/chrome/segmented-control"
import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"

import { ChipField } from "./chips"
import {
  AddButton,
  NoneLine,
  notesWithin,
  refusedAt,
  RemarkInput,
  RemoveButton,
  RuleRow,
  selectorChips,
  type Names,
  type Note,
} from "./form-kit"
import { pointerOf } from "./jsonc"

const actionLabel: Record<TopologyAction, () => string> = {
  allow: () => m.configuration_topology_allow(),
  block: () => m.configuration_topology_block(),
}

const defaultLabel: Record<TopologyDefault, () => string> = {
  allow: () => m.configuration_topology_default_allow(),
  deny: () => m.configuration_topology_default_deny(),
}

function ActionBadge({ action }: { action: TopologyAction }) {
  return (
    <span
      className={cn(
        "inline-flex h-6 shrink-0 items-center rounded-full px-2.5 text-meta font-medium",
        action === "block"
          ? "bg-destructive/12 text-destructive"
          : "bg-success/14 text-success"
      )}
    >
      {actionLabel[action]()}
    </span>
  )
}

/** Which pairs of nodes may connect directly: any matching block wins, then any allow, then the default. */
export function TopologyForm({
  rules,
  fallback,
  editable,
  names,
  notes,
  onChange,
  onFallback,
}: {
  rules: readonly TopologyRule[]
  /** `defaults.topology`: when no rule matches. */
  fallback: TopologyDefault
  editable: boolean
  names: Names
  notes: readonly Note[]
  onChange: (rules: TopologyRule[]) => void
  onFallback: (fallback: TopologyDefault) => void
}) {
  const ends = selectorChips(names, false)
  const update = (index: number, next: TopologyRule) =>
    onChange(rules.map((rule, at) => (at === index ? next : rule)))

  return (
    <div className="flex flex-col gap-6">
      <div className="flex min-h-12 items-center justify-between gap-3 rounded-tile bg-fill-subtle py-2 pr-2 pl-3.5">
        <span className="text-row font-medium">
          {m.configuration_topology_default()}
        </span>
        {editable ? (
          <SegmentedControl
            size="sm"
            label={m.configuration_topology_default()}
            value={fallback}
            options={[
              { value: "allow", label: defaultLabel.allow() },
              { value: "deny", label: defaultLabel.deny() },
            ]}
            onChange={onFallback}
          />
        ) : (
          <span className="pr-1.5 text-row">{defaultLabel[fallback]()}</span>
        )}
      </div>
      <Section title={m.configuration_topology_rules()} count={rules.length}>
        <div className="flex flex-col gap-2">
          {rules.length === 0 ? (
            <NoneLine>{m.configuration_topology_none()}</NoneLine>
          ) : null}
          <ul
            aria-label={m.configuration_topology_rules()}
            className="flex flex-col gap-2"
          >
            {rules.map((rule, index) => (
              <li key={index}>
                <RuleRow
                  label={m.configuration_rule_label({ index: index + 1 })}
                  notes={notesWithin(notes, pointerOf(["topology", index]))}
                  end={
                    <>
                      {editable ? (
                        <SegmentedControl
                          size="sm"
                          label={m.configuration_topology_action()}
                          value={rule.action}
                          // The chosen action wears the colour of its read-only badge.
                          options={(["allow", "block"] as const).map(
                            (action) => ({
                              value: action,
                              label: (
                                <span
                                  className={cn(
                                    rule.action === action &&
                                      (action === "block"
                                        ? "text-destructive"
                                        : "text-success")
                                  )}
                                >
                                  {actionLabel[action]()}
                                </span>
                              ),
                            })
                          )}
                          onChange={(action) =>
                            update(index, { ...rule, action })
                          }
                        />
                      ) : (
                        <ActionBadge action={rule.action} />
                      )}
                      <RemarkInput
                        value={rule.remark ?? ""}
                        editable={editable}
                        onChange={(remark) =>
                          update(index, { ...rule, remark })
                        }
                      />
                      {editable ? (
                        <RemoveButton
                          label={m.configuration_rule_remove({
                            index: index + 1,
                          })}
                          onRemove={() =>
                            onChange(rules.filter((_, at) => at !== index))
                          }
                        />
                      ) : null}
                    </>
                  }
                >
                  <ChipField
                    value={rule.a}
                    kind={ends}
                    label={m.configuration_topology_a()}
                    editable={editable}
                    invalid={refusedAt(
                      notes,
                      pointerOf(["topology", index, "a"])
                    )}
                    onChange={(a) => update(index, { ...rule, a })}
                  />
                  <ArrowLeftRightIcon
                    role="img"
                    aria-label={m.configuration_topology_between()}
                    className="size-3.5 shrink-0 text-muted-foreground"
                  />
                  <ChipField
                    value={rule.b}
                    kind={ends}
                    label={m.configuration_topology_b()}
                    editable={editable}
                    invalid={refusedAt(
                      notes,
                      pointerOf(["topology", index, "b"])
                    )}
                    onChange={(b) => update(index, { ...rule, b })}
                  />
                </RuleRow>
              </li>
            ))}
          </ul>
          {editable ? (
            <AddButton
              label={m.configuration_topology_add()}
              onAdd={() =>
                onChange([...rules, { action: "allow", a: "*", b: "*" }])
              }
            />
          ) : null}
        </div>
      </Section>
    </div>
  )
}
