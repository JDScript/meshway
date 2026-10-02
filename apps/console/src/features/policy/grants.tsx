import type { Grant } from "@meshway/control-client"
import { ArrowLeftRightIcon, ArrowRightIcon } from "lucide-react"

import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
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
import { SelectorSetField } from "./selectors"

// Mesh selectors only: the controller does not yet save grants to the
// internet, IP ranges or domains. Ones already in the source are shown
// and can be removed.
const KINDS = ["all", "tag", "node"] as const

/** Who may reach whom: allows only, added up; what no grant allows is denied. */
export function GrantsLayer(props: LayerProps) {
  const { policy, editable, names, edit, rule: linked, onRule } = props
  const rules = policy.grants
  const rows = useRuleRows(rules, linked, onRule)
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
        <p className="flex h-11 items-center rounded-surface bg-fill-subtle px-4 text-row text-muted-foreground">
          {m.policy_grants_none()}
        </p>
      ) : null}
      <RuleList label={m.policy_layer_grants()}>
        {rules.map((rule, index) => {
          const invalidAt = (field: "src" | "dst") => (at: number) =>
            notesOn(props.diagnostics, "grants", rule.id, field, at).length > 0
          return (
            <li key={rows.key(index)}>
              <RuleRow
                current={rows.current(index)}
                added={rows.autoOpen(index)}
                label={rule.id}
                notes={notesOn(props.diagnostics, "grants", rule.id)}
                names={names}
                end={
                  <>
                    <RuleIdInput
                      value={rule.id}
                      invalid={idInvalid(props.problems, "grants", index)}
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
                            grants: current.grants.filter(
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
                <SelectorSetField
                  values={rule.src}
                  names={names}
                  kinds={KINDS}
                  label={m.policy_grant_src()}
                  addLabel={m.policy_grant_src_add()}
                  disabled={!editable}
                  invalid={invalidAt("src")}
                  autoOpen={rows.autoOpen(index)}
                  onClosed={rows.settled}
                  onOpenNode={props.onOpenNode}
                  onChange={(src) => update(index, { ...rule, src })}
                />
                {/* The direction keeps to the destinations when the row wraps. */}
                <span className="inline-flex min-w-0 flex-wrap items-center gap-1.5">
                  <Direction
                    both={rule.bidirectional}
                    editable={editable}
                    onChange={(bidirectional) =>
                      update(index, { ...rule, bidirectional })
                    }
                  />
                  <SelectorSetField
                    values={rule.dst}
                    names={names}
                    kinds={KINDS}
                    label={m.policy_grant_dst()}
                    addLabel={m.policy_grant_dst_add()}
                    disabled={!editable}
                    invalid={invalidAt("dst")}
                    onOpenNode={props.onOpenNode}
                    onChange={(dst) => update(index, { ...rule, dst })}
                  />
                </span>
              </RuleRow>
            </li>
          )
        })}
      </RuleList>
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
            rows.added()
          }}
        />
      ) : null}
    </div>
  )
}

/** One way or both, between a grant's sources and destinations; pressing it turns it round. */
function Direction({
  both,
  editable,
  onChange,
}: {
  both: boolean
  editable: boolean
  onChange: (both: boolean) => void
}) {
  const Icon = both ? ArrowLeftRightIcon : ArrowRightIcon
  const label = both ? m.policy_grant_both() : m.policy_grant_to()
  if (!editable)
    return (
      <Icon
        aria-label={label}
        className="size-3.5 shrink-0 text-muted-foreground"
      />
    )
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <button
            type="button"
            aria-pressed={both}
            aria-label={m.policy_grant_bidirectional()}
            onClick={() => onChange(!both)}
            className="inline-flex size-7 shrink-0 items-center justify-center rounded-full text-muted-foreground transition-colors outline-none hover:bg-fill-hover hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring aria-pressed:text-foreground"
          />
        }
      >
        <Icon className="size-3.5" />
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  )
}
