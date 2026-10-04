import type { MeshRoute } from "@meshway/control-client"
import { ArrowRightIcon } from "lucide-react"

import { Section } from "@/components/chrome/section"
import { ChipSetField } from "@/components/configuration/chips"
import {
  AddButton,
  notesWithin,
  refusedAt,
  RuleRow,
  selectorChips,
  type Names,
  type Note,
} from "@/components/configuration/form-kit"
import { pointerOf } from "@/lib/configuration/jsonc"
import { moved } from "@/lib/configuration/routing"
import { m } from "@/paraglide/messages.js"

import { moveKeys, OrderedRules, useRowKeys } from "./ordered"
import { Hint, nodeChips, replaceAt, RowEnd } from "./route-kit"

/** Mesh routes: the first rule matching source and destination decides the nodes traffic must pass, in order. */
export function MeshRoutes({
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
