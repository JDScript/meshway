import {
  previewFlow,
  type FlowAuthorizationAction,
  type FlowQuery,
  type PreviewFlowResponse,
  type RouteDecisionAction,
} from "@meshway/control-client"
import { zFlowQuery } from "@meshway/control-client/schemas"
import { useMutation } from "@tanstack/react-query"
import { ChevronDownIcon, ChevronRightIcon } from "lucide-react"
import { useState, type ReactNode } from "react"

import { FieldInput } from "@/components/chrome/form"
import { SegmentedControl } from "@/components/chrome/segmented-control"
import { Button } from "@/components/ui/button"
import { Spinner } from "@/components/ui/spinner"
import { useNetwork } from "@/features/network/use-network-view"
import { authed } from "@/lib/api"
import { toastFailure } from "@/lib/failure"
import { m } from "@/paraglide/messages.js"

import { SelectorChip, withNodeNames, type SelectorNames } from "./selectors"

const SELECT =
  "h-10 w-full min-w-0 appearance-none rounded-tile border-transparent bg-field pr-9 pl-3.5 text-base outline-none md:text-row focus-visible:ring-2 focus-visible:ring-ring aria-invalid:ring-2 aria-invalid:ring-destructive/60"

const AUTHORIZATION: Record<FlowAuthorizationAction, () => string> = {
  allow: () => m.policy_flow_allow(),
  reject: () => m.policy_flow_reject(),
  invalid: () => m.policy_flow_invalid(),
}
const ROUTING: Record<RouteDecisionAction, () => string> = {
  route: () => m.policy_flow_route(),
  reject: () => m.policy_flow_reject(),
  invalid: () => m.policy_flow_invalid(),
}
const tone = (action: string) =>
  action === "allow" || action === "route"
    ? "text-success"
    : action === "reject"
      ? "text-destructive"
      : "text-warning"

/**
 * Asks the controller how it would treat one flow under the saved source:
 * authorization and the planned path. Run on request only; the answer is
 * the compiled plan, not a trace of packets.
 */
export function FlowPreview({
  networkId,
  names,
}: {
  networkId: string
  names: SelectorNames
}) {
  const { inventory } = useNetwork()
  const [to, setTo] = useState<"node" | "ip">("node")
  const [invalid, setInvalid] = useState<string | null>(null)
  const mutation = useMutation({
    mutationFn: (flow: FlowQuery) =>
      authed((base, token) => previewFlow(base, token, networkId, { flow })),
    onError: toastFailure,
  })

  return (
    <section
      aria-labelledby="policy-flow-title"
      className="flex flex-col gap-3 rounded-row border border-divider p-3.5"
    >
      <header className="flex min-w-0 flex-wrap items-baseline gap-x-3 gap-y-1">
        <h3 id="policy-flow-title" className="text-row font-medium">
          {m.policy_flow_title()}
        </h3>
        <span className="text-meta text-muted-foreground">
          {m.policy_flow_fact()}
        </span>
      </header>
      <form
        className="grid grid-cols-1 gap-3 @2xl/workspace:grid-cols-[1fr_1fr_1fr_auto] @2xl/workspace:items-end"
        onSubmit={(event) => {
          event.preventDefault()
          const data = new FormData(event.currentTarget)
          const text = (name: string) => {
            const value = data.get(name)
            return typeof value === "string" && value.trim() !== ""
              ? value.trim()
              : null
          }
          const flow = zFlowQuery.safeParse({
            source_node_id: text("flow-source") ?? "",
            destination_node_id: to === "node" ? text("flow-node") : null,
            destination_ip: to === "ip" ? text("flow-ip") : null,
            destination_country: null,
            domain: text("flow-domain"),
          })
          if (
            !flow.success ||
            flow.data.source_node_id === "" ||
            (to === "node"
              ? flow.data.destination_node_id === null
              : flow.data.destination_ip === null)
          ) {
            setInvalid(
              !flow.success
                ? "flow"
                : flow.data.source_node_id === ""
                  ? "source"
                  : "destination"
            )
            return
          }
          setInvalid(null)
          mutation.mutate(flow.data)
        }}
      >
        <label className="flex min-w-0 flex-col gap-1.5">
          <span className="flex h-7 items-center text-meta text-muted-foreground">
            {m.policy_flow_source()}
          </span>
          <NodeSelect
            name="flow-source"
            invalid={invalid === "source"}
            nodes={inventory}
          />
        </label>
        <div className="flex min-w-0 flex-col gap-1.5">
          <span className="flex h-7 items-center justify-between gap-2 text-meta text-muted-foreground">
            {m.policy_flow_destination()}
            <SegmentedControl
              label={m.policy_flow_destination()}
              value={to}
              className="origin-right scale-[0.85]"
              options={[
                { value: "node", label: m.policy_flow_to_node() },
                { value: "ip", label: m.policy_flow_to_ip() },
              ]}
              onChange={setTo}
            />
          </span>
          {to === "node" ? (
            <NodeSelect
              name="flow-node"
              invalid={invalid === "destination"}
              nodes={inventory}
            />
          ) : (
            <FieldInput
              name="flow-ip"
              inputMode="decimal"
              autoComplete="off"
              spellCheck={false}
              placeholder="100.96.0.2"
              className="font-mono"
              aria-invalid={invalid === "destination" || undefined}
            />
          )}
        </div>
        <label className="flex min-w-0 flex-col gap-1.5">
          <span className="flex h-7 items-center text-meta text-muted-foreground">
            {m.policy_flow_domain()}
          </span>
          <FieldInput
            name="flow-domain"
            autoComplete="off"
            autoCapitalize="none"
            spellCheck={false}
            placeholder="example.com"
            className="font-mono"
          />
        </label>
        <Button
          type="submit"
          className="h-10 rounded-full px-4"
          disabled={mutation.isPending}
          aria-busy={mutation.isPending}
        >
          {mutation.isPending ? <Spinner /> : null}
          {m.policy_flow_run()}
        </Button>
      </form>
      <FlowResult result={mutation.data ?? null} names={names} />
    </section>
  )
}

/** A node, by name: a native select (it suits phones), on the field fill. */
function NodeSelect({
  name,
  invalid,
  nodes,
}: {
  name: string
  invalid: boolean
  nodes: readonly { id: string; name: string }[]
}) {
  return (
    <span className="relative flex">
      <select
        name={name}
        className={SELECT}
        aria-invalid={invalid || undefined}
        defaultValue=""
      >
        <option value="" disabled>
          {m.policy_flow_pick()}
        </option>
        {nodes.map((node) => (
          <option key={node.id} value={node.id}>
            {node.name}
          </option>
        ))}
      </select>
      <ChevronDownIcon
        aria-hidden="true"
        className="pointer-events-none absolute top-1/2 right-3 size-4 -translate-y-1/2 text-muted-foreground"
      />
    </span>
  )
}

/** Both decisions side by side; the slots stay while nothing has been asked yet. */
function FlowResult({
  result,
  names,
}: {
  result: PreviewFlowResponse | null
  names: SelectorNames
}) {
  return (
    <div className="grid grid-cols-1 gap-2 @2xl/workspace:grid-cols-2">
      <Decision
        label={m.policy_flow_authorization()}
        action={result ? AUTHORIZATION[result.authorization.action]() : null}
        tone={result ? tone(result.authorization.action) : ""}
        rule={result?.authorization.rule_id ?? null}
        reason={
          result ? withNodeNames(result.authorization.reason, names) : null
        }
      >
        {result?.authorization.matched_direction ? (
          <span className="text-meta text-muted-foreground">
            {result.authorization.matched_direction === "reverse"
              ? m.policy_flow_reverse()
              : m.policy_flow_forward()}
          </span>
        ) : null}
      </Decision>
      <Decision
        label={m.policy_flow_routing()}
        action={result ? ROUTING[result.routing.action]() : null}
        tone={result ? tone(result.routing.action) : ""}
        rule={result?.routing.rule_id ?? null}
        reason={result ? withNodeNames(result.routing.reason, names) : null}
      >
        {result && result.routing.path.length > 0 ? (
          <span className="flex min-w-0 flex-wrap items-center gap-1">
            {result.routing.path.map((nodeId, index) => (
              <span
                key={`${nodeId}-${index}`}
                className="inline-flex items-center gap-1"
              >
                {index > 0 ? (
                  <ChevronRightIcon
                    aria-hidden="true"
                    className="size-3 text-muted-foreground"
                  />
                ) : null}
                <SelectorChip selector={`node:${nodeId}`} names={names} />
              </span>
            ))}
          </span>
        ) : null}
        {result?.routing.exit_node_id ? (
          <span className="flex items-center gap-1.5 text-meta text-muted-foreground">
            {m.policy_egress_exit()}
            <SelectorChip
              selector={`node:${result.routing.exit_node_id}`}
              names={names}
            />
          </span>
        ) : null}
      </Decision>
      {result ? (
        <p className="text-meta text-muted-foreground @2xl/workspace:col-span-2">
          {m.policy_flow_revision({ revision: result.revision })}
          {" · "}
          {result.execution_supported
            ? m.policy_flow_executable()
            : m.policy_flow_not_executable()}
          {" · "}
          {result.execution_reason}
        </p>
      ) : null}
    </div>
  )
}

function Decision({
  label,
  action,
  tone,
  rule,
  reason,
  children,
}: {
  label: string
  action: string | null
  tone: string
  rule: string | null
  reason: string | null
  children?: ReactNode
}) {
  return (
    <div className="flex min-h-24 flex-col gap-1.5 rounded-tile bg-fill-subtle px-3.5 py-3">
      <span className="flex items-baseline justify-between gap-2 text-meta text-muted-foreground">
        {label}
        {rule ? <code className="font-mono">{rule}</code> : null}
      </span>
      <span
        className={`text-lead font-semibold ${action ? tone : "text-muted-foreground"}`}
      >
        {action ?? "—"}
      </span>
      {children}
      {reason ? (
        <span className="text-meta break-words text-muted-foreground">
          {reason}
        </span>
      ) : null}
    </div>
  )
}
