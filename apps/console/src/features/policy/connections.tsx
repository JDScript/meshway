import type { ConnectionRule } from "@meshway/control-client"
import { cn } from "cn"
import { ArrowLeftRightIcon } from "lucide-react"

import { SegmentedControl } from "@/components/chrome/segmented-control"
import { m } from "@/paraglide/messages.js"

import { freshId } from "./document"
import {
  idInvalid,
  useOpenRule,
  notesOn,
  replaceAt,
  type LayerProps,
} from "./layer"
import {
  AddRule,
  Fact,
  LayerFacts,
  NoteCounts,
  RuleField,
  RuleFooter,
  RuleGrid,
  RuleIdField,
  RuleItem,
  RuleNotes,
} from "./rules"
import { NodeLink, SelectorChip, SelectorField } from "./selectors"

const KINDS = ["all", "tag", "node"] as const

export function ActionBadge({ action }: { action: ConnectionRule["action"] }) {
  return (
    <span
      // The size stays outside `cn`, which would take `text-meta` for a colour.
      className={`text-meta ${cn(
        "inline-flex h-5 shrink-0 items-center rounded-full px-2 font-medium",
        action === "block"
          ? "bg-destructive/12 text-destructive"
          : "bg-success/14 text-success"
      )}`}
    >
      {action === "block"
        ? m.policy_connection_block()
        : m.policy_connection_allow()}
    </span>
  )
}

/** Which pairs of nodes may connect: allows add up, any matching block wins; order means nothing. */
export function ConnectionsLayer(props: LayerProps) {
  const { policy, editable, names, edit, rule: initial, onRule } = props
  const rules = policy.connections
  const opened = useOpenRule(rules, initial, onRule)
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
        <p className="flex h-14 items-center rounded-row bg-fill-subtle px-3.5 text-row text-muted-foreground">
          {m.policy_connections_none()}
        </p>
      ) : null}
      <RuleGrid label={m.policy_layer_connections()}>
        {rules.map((rule, index) => {
          const open = opened.open === index
          const notes = notesOn(props.diagnostics, "connections", rule.id)
          const fieldId = `connection-${index}`
          return (
            <li
              key={opened.key(index)}
              className={cn(open && "@2xl/workspace:col-span-2")}
            >
              <RuleItem
                open={open}
                label={rule.id}
                onToggle={() => opened.toggle(index)}
                summary={
                  <>
                    <span className="flex min-w-0 flex-wrap items-center gap-1.5">
                      <SelectorChip selector={rule.a} names={names} />
                      <ArrowLeftRightIcon
                        aria-label={m.policy_connection_between()}
                        className="size-3.5 shrink-0 text-muted-foreground"
                      />
                      <SelectorChip selector={rule.b} names={names} />
                    </span>
                    <span className="truncate font-mono text-meta text-muted-foreground">
                      {rule.id}
                    </span>
                  </>
                }
                aside={
                  <span className="flex flex-col items-end gap-1.5">
                    <ActionBadge action={rule.action} />
                    <NoteCounts notes={notes} />
                  </span>
                }
              >
                <RuleNotes notes={notes} names={names} />
                <RuleIdField
                  id={`${fieldId}-id`}
                  value={rule.id}
                  invalid={idInvalid(props.problems, "connections", index)}
                  disabled={!editable}
                  onChange={(id) => {
                    update(index, { ...rule, id })
                    opened.renamed(id)
                  }}
                />
                <RuleField label={m.policy_connection_ends()}>
                  <SelectorField
                    value={rule.a}
                    names={names}
                    kinds={KINDS}
                    label={m.policy_connection_a()}
                    disabled={!editable}
                    invalid={
                      notesOn(props.diagnostics, "connections", rule.id, "a")
                        .length > 0
                    }
                    onChange={(a) => update(index, { ...rule, a })}
                  />
                  <ArrowLeftRightIcon
                    aria-hidden="true"
                    className="size-3.5 text-muted-foreground"
                  />
                  <SelectorField
                    value={rule.b}
                    names={names}
                    kinds={KINDS}
                    label={m.policy_connection_b()}
                    disabled={!editable}
                    invalid={
                      notesOn(props.diagnostics, "connections", rule.id, "b")
                        .length > 0
                    }
                    onChange={(b) => update(index, { ...rule, b })}
                  />
                </RuleField>
                <RuleField label={m.policy_connection_action()}>
                  {editable ? (
                    <SegmentedControl
                      label={m.policy_connection_action()}
                      value={rule.action}
                      options={[
                        { value: "allow", label: m.policy_connection_allow() },
                        { value: "block", label: m.policy_connection_block() },
                      ]}
                      onChange={(action) => update(index, { ...rule, action })}
                    />
                  ) : (
                    <ActionBadge action={rule.action} />
                  )}
                </RuleField>
                <RuleFooter
                  onRemove={
                    editable
                      ? () => {
                          edit((current) => ({
                            ...current,
                            connections: current.connections.filter(
                              (_, at) => at !== index
                            ),
                          }))
                          opened.removed(index)
                        }
                      : undefined
                  }
                >
                  <NodeLink
                    selector={rule.a}
                    names={names}
                    onOpenNode={props.onOpenNode}
                  />
                  <NodeLink
                    selector={rule.b}
                    names={names}
                    onOpenNode={props.onOpenNode}
                  />
                </RuleFooter>
              </RuleItem>
            </li>
          )
        })}
      </RuleGrid>
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
            opened.added(rules.length, id)
          }}
        />
      ) : null}
    </div>
  )
}
