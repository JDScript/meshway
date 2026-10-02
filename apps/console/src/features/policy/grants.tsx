import type { Grant } from "@meshway/control-client"
import { cn } from "cn"
import { ArrowLeftRightIcon, ArrowRightIcon } from "lucide-react"

import { Switch } from "@/components/ui/switch"
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
import { SelectorGroup, SelectorSetField } from "./selectors"

// Mesh selectors only: the controller does not yet save grants to the
// internet, IP ranges or domains. Ones already in the source are shown
// and can be removed.
const KINDS = ["all", "tag", "node"] as const

/** Who may reach whom: allows only, added up; what no grant allows is denied. */
export function GrantsLayer(props: LayerProps) {
  const { policy, editable, names, edit, rule: initial, onRule } = props
  const rules = policy.grants
  const opened = useOpenRule(rules, initial, onRule)
  const update = (index: number, next: Grant) =>
    edit((current) => ({
      ...current,
      grants: replaceAt(current.grants, index, next),
    }))

  return (
    <div className="flex flex-col gap-4">
      <LayerFacts>
        <Fact>{m.policy_count_rules({ count: rules.length })}</Fact>
        <Fact>{m.policy_fact_additive()}</Fact>
        <Fact>{m.policy_fact_unmatched_denied()}</Fact>
      </LayerFacts>
      {rules.length === 0 ? (
        <p className="flex h-14 items-center rounded-row bg-fill-subtle px-3.5 text-row text-muted-foreground">
          {m.policy_grants_none()}
        </p>
      ) : null}
      <RuleGrid label={m.policy_layer_grants()}>
        {rules.map((rule, index) => {
          const open = opened.open === index
          const notes = notesOn(props.diagnostics, "grants", rule.id)
          const fieldId = `grant-${index}`
          const Arrow = rule.bidirectional ? ArrowLeftRightIcon : ArrowRightIcon
          const invalidAt = (field: "src" | "dst") => (at: number) =>
            notesOn(props.diagnostics, "grants", rule.id, field, at).length > 0
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
                      <SelectorGroup selectors={rule.src} names={names} />
                      <Arrow
                        aria-label={
                          rule.bidirectional
                            ? m.policy_grant_both()
                            : m.policy_grant_to()
                        }
                        className="size-3.5 shrink-0 text-muted-foreground"
                      />
                      <SelectorGroup selectors={rule.dst} names={names} />
                    </span>
                    <span className="truncate font-mono text-meta text-muted-foreground">
                      {rule.id}
                    </span>
                  </>
                }
                aside={<NoteCounts notes={notes} />}
              >
                <RuleNotes notes={notes} names={names} />
                <RuleIdField
                  id={`${fieldId}-id`}
                  value={rule.id}
                  invalid={idInvalid(props.problems, "grants", index)}
                  disabled={!editable}
                  onChange={(id) => {
                    update(index, { ...rule, id })
                    opened.renamed(id)
                  }}
                />
                <RuleField label={m.policy_grant_src()}>
                  <SelectorSetField
                    values={rule.src}
                    names={names}
                    kinds={KINDS}
                    label={m.policy_grant_src_add()}
                    disabled={!editable}
                    invalid={invalidAt("src")}
                    onChange={(src) => update(index, { ...rule, src })}
                  />
                </RuleField>
                <RuleField label={m.policy_grant_dst()}>
                  <SelectorSetField
                    values={rule.dst}
                    names={names}
                    kinds={KINDS}
                    label={m.policy_grant_dst_add()}
                    disabled={!editable}
                    invalid={invalidAt("dst")}
                    onChange={(dst) => update(index, { ...rule, dst })}
                  />
                </RuleField>
                <RuleField
                  label={m.policy_grant_bidirectional()}
                  htmlFor={`${fieldId}-both`}
                >
                  <Switch
                    id={`${fieldId}-both`}
                    checked={rule.bidirectional}
                    disabled={!editable}
                    onCheckedChange={(bidirectional) =>
                      update(index, { ...rule, bidirectional })
                    }
                  />
                </RuleField>
                <RuleFooter
                  onRemove={
                    editable
                      ? () => {
                          edit((current) => ({
                            ...current,
                            grants: current.grants.filter(
                              (_, at) => at !== index
                            ),
                          }))
                          opened.removed(index)
                        }
                      : undefined
                  }
                />
              </RuleItem>
            </li>
          )
        })}
      </RuleGrid>
      {editable ? (
        <AddRule
          label={m.policy_grant_add()}
          onAdd={() => {
            const id = freshId(policy, "grant")
            edit((current) => ({
              ...current,
              grants: [
                ...current.grants,
                { id, src: ["*"], dst: ["*"], bidirectional: false },
              ],
            }))
            opened.added(rules.length, id)
          }}
        />
      ) : null}
    </div>
  )
}
