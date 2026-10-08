import type { NodeInventoryView, StateView } from "@meshway/control-client"

import { CopyButton } from "@/components/chrome/copy-button"
import { FactList, FactRow } from "@/components/chrome/fact-list"
import { Section } from "@/components/chrome/section"
import { useNow } from "@/hooks/use-now"
import { agoTime, dateTime } from "@/lib/format"
import { fullReport, stateOf } from "@/lib/nodes/state-model"
import { m } from "@/paraglide/messages.js"

/** Registration and device facts, separate from everyday connection status. */
export function NodeDevice({
  node,
  state,
}: {
  node: NodeInventoryView
  state: StateView | undefined
}) {
  const report = fullReport(stateOf(state, node.id))
  const now = useNow(1_000)
  const facts = [
    { label: m.nodes_id(), value: node.id, copy: true },
    {
      label:
        node.identity.type === "tag"
          ? m.nodes_identity_tags()
          : m.nodes_identity_user(),
      value:
        node.identity.type === "tag"
          ? node.identity.tags.join(", ")
          : node.identity.user,
    },
    {
      label: m.nodes_system_os(),
      value: report ? `${report.system.os} · ${report.system.arch}` : null,
    },
    { label: m.nodes_system_agent(), value: report?.system.agentVersion },
    { label: m.nodes_system_hostname(), value: report?.system.hostname },
    {
      label: m.nodes_system_selection(),
      value: report
        ? (report.selection.appliedProfile ?? m.nodes_system_selection_none())
        : null,
    },
    {
      label: m.nodes_registered(),
      value: `${dateTime(node.registeredAt)} · ${agoTime(node.registeredAt, now)}`,
    },
  ]
  return (
    <Section title={m.nodes_facts()}>
      <FactList>
        {facts.map(({ label, value, copy }) => (
          <FactRow
            key={label}
            label={label}
            action={
              copy && value ? <CopyButton label={label} value={value} /> : null
            }
          >
            <span title={value ?? undefined}>{value ?? "—"}</span>
          </FactRow>
        ))}
      </FactList>
    </Section>
  )
}
