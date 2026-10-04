import { ChevronDownIcon } from "lucide-react"
import { useState, type ReactNode } from "react"

import { FIELD_SIZE, TextField } from "@/components/chrome/form"
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible"
import {
  advancedSet,
  protocolOf,
  suggestedPort,
  type ListenerField,
  type ListenerFields,
  type ListenerProblem,
} from "@/lib/nodes/listener-model"
import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"

export const fieldLabelClass = cn(
  "leading-snug font-medium",
  FIELD_SIZE.default.label
)

export const listenerProblemText: Record<ListenerProblem, () => string> = {
  port: () => m.nodes_listener_port_invalid(),
  host: () => m.nodes_listener_host_invalid(),
  public_port: () => m.nodes_listener_public_port_invalid(),
  listen: () => m.nodes_listener_listen_invalid(),
  server_name: () => m.nodes_listener_server_name_invalid(),
  server_name_missing: () => m.nodes_listener_server_name_missing(),
}

/** A listener's reachability inputs: public address, port, TLS name, and under "advanced" the bind address and a public port of its own. */
export function ListenerInputs({
  fields,
  invalid,
  onChange,
  idPrefix = "add-listener",
  generated = true,
  children,
}: {
  fields: ListenerFields
  invalid: ReadonlySet<ListenerField>
  onChange: (fields: ListenerFields) => void
  /** Prefixes the fields' IDs, unique on the page. */
  idPrefix?: string
  /** Says the rest is generated (a new node's listener). */
  generated?: boolean
  /** More inputs under "advanced". */
  children?: ReactNode
}) {
  const set = (key: ListenerField, value: string) =>
    onChange({ ...fields, [key]: value })
  const vless = protocolOf(fields.preset) === "vless"
  // Opens by itself when something inside is wrong; it can still be closed.
  const hidden = invalid.has("listen") || invalid.has("publicPort")
  const [open, setOpen] = useState(advancedSet(fields))
  const [seen, setSeen] = useState(hidden)
  if (hidden !== seen) {
    setSeen(hidden)
    if (hidden) setOpen(true)
  }
  const host = fields.host.trim()

  return (
    <div className="swap-in flex flex-col gap-4">
      <div className="grid grid-cols-[minmax(0,1fr)_6rem] gap-x-2 gap-y-4">
        <TextField
          id={`${idPrefix}-host`}
          name="public-host"
          label={m.nodes_listener_host()}
          autoComplete="off"
          autoCapitalize="none"
          spellCheck={false}
          maxLength={253}
          className="font-mono placeholder:font-sans"
          placeholder={m.nodes_listener_host_placeholder()}
          value={fields.host}
          invalid={invalid.has("host")}
          onChange={(event) => set("host", event.target.value)}
        />
        <TextField
          id={`${idPrefix}-port`}
          name="port"
          label={m.nodes_listener_port()}
          inputMode="numeric"
          autoComplete="off"
          maxLength={5}
          className="font-mono tabular-nums"
          placeholder={suggestedPort(fields.preset)}
          value={fields.port}
          invalid={invalid.has("port")}
          onChange={(event) => set("port", event.target.value.trim())}
        />
        {vless ? (
          <div className="col-span-2">
            <TextField
              id={`${idPrefix}-server-name`}
              name="server-name"
              label={m.nodes_listener_server_name()}
              autoComplete="off"
              autoCapitalize="none"
              spellCheck={false}
              maxLength={253}
              className="font-mono"
              placeholder={host || m.nodes_listener_server_name_placeholder()}
              value={fields.serverName}
              invalid={invalid.has("serverName")}
              onChange={(event) => set("serverName", event.target.value)}
            />
          </div>
        ) : null}
      </div>

      <Collapsible open={open} onOpenChange={setOpen}>
        <CollapsibleTrigger className="list-row -mx-2 h-10 w-[calc(100%+1rem)] gap-3 px-2">
          <span className="shrink-0 text-row font-medium">
            {m.nodes_listener_advanced()}
          </span>
          <span className="min-w-0 flex-1 truncate text-left text-meta text-muted-foreground">
            {advancedSet(fields)
              ? [
                  fields.listen.trim() &&
                    m.nodes_listener_fact_listen({
                      address: fields.listen.trim(),
                    }),
                  fields.publicPort.trim() &&
                    m.nodes_listener_fact_public_port({
                      port: fields.publicPort.trim(),
                    }),
                ]
                  .filter(Boolean)
                  .join(" · ")
              : m.nodes_listener_defaults()}
          </span>
          <ChevronDownIcon
            className="size-4 shrink-0 text-muted-foreground transition-transform duration-200 data-[open=true]:rotate-180"
            data-open={open}
          />
        </CollapsibleTrigger>
        <CollapsibleContent>
          <div className="grid grid-cols-[minmax(0,1fr)_6rem] gap-x-2 pt-3">
            <TextField
              id={`${idPrefix}-listen`}
              name="listen"
              label={m.nodes_listener_listen()}
              autoComplete="off"
              autoCapitalize="none"
              spellCheck={false}
              maxLength={45}
              className="font-mono"
              placeholder="0.0.0.0"
              value={fields.listen}
              invalid={invalid.has("listen")}
              onChange={(event) => set("listen", event.target.value)}
            />
            <TextField
              id={`${idPrefix}-public-port`}
              name="public-port"
              label={m.nodes_listener_public_port()}
              inputMode="numeric"
              autoComplete="off"
              maxLength={5}
              className="font-mono tabular-nums"
              placeholder={fields.port || suggestedPort(fields.preset)}
              value={fields.publicPort}
              invalid={invalid.has("publicPort")}
              disabled={host === ""}
              onChange={(event) => set("publicPort", event.target.value.trim())}
            />
          </div>
          {children}
        </CollapsibleContent>
      </Collapsible>
      {generated ? (
        <p className="text-meta text-muted-foreground">
          {m.nodes_listener_generated()}
        </p>
      ) : null}
    </div>
  )
}
