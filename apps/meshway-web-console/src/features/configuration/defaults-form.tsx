import { useQuery } from "@tanstack/react-query"

import { Section } from "@/components/chrome/section"
import { transportCatalogQuery } from "@/features/nodes/queries"
import { m } from "@/paraglide/messages.js"

import { DIALER_PROTOCOLS, type DialerProtocol } from "./dialer-model"
import { DialerSetForm } from "./dialer-set-form"
import { Notes, notesWithin, type Note } from "./form-kit"
import { pointerOf } from "./jsonc"
import type { DefaultsValue } from "./operations"
import { TopologyDefaultControl } from "./topology-form"

/**
 * The defaults (defaults.md): what access does when no rule grants it
 * (`deny`, its one value), what the topology does when no rule matches
 * (the same control as the topology form's, over the same draft value),
 * and the dialers a node without its own set uses.
 */
export function DefaultsForm({
  value,
  access,
  editable,
  notes,
  onChange,
}: {
  value: DefaultsValue
  access: string
  editable: boolean
  notes: readonly Note[]
  onChange: (value: DefaultsValue) => void
}) {
  const catalog = useQuery(transportCatalogQuery())
  const offered: DialerProtocol[] | null = catalog.data
    ? DIALER_PROTOCOLS.filter((protocol) =>
        catalog.data.items.some((item) => item.protocol === protocol)
      )
    : null

  return (
    <div className="flex flex-col gap-8">
      <Section title={m.configuration_defaults_access()}>
        <div className="flex min-h-12 items-center justify-between gap-3 rounded-tile bg-fill-subtle py-2 pr-3.5 pl-3.5">
          <span className="text-row font-medium">
            {m.configuration_defaults_access_unmatched()}
          </span>
          <span className="text-row">
            {access === "deny"
              ? m.configuration_defaults_access_deny()
              : access}
          </span>
        </div>
      </Section>
      <Section title={m.configuration_defaults_topology()}>
        <TopologyDefaultControl
          fallback={value.topology}
          editable={editable}
          onFallback={(topology) => onChange({ ...value, topology })}
        />
      </Section>
      <Section title={m.configuration_defaults_dialers()}>
        <DialerSetForm
          id="defaults-dialers"
          value={value.dialers}
          offered={offered}
          editable={editable}
          onChange={(dialers) => onChange({ ...value, dialers })}
        />
        <Notes notes={notesWithin(notes, pointerOf(["defaults", "dialers"]))} />
        <p className="text-meta text-muted-foreground">
          {m.configuration_defaults_dialers_inherit()}
        </p>
      </Section>
    </div>
  )
}
