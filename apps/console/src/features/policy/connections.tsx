import type { ConnectionRule } from "@meshway/control-client"
import { ArrowLeftRightIcon } from "lucide-react"

import { SegmentedControl } from "@/components/chrome/segmented-control"
import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"

import { freshId } from "./document"
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
  RemoveRule,
  RuleIdInput,
  RuleList,
  RuleRow,
} from "./rules"
import { SelectorField } from "./selectors"

const KINDS = ["all", "tag", "node"] as const

function ActionBadge({ action }: { action: ConnectionRule["action"] }) {
  return (
    <span
      className={cn(
        "inline-flex h-6 shrink-0 items-center rounded-full px-2.5 text-meta font-medium",
        action === "block"
          ? "bg-destructive/12 text-destructive"
          : "bg-success/14 text-success"
      )}
    >
      {action === "block"
        ? m.policy_connection_block()
        : m.policy_connection_allow()}
    </span>
  )
}

/** Which pairs of nodes may connect: allows add up, any matching block wins; order means nothing. */
export function ConnectionsLayer(props: LayerProps) {
  const { policy, editable, names, edit, rule: linked, onRule } = props
  const rules = policy.connections
  const rows = useRuleRows(rules, linked, onRule)
  const blocks = rules.filter((rule) => rule.action === "block").length
  const update = (index: number, next: ConnectionRule) =>
    edit((current) => ({
      ...current,
      connections: replaceAt(current.connections, index, next),
    }))

  return (
    <div className="flex flex-col gap-4">
      <LayerFacts>
        <Fact>{m.policy_count_rules({ count: rules.length })}</Fact>
        <Fact>{m.policy_fact_block_wins()}</Fact>
        <Fact>{m.policy_fact_unordered()}</Fact>
        {blocks > 0 ? (
          <Fact>{m.policy_count_blocks({ count: blocks })}</Fact>
        ) : null}
      </LayerFacts>
      {rules.length === 0 ? (
        <p className="flex h-11 items-center rounded-surface bg-fill-subtle px-4 text-row text-muted-foreground">
          {m.policy_connections_none()}
        </p>
      ) : null}
      <RuleList label={m.policy_layer_connections()}>
        {rules.map((rule, index) => {
          const invalid = (field: "a" | "b") =>
            notesOn(props.diagnostics, "connections", rule.id, field).length > 0
          return (
            <li key={rows.key(index)}>
              <RuleRow
                current={rows.current(index)}
                added={rows.autoOpen(index)}
                label={rule.id}
                notes={notesOn(props.diagnostics, "connections", rule.id)}
                names={names}
                end={
                  <>
                    {editable ? (
                      <SegmentedControl
                        size="sm"
                        label={m.policy_connection_action()}
                        value={rule.action}
                        // The chosen action wears the colour of its read-only badge.
                        options={[
                          {
                            value: "allow",
                            label: (
                              <span
                                className={cn(
                                  rule.action === "allow" && "text-success"
                                )}
                              >
                                {m.policy_connection_allow()}
                              </span>
                            ),
                          },
                          {
                            value: "block",
                            label: (
                              <span
                                className={cn(
                                  rule.action === "block" && "text-destructive"
                                )}
                              >
                                {m.policy_connection_block()}
                              </span>
                            ),
                          },
                        ]}
                        onChange={(action) =>
                          update(index, { ...rule, action })
                        }
                      />
                    ) : (
                      <ActionBadge action={rule.action} />
                    )}
                    <RuleIdInput
                      value={rule.id}
                      invalid={idInvalid(props.problems, "connections", index)}
                      editable={editable}
                      onChange={(id) => {
                        update(index, { ...rule, id })
                        rows.renamed(index, id)
                      }}
                    />
                    {editable ? (
                      <RemoveRule
                        label={rule.id}
                        onRemove={() => {
                          edit((current) => ({
                            ...current,
                            connections: current.connections.filter(
                              (_, at) => at !== index
                            ),
                          }))
                          rows.removed(index)
                        }}
                      />
                    ) : null}
                  </>
                }
              >
                <SelectorField
                  value={rule.a}
                  names={names}
                  kinds={KINDS}
                  label={m.policy_connection_a()}
                  disabled={!editable}
                  invalid={invalid("a")}
                  autoOpen={rows.autoOpen(index)}
                  onClosed={rows.settled}
                  onOpenNode={props.onOpenNode}
                  onChange={(a) => update(index, { ...rule, a })}
                />
                <ArrowLeftRightIcon
                  aria-label={m.policy_connection_between()}
                  className="size-3.5 shrink-0 text-muted-foreground"
                />
                <SelectorField
                  value={rule.b}
                  names={names}
                  kinds={KINDS}
                  label={m.policy_connection_b()}
                  disabled={!editable}
                  invalid={invalid("b")}
                  onOpenNode={props.onOpenNode}
                  onChange={(b) => update(index, { ...rule, b })}
                />
              </RuleRow>
            </li>
          )
        })}
      </RuleList>
      {editable ? (
        <AddRule
          label={m.policy_connection_add()}
          onAdd={() => {
            const id = freshId(policy, "connection")
            edit((current) => ({
              ...current,
              connections: [
                ...current.connections,
                { id, a: "*", b: "*", action: "allow" },
              ],
            }))
            rows.added()
          }}
        />
      ) : null}
    </div>
  )
}
