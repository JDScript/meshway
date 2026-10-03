import type { MeshRule } from "@meshway/control-client"
import { ArrowRightIcon, ChevronRightIcon } from "lucide-react"

import { Section } from "@/components/chrome/section"
import { m } from "@/paraglide/messages.js"

import { freshId, type PlacedDiagnostic } from "./document"
import {
  idInvalid,
  notesOn,
  replaceAt,
  useRuleRows,
  type LayerProps,
} from "./layer"
import {
  AddRule,
  Fact,
  LayerFacts,
  MoveButtons,
  moveKeys,
  OrderedRules,
  RemoveRule,
  RuleIdInput,
  RuleRow,
} from "./rules"
import {
  SelectorChip,
  SelectorField,
  SelectorSetField,
  withNodeNames,
  type SelectorNames,
} from "./selectors"

const ENDS = ["all", "tag", "node"] as const
const HOPS = ["tag", "node"] as const

/** The mandatory hops of a route, in order. */
export function HopChain({
  via,
  names,
}: {
  via: readonly string[]
  names: SelectorNames
}) {
  if (via.length === 0)
    return (
      <span className="text-meta text-muted-foreground">
        {m.policy_mesh_no_hops()}
      </span>
    )
  return (
    <span className="flex min-w-0 flex-wrap items-center gap-1 text-meta text-muted-foreground">
      <span>{m.policy_mesh_via()}</span>
      {via.map((hop, index) => (
        <span
          key={`${hop}-${index}`}
          className="inline-flex items-center gap-1"
        >
          {index > 0 ? (
            <ChevronRightIcon aria-hidden="true" className="size-3" />
          ) : null}
          <SelectorChip selector={hop} names={names} />
        </span>
      ))}
    </span>
  )
}

/** Routes inside the mesh: the first rule matching source and destination decides the mandatory hops. */
export function MeshLayer(props: LayerProps) {
  const { policy, editable, names, edit, move, rule: linked, onRule } = props
  const rules = policy.mesh
  const rows = useRuleRows(rules, linked, onRule)
  const failures = props.diagnostics.filter(
    (
      diagnostic
    ): diagnostic is PlacedDiagnostic & {
      placed: { kind: "path"; nodeId: string }
    } => diagnostic.placed.kind === "path"
  )
  const update = (index: number, next: MeshRule) =>
    edit((current) => ({
      ...current,
      mesh: replaceAt(current.mesh, index, next),
    }))

  return (
    <div className="flex flex-col gap-4">
      <LayerFacts>
        <Fact>{m.policy_count_rules({ count: rules.length })}</Fact>
        <Fact>{m.policy_fact_first_match()}</Fact>
      </LayerFacts>
      {failures.length > 0 ? (
        <Section title={m.policy_mesh_failures()} count={failures.length}>
          <ul className="flex flex-col gap-1">
            {failures.map((failure, index) => (
              <li
                key={`${failure.field}-${index}`}
                className="flex min-w-0 items-start gap-2.5 rounded-row bg-fill-subtle px-3.5 py-2.5"
              >
                <button
                  type="button"
                  onClick={() => props.onOpenNode(failure.placed.nodeId)}
                  className="shrink-0 rounded-full outline-none focus-visible:ring-2 focus-visible:ring-ring"
                >
                  <SelectorChip
                    selector={`node:${failure.placed.nodeId}`}
                    names={names}
                    className="hover:bg-fill-hover"
                  />
                </button>
                <span className="min-w-0 pt-0.5 text-meta break-words text-muted-foreground">
                  {withNodeNames(failure.message, names)}
                </span>
              </li>
            ))}
          </ul>
        </Section>
      ) : null}
      <OrderedRules
        label={m.policy_layer_mesh()}
        end={m.policy_mesh_end()}
        items={rules.map((rule, index) => {
          const onMove = (delta: -1 | 1) => {
            move(["mesh"], index, delta)
            rows.moved(index, delta)
          }
          const invalid = (field: "src" | "dst") =>
            notesOn(props.diagnostics, "mesh", rule.id, field).length > 0
          return {
            // A moved rule keeps its row, so it glides.
            key: rows.key(index),
            node: (
              <RuleRow
                current={rows.current(index)}
                added={rows.autoOpen(index)}
                label={rule.id}
                notes={notesOn(props.diagnostics, "mesh", rule.id)}
                names={names}
                onKeyDown={
                  editable ? moveKeys(index, rules.length, onMove) : undefined
                }
                end={
                  <>
                    <RuleIdInput
                      value={rule.id}
                      invalid={idInvalid(props.problems, "mesh", index)}
                      editable={editable}
                      onChange={(id) => {
                        update(index, { ...rule, id })
                        rows.renamed(index, id)
                      }}
                    />
                    {editable ? (
                      <>
                        <MoveButtons
                          index={index}
                          count={rules.length}
                          label={rule.id}
                          onMove={onMove}
                        />
                        <RemoveRule
                          label={rule.id}
                          onRemove={() => {
                            edit((current) => ({
                              ...current,
                              mesh: current.mesh.filter(
                                (_, at) => at !== index
                              ),
                            }))
                            rows.removed(index)
                          }}
                        />
                      </>
                    ) : null}
                  </>
                }
              >
                <SelectorField
                  value={rule.src}
                  names={names}
                  kinds={ENDS}
                  label={m.policy_mesh_src()}
                  disabled={!editable}
                  invalid={invalid("src")}
                  autoOpen={rows.autoOpen(index)}
                  onClosed={rows.settled}
                  onOpenNode={props.onOpenNode}
                  onChange={(src) => update(index, { ...rule, src })}
                />
                <ArrowRightIcon
                  aria-label={m.policy_grant_to()}
                  className="size-3.5 shrink-0 text-muted-foreground"
                />
                <SelectorField
                  value={rule.dst}
                  names={names}
                  kinds={ENDS}
                  label={m.policy_mesh_dst()}
                  disabled={!editable}
                  invalid={invalid("dst")}
                  onOpenNode={props.onOpenNode}
                  onChange={(dst) => update(index, { ...rule, dst })}
                />
                <span className="inline-flex min-w-0 items-start gap-1.5 pl-1.5">
                  <span className="shrink-0 text-meta leading-7 text-muted-foreground">
                    {m.policy_mesh_via()}
                  </span>
                  <SelectorSetField
                    values={rule.via}
                    names={names}
                    kinds={HOPS}
                    label={m.policy_mesh_hops()}
                    addLabel={m.policy_mesh_hop_add()}
                    disabled={!editable}
                    invalid={(at) =>
                      notesOn(props.diagnostics, "mesh", rule.id, "via", at)
                        .length > 0
                    }
                    joiner={
                      <ChevronRightIcon
                        aria-hidden="true"
                        className="size-3 shrink-0 text-muted-foreground"
                      />
                    }
                    empty={m.policy_mesh_no_hops()}
                    onOpenNode={props.onOpenNode}
                    onChange={(via) => update(index, { ...rule, via })}
                  />
                </span>
              </RuleRow>
            ),
          }
        })}
        add={
          editable ? (
            <AddRule
              label={m.policy_mesh_add()}
              onAdd={() => {
                const id = freshId(policy, "route")
                edit((current) => ({
                  ...current,
                  mesh: [...current.mesh, { id, src: "*", dst: "*", via: [] }],
                }))
                rows.added()
              }}
            />
          ) : undefined
        }
      />
    </div>
  )
}
