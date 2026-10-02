import type { MeshRule } from "@meshway/control-client"
import { ArrowRightIcon, ChevronRightIcon, PlusIcon, XIcon } from "lucide-react"

import { Button } from "@/components/ui/button"
import { m } from "@/paraglide/messages.js"

import { freshId, type PlacedDiagnostic } from "./document"
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
  MoveButtons,
  moveKeys,
  NoteCounts,
  OrderedRules,
  RuleField,
  RuleFooter,
  RuleIdField,
  RuleItem,
  RuleNotes,
} from "./rules"
import {
  NodeLink,
  SelectorChip,
  SelectorField,
  SelectorPicker,
  selectorShown,
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
  const { policy, editable, names, edit, move, rule: initial, onRule } = props
  const rules = policy.mesh
  const opened = useOpenRule(rules, initial, onRule)
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
        <section className="flex flex-col gap-2">
          <h3 className="text-row font-medium text-muted-foreground">
            {m.policy_mesh_failures({ count: failures.length })}
          </h3>
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
        </section>
      ) : null}
      <OrderedRules
        label={m.policy_layer_mesh()}
        end={m.policy_mesh_end()}
        items={rules.map((rule, index) => {
          const open = opened.open === index
          const notes = notesOn(props.diagnostics, "mesh", rule.id)
          const fieldId = `mesh-${index}`
          const onMove = (delta: -1 | 1) => {
            move(["mesh"], index, delta)
            opened.moved(index, delta)
          }
          const hopInvalid = (at: number) =>
            notesOn(props.diagnostics, "mesh", rule.id, "via", at).length > 0
          return {
            // A moved rule keeps its node, so it glides.
            key: opened.key(index),
            node: (
              <RuleItem
                open={open}
                label={rule.id}
                onToggle={() => opened.toggle(index)}
                onKeyDown={
                  editable ? moveKeys(index, rules.length, onMove) : undefined
                }
                summary={
                  <>
                    <span className="flex min-w-0 flex-wrap items-center gap-1.5">
                      <SelectorChip selector={rule.src} names={names} />
                      <ArrowRightIcon
                        aria-label={m.policy_grant_to()}
                        className="size-3.5 shrink-0 text-muted-foreground"
                      />
                      <SelectorChip selector={rule.dst} names={names} />
                      <span className="truncate font-mono text-meta text-muted-foreground">
                        {rule.id}
                      </span>
                    </span>
                    <HopChain via={rule.via} names={names} />
                  </>
                }
                aside={<NoteCounts notes={notes} />}
                actions={
                  editable ? (
                    <MoveButtons
                      index={index}
                      count={rules.length}
                      label={rule.id}
                      onMove={onMove}
                    />
                  ) : undefined
                }
              >
                <RuleNotes notes={notes} names={names} />
                <RuleIdField
                  id={`${fieldId}-id`}
                  value={rule.id}
                  invalid={idInvalid(props.problems, "mesh", index)}
                  disabled={!editable}
                  onChange={(id) => {
                    update(index, { ...rule, id })
                    opened.renamed(id)
                  }}
                />
                <RuleField label={m.policy_mesh_flow()}>
                  <SelectorField
                    value={rule.src}
                    names={names}
                    kinds={ENDS}
                    label={m.policy_mesh_src()}
                    disabled={!editable}
                    invalid={
                      notesOn(props.diagnostics, "mesh", rule.id, "src")
                        .length > 0
                    }
                    onChange={(src) => update(index, { ...rule, src })}
                  />
                  <ArrowRightIcon
                    aria-hidden="true"
                    className="size-3.5 text-muted-foreground"
                  />
                  <SelectorField
                    value={rule.dst}
                    names={names}
                    kinds={ENDS}
                    label={m.policy_mesh_dst()}
                    disabled={!editable}
                    invalid={
                      notesOn(props.diagnostics, "mesh", rule.id, "dst")
                        .length > 0
                    }
                    onChange={(dst) => update(index, { ...rule, dst })}
                  />
                </RuleField>
                <RuleField label={m.policy_mesh_hops()}>
                  <HopsEditor
                    via={rule.via}
                    names={names}
                    editable={editable}
                    invalid={hopInvalid}
                    onChange={(via) => update(index, { ...rule, via })}
                  />
                </RuleField>
                <RuleFooter
                  onRemove={
                    editable
                      ? () => {
                          edit((current) => ({
                            ...current,
                            mesh: current.mesh.filter((_, at) => at !== index),
                          }))
                          opened.removed(index)
                        }
                      : undefined
                  }
                >
                  {[rule.src, rule.dst, ...rule.via].map((selector, at) => (
                    <NodeLink
                      key={`${selector}-${at}`}
                      selector={selector}
                      names={names}
                      onOpenNode={props.onOpenNode}
                    />
                  ))}
                </RuleFooter>
              </RuleItem>
            ),
          }
        })}
      />
      {editable ? (
        <AddRule
          label={m.policy_mesh_add()}
          onAdd={() => {
            const id = freshId(policy, "route")
            edit((current) => ({
              ...current,
              mesh: [...current.mesh, { id, src: "*", dst: "*", via: [] }],
            }))
            opened.added(rules.length, id)
          }}
        />
      ) : null}
    </div>
  )
}

/** The ordered mandatory hops: each one can move, go, or another can be added at the end. */
function HopsEditor({
  via,
  names,
  editable,
  invalid,
  onChange,
}: {
  via: readonly string[]
  names: SelectorNames
  editable: boolean
  invalid: (index: number) => boolean
  onChange: (via: string[]) => void
}) {
  const swap = (index: number, delta: -1 | 1) => {
    const next = [...via]
    const other = index + delta
    const a = next[index]
    const b = next[other]
    if (a === undefined || b === undefined) return
    next[index] = b
    next[other] = a
    onChange(next)
  }
  return (
    <ol className="flex w-full flex-col gap-1">
      {via.length === 0 ? (
        <li className="flex h-7 items-center text-row text-muted-foreground">
          {m.policy_mesh_no_hops()}
        </li>
      ) : null}
      {via.map((hop, index) => {
        const name = selectorShown(hop, names).label
        return (
          <li
            key={`${hop}-${index}`}
            aria-invalid={invalid(index) || undefined}
            className="flex h-9 items-center gap-2 rounded-tile bg-fill-subtle pr-1 pl-2.5 aria-invalid:ring-2 aria-invalid:ring-destructive/60"
          >
            <span className="w-4 font-mono text-meta text-muted-foreground tabular-nums">
              {index + 1}
            </span>
            <SelectorChip selector={hop} names={names} />
            <span className="flex-1" />
            {editable ? (
              <>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-xs"
                  className="rounded-full text-muted-foreground"
                  disabled={index === 0}
                  aria-label={m.policy_move_up({ name })}
                  onClick={() => swap(index, -1)}
                >
                  <ArrowRightIcon className="-rotate-90" />
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-xs"
                  className="rounded-full text-muted-foreground"
                  disabled={index === via.length - 1}
                  aria-label={m.policy_move_down({ name })}
                  onClick={() => swap(index, 1)}
                >
                  <ArrowRightIcon className="rotate-90" />
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-xs"
                  className="rounded-full text-muted-foreground"
                  aria-label={m.policy_selector_remove({ name })}
                  onClick={() => onChange(via.filter((_, at) => at !== index))}
                >
                  <XIcon />
                </Button>
              </>
            ) : null}
          </li>
        )
      })}
      {editable ? (
        <li>
          <SelectorPicker
            names={names}
            kinds={HOPS}
            chosen={via}
            label={m.policy_mesh_hop_add()}
            onPick={(hop) => onChange([...via, hop])}
          >
            <span className="inline-flex h-7 items-center gap-1 rounded-full border border-dashed border-divider px-2.5 text-row text-muted-foreground transition-colors hover:bg-fill-hover hover:text-foreground">
              <PlusIcon className="size-3" />
              {m.policy_mesh_hop_add()}
            </span>
          </SelectorPicker>
        </li>
      ) : null}
    </ol>
  )
}
