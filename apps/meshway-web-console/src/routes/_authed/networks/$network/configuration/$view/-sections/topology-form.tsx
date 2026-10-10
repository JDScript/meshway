import type {
  TopologyAction,
  TopologyDefault,
  TopologyRule,
  TransportProtocol,
} from "@meshway/control-client"
import { cn } from "@meshway/ui/cn"
import { ArrowLeftRightIcon } from "lucide-react"

import { Section } from "@/components/chrome/section"
import { SegmentedControl } from "@/components/chrome/segmented-control"
import { ChipField } from "@/components/configuration/chips"
import { protocolLabel } from "@/components/configuration/dialer-set-form"
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
} from "@/components/configuration/form-kit"
import { DIALER_PROTOCOLS } from "@/lib/configuration/dialer-model"
import { pointerOf } from "@/lib/configuration/jsonc"
import { moved } from "@/lib/configuration/routing"
import { m } from "@/paraglide/messages.js"

import { MoveButtons, moveKeys } from "./ordered"
import { Hint } from "./route-kit"

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

/**
 * `defaults.topology`, the one control for it: the topology form and the
 * defaults form both show it, over the same draft value.
 */
export function TopologyDefaultControl({
  fallback,
  editable,
  onFallback,
}: {
  fallback: TopologyDefault
  editable: boolean
  onFallback: (fallback: TopologyDefault) => void
}) {
  return (
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
  )
}

/** The rule without transports: any, for an allow rule. */
function withoutTransports(rule: TopologyRule): TopologyRule {
  const out = { ...rule }
  delete out.transports
  return out
}

/** The rule without `keepalive`: its edge is connected when used. */
function onDemand(rule: TopologyRule): TopologyRule {
  const out = { ...rule }
  delete out.keepalive
  return out
}

/** A rule with `action`: a block rule carries no transports or keepalive (they would have no effect). */
const withAction = (rule: TopologyRule, action: TopologyAction) =>
  action === "allow"
    ? { ...rule, action }
    : onDemand(withoutTransports({ ...rule, action }))

/** `transports` with `protocol` in or out, the others in the order written. */
const toggled = (
  transports: readonly TransportProtocol[],
  protocol: TransportProtocol,
  on: boolean
): TransportProtocol[] =>
  on
    ? [...transports.filter((item) => item !== protocol), protocol]
    : transports.filter((item) => item !== protocol)

const PILL =
  "inline-flex h-6 items-center rounded-full border px-2.5 text-meta font-medium"

/**
 * The transports an allow rule's edge may use: any (omitted, protocols
 * added later included) or only the ones chosen, none chosen being `[]`
 * (the two ends cannot connect directly). A block rule has none to set;
 * one the source gives it is said to have no effect.
 */
function RuleTransports({
  rule,
  editable,
  invalid,
  onChange,
}: {
  rule: TopologyRule
  editable: boolean
  invalid: boolean
  onChange: (transports: TransportProtocol[] | undefined) => void
}) {
  const { transports } = rule
  if (rule.action === "block")
    return transports === undefined ? null : (
      <p className="px-1.5 text-meta text-muted-foreground">
        {m.configuration_topology_transports_block()}
      </p>
    )
  return (
    <div
      role="group"
      aria-label={m.configuration_topology_transports()}
      className="flex min-h-7 flex-wrap items-center gap-1.5 px-1.5"
    >
      <span className="text-meta text-muted-foreground">
        {m.configuration_topology_transports()}
      </span>
      {editable ? (
        <SegmentedControl
          size="sm"
          label={m.configuration_topology_transports()}
          value={transports === undefined ? "any" : "some"}
          options={[
            { value: "any", label: m.configuration_topology_transports_any() },
            {
              value: "some",
              label: m.configuration_topology_transports_some(),
            },
          ]}
          onChange={(mode) =>
            onChange(mode === "any" ? undefined : [...DIALER_PROTOCOLS])
          }
        />
      ) : null}
      {transports !== undefined ? (
        <ol
          className={cn(
            "inline-flex flex-wrap items-center gap-1.5 rounded-full",
            invalid && "ring-1 ring-destructive/60"
          )}
        >
          {transports.map((protocol, index) => (
            <li
              key={protocol}
              className="flex items-center gap-0.5 rounded-full border border-border bg-fill-selected"
              onKeyDown={
                editable
                  ? moveKeys(index, transports.length, (delta) =>
                      onChange(moved(transports, index, delta))
                    )
                  : undefined
              }
            >
              {editable ? (
                <button
                  type="button"
                  aria-pressed
                  className="inline-flex h-6 items-center gap-1.5 rounded-full px-2.5 text-meta font-medium outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  onClick={() => onChange(toggled(transports, protocol, false))}
                >
                  <span className="text-muted-foreground tabular-nums">
                    {index + 1}
                  </span>
                  {protocolLabel[protocol]()}
                </button>
              ) : (
                <span className="inline-flex h-6 items-center gap-1.5 px-2.5 text-meta">
                  <span className="text-muted-foreground tabular-nums">
                    {index + 1}
                  </span>
                  {protocolLabel[protocol]()}
                </span>
              )}
              {editable ? (
                <span className="flex pr-0.5">
                  <MoveButtons
                    index={index}
                    count={transports.length}
                    label={protocolLabel[protocol]()}
                    onMove={(delta) =>
                      onChange(moved(transports, index, delta))
                    }
                  />
                </span>
              ) : null}
            </li>
          ))}
          {editable
            ? DIALER_PROTOCOLS.filter(
                (protocol) => !transports.includes(protocol)
              ).map((protocol) => (
                <li key={protocol}>
                  <button
                    type="button"
                    aria-pressed={false}
                    onClick={() =>
                      onChange(toggled(transports, protocol, true))
                    }
                    className={cn(
                      PILL,
                      "border-dashed border-divider text-muted-foreground outline-none hover:bg-fill-hover focus-visible:ring-2 focus-visible:ring-ring"
                    )}
                  >
                    {protocolLabel[protocol]()}
                  </button>
                </li>
              ))
            : null}
        </ol>
      ) : null}
      {transports === undefined ? (
        <span className="text-meta text-muted-foreground">
          {DIALER_PROTOCOLS.map((protocol) => protocolLabel[protocol]()).join(
            " → "
          )}
        </span>
      ) : null}
      {transports !== undefined && transports.length === 0 ? (
        <span className="text-meta text-warning">
          {m.configuration_topology_transports_none()}
        </span>
      ) : null}
    </div>
  )
}

/**
 * Whether an allow rule's edge is kept connected (`keepalive`) or connected
 * when traffic needs it and closed when idle (omitted). An edge only one end
 * can dial is kept connected anyway; that is the node's, not the rule's.
 */
function RuleKeepalive({
  rule,
  editable,
  onChange,
}: {
  rule: TopologyRule
  editable: boolean
  onChange: (rule: TopologyRule) => void
}) {
  const kept = rule.keepalive === true
  if (rule.action === "block")
    return rule.keepalive === undefined ? null : (
      <p className="px-1.5 text-meta text-muted-foreground">
        {m.configuration_topology_keepalive_block()}
      </p>
    )
  if (!editable && !kept) return null
  return (
    <div className="flex min-h-7 flex-wrap items-center gap-1.5 px-1.5">
      <span className="text-meta text-muted-foreground">
        {m.configuration_topology_keepalive()}
      </span>
      {editable ? (
        <SegmentedControl
          size="sm"
          label={m.configuration_topology_keepalive()}
          value={kept ? "kept" : "demand"}
          options={[
            {
              value: "demand",
              label: m.configuration_topology_keepalive_demand(),
            },
            { value: "kept", label: m.configuration_topology_keepalive_kept() },
          ]}
          onChange={(mode) =>
            onChange(
              mode === "kept" ? { ...rule, keepalive: true } : onDemand(rule)
            )
          }
        />
      ) : (
        <span className="text-meta">
          {m.configuration_topology_keepalive_kept()}
        </span>
      )}
    </div>
  )
}

/**
 * Which pairs of nodes may connect directly: any matching block wins, then
 * the first matching allow, whose transports the edge may use (later ones
 * are not merged in), then the default.
 */
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
      <TopologyDefaultControl
        fallback={fallback}
        editable={editable}
        onFallback={onFallback}
      />
      <Section title={m.configuration_topology_rules()} count={rules.length}>
        <Hint>{m.configuration_topology_hint()}</Hint>
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
                  detail={
                    <>
                      <RuleTransports
                        rule={rule}
                        editable={editable}
                        invalid={refusedAt(
                          notes,
                          pointerOf(["topology", index, "transports"])
                        )}
                        onChange={(transports) =>
                          update(
                            index,
                            transports === undefined
                              ? withoutTransports(rule)
                              : { ...rule, transports }
                          )
                        }
                      />
                      <RuleKeepalive
                        rule={rule}
                        editable={editable}
                        onChange={(next) => update(index, next)}
                      />
                    </>
                  }
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
                            update(index, withAction(rule, action))
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
