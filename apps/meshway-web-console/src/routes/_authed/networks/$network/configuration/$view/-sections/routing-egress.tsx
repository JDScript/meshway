import type { EgressRule } from "@meshway/control-client"
import { ArrowRightIcon } from "lucide-react"
import { type ReactNode } from "react"

import { ChipSetField } from "@/components/configuration/chips"
import {
  AddButton,
  notesWithin,
  refusedAt,
  RuleRow,
  type Note,
} from "@/components/configuration/form-kit"
import { pointerOf, type JsonPath } from "@/lib/configuration/jsonc"
import { matchOf, matchValues, moved } from "@/lib/configuration/routing"
import { m } from "@/paraglide/messages.js"

import { moveKeys, OrderedRules, useRowKeys } from "./ordered"
import {
  ActionField,
  destinationChips,
  replaceAt,
  RowEnd,
  type Common,
} from "./route-kit"

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

/** Egress rules, top to bottom, ending in `end` (or a profile's fallback). */
export function EgressRules({
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
