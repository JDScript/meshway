import { ChevronDownIcon, PlusIcon, XIcon } from "lucide-react"
import { useState } from "react"

import {
  FIELD_SIZE,
  FieldInput,
  SwitchRow,
  TextField,
} from "@/components/chrome/form"
import { Section } from "@/components/chrome/section"
import { SegmentedControl } from "@/components/chrome/segmented-control"
import { Button } from "@/components/ui/button"
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible"
import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"

import {
  advancedFacts,
  advancedProblems,
  MAX_ENDPOINTS,
  publicHostOf,
  transportOf,
  withPort,
  withPreset,
  withPublicHost,
  withPublicPort,
  type AdvancedFact,
  type ListenerFields,
  type ListenerProblem,
  type Preset,
} from "./listener-model"

/** A label for a control that is not a text field, looking as the form's own field labels do. */
export const fieldLabelClass = cn(
  "leading-snug font-medium",
  FIELD_SIZE.default.label
)

/**
 * The columns a listener's facts and fields share, so tiles above the form
 * and every field line up: two on narrow screens, four on wide ones (a
 * field takes two, a port one).
 */
export const LISTENER_GRID = "grid grid-cols-2 gap-x-2 xl:grid-cols-4"

/** An address and its port on one line at every width. */
const ADDRESS_GRID =
  "grid grid-cols-[minmax(0,1fr)_5.5rem] gap-x-2 xl:grid-cols-4 xl:[&>:first-child]:col-span-2"
/** The same, with a remove button after the port. */
const ENDPOINT_GRID =
  "grid grid-cols-[minmax(0,1fr)_5.5rem_auto] items-center gap-x-2 xl:grid-cols-4 xl:[&>:first-child]:col-span-2"

export const presetLabel: Record<Preset, () => string> = {
  native_udp: () => m.node_preset_native_udp(),
  native_tcp: () => m.node_preset_native_tcp(),
  vless_tcp: () => m.node_preset_vless_tcp(),
  vless_websocket: () => m.node_preset_vless_websocket(),
}

export const listenerProblemText: Record<
  ListenerProblem["kind"],
  () => string
> = {
  id: () => m.node_listener_id_invalid(),
  id_taken: () => m.node_listener_id_taken(),
  bind_address: () => m.node_listener_bind_invalid(),
  port: () => m.node_listener_port_invalid(),
  endpoints_max: () =>
    m.node_listener_endpoints_max({ max: String(MAX_ENDPOINTS) }),
  endpoint_host: () => m.node_listener_endpoint_invalid(),
  endpoint_duplicate: () => m.node_listener_endpoint_duplicate(),
  server_name: () => m.node_listener_server_name_invalid(),
  websocket_path: () => m.node_listener_ws_path_invalid(),
  websocket_host: () => m.node_listener_ws_host_invalid(),
}

const factText = (fact: AdvancedFact) => {
  switch (fact.kind) {
    case "bind_address":
      return m.node_listener_fact_bind({ address: fact.value })
    case "public_port":
      return m.node_listener_fact_public_port({ port: fact.value })
    case "endpoints":
      return m.node_listener_advertised_count({ count: String(fact.count) })
    case "server_name":
      return m.node_listener_fact_server_name({ name: fact.value })
    case "websocket_path":
      return m.node_listener_fact_ws_path({ path: fact.value })
    case "websocket_host":
      return m.node_listener_fact_ws_host({ host: fact.value })
  }
}

/**
 * The public address and the port: what decides whether a listener can be
 * dialled. Adding a node shows only these; a listener's editor starts with
 * them. The mapping onto endpoints and the bind port is the model's.
 */
export function ListenerAddress({
  idPrefix,
  fields,
  invalid,
  disabled,
  onChange,
}: {
  /** Prefix of the element IDs, unique on the page. */
  idPrefix: string
  fields: ListenerFields
  invalid: ReadonlySet<string>
  disabled: boolean
  onChange: (fields: ListenerFields) => void
}) {
  return (
    <div className={ADDRESS_GRID}>
      <TextField
        id={`${idPrefix}-public-host`}
        placeholder={m.node_listener_public_host_placeholder()}
        name="public-host"
        label={m.node_listener_public_host()}
        autoComplete="off"
        autoCapitalize="none"
        spellCheck={false}
        maxLength={253}
        className="font-mono"
        value={publicHostOf(fields)}
        invalid={invalid.has("endpoint.0.host")}
        disabled={disabled}
        onChange={(event) =>
          onChange(withPublicHost(fields, event.target.value))
        }
      />
      <TextField
        id={`${idPrefix}-port`}
        placeholder={m.node_listener_port_placeholder()}
        name="port"
        label={m.node_listener_port()}
        inputMode="numeric"
        autoComplete="off"
        maxLength={5}
        className="font-mono tabular-nums"
        value={fields.bindPort}
        invalid={invalid.has("bind_port")}
        disabled={disabled}
        onChange={(event) =>
          onChange(withPort(fields, event.target.value.trim()))
        }
      />
    </div>
  )
}

/**
 * A managed listener's editor: the basics (transport, enabled, public
 * address, port), then everything else behind "advanced", which states
 * what differs from the platform defaults while closed and opens by itself
 * when something in it is wrong. The ID and transport are fixed once saved.
 */
export function ListenerForm({
  idPrefix,
  fields,
  presets,
  creating,
  problems,
  disabled,
  showEnabled,
  onChange,
}: {
  /** Prefix of the element IDs, unique on the page. */
  idPrefix: string
  fields: ListenerFields
  presets: readonly Preset[]
  /** The network's other listener IDs while creating; null for a saved listener. */
  creating: { taken: ReadonlySet<string> } | null
  problems: readonly ListenerProblem[]
  disabled: boolean
  showEnabled: boolean
  onChange: (fields: ListenerFields) => void
}) {
  const invalid = new Set<string>(problems.map((problem) => problem.field))
  // Opens by itself when something inside becomes wrong; it can still be closed.
  const hidden = advancedProblems(fields, problems)
    .map((problem) => problem.field)
    .join(" ")
  const [open, setOpen] = useState(hidden !== "")
  const [seen, setSeen] = useState(hidden)
  if (hidden !== seen) {
    setSeen(hidden)
    if (hidden !== "") setOpen(true)
  }
  const facts = advancedFacts(fields)

  return (
    <div className="flex flex-col gap-6">
      <div className={cn(LISTENER_GRID, "gap-y-4")}>
        <div className="col-span-2 flex min-w-0 flex-col gap-2">
          <span className={fieldLabelClass}>{m.node_listener_transport()}</span>
          {presets.length > 1 && !disabled ? (
            <SegmentedControl<Preset>
              label={m.node_listener_transport()}
              value={fields.preset}
              options={presets.map((preset) => ({
                value: preset,
                label: presetLabel[preset](),
              }))}
              onChange={(preset) =>
                onChange(withPreset(fields, preset, creating))
              }
              className="self-start"
            />
          ) : (
            <span className="flex h-9 items-center text-row font-medium">
              {presetLabel[fields.preset]()}
            </span>
          )}
        </div>
      </div>

      {showEnabled ? (
        <SwitchRow
          id={`${idPrefix}-enabled`}
          label={m.node_listener_enabled()}
          checked={fields.enabled}
          disabled={disabled}
          onCheckedChange={(next) => onChange({ ...fields, enabled: next })}
        />
      ) : null}

      <ListenerAddress
        idPrefix={idPrefix}
        fields={fields}
        invalid={invalid}
        disabled={disabled}
        onChange={onChange}
      />

      <Collapsible open={open} onOpenChange={setOpen}>
        <CollapsibleTrigger className="list-row -mx-2 h-10 w-[calc(100%+1rem)] gap-3 px-2">
          <span className="shrink-0 text-row font-medium">
            {m.node_listener_advanced()}
          </span>
          <span className="min-w-0 flex-1 truncate text-meta text-muted-foreground">
            {facts.length === 0
              ? m.node_listener_defaults()
              : facts.map(factText).join(" · ")}
          </span>
          <ChevronDownIcon
            className="size-4 shrink-0 text-muted-foreground transition-transform duration-200 data-[open=true]:rotate-180"
            data-open={open}
          />
        </CollapsibleTrigger>
        <CollapsibleContent>
          <Advanced
            idPrefix={idPrefix}
            fields={fields}
            invalid={invalid}
            disabled={disabled}
            onChange={onChange}
          />
        </CollapsibleContent>
      </Collapsible>
    </div>
  )
}

function Advanced({
  idPrefix,
  fields,
  invalid,
  disabled,
  onChange,
}: {
  idPrefix: string
  fields: ListenerFields
  invalid: ReadonlySet<string>
  disabled: boolean
  onChange: (fields: ListenerFields) => void
}) {
  const set = <K extends keyof ListenerFields>(
    key: K,
    value: ListenerFields[K]
  ) => onChange({ ...fields, [key]: value })
  const vless = transportOf(fields.preset) === "vless"
  const ws = fields.preset === "vless_websocket"
  const field = (name: string) => `${idPrefix}-${name}`
  const first = fields.endpoints[0]
  const others = fields.endpoints.slice(1)
  const setEndpoint = (index: number, key: "host" | "port", value: string) =>
    set(
      "endpoints",
      fields.endpoints.map((current, at) =>
        at === index ? { ...current, [key]: value } : current
      )
    )

  return (
    <div className="flex flex-col gap-6 pt-3">
      <div className={ADDRESS_GRID}>
        <TextField
          id={field("bind-address")}
          placeholder={m.node_listener_bind_address_placeholder()}
          name="bind-address"
          label={m.node_listener_bind_address()}
          autoComplete="off"
          autoCapitalize="none"
          spellCheck={false}
          className="font-mono"
          value={fields.bindAddress}
          invalid={invalid.has("bind_address")}
          disabled={disabled}
          onChange={(event) => set("bindAddress", event.target.value)}
        />
        <TextField
          id={field("public-port")}
          placeholder={fields.bindPort || m.node_listener_port_placeholder()}
          name="public-port"
          label={m.node_listener_public_port()}
          inputMode="numeric"
          autoComplete="off"
          maxLength={5}
          className="font-mono tabular-nums"
          value={first?.port ?? ""}
          invalid={invalid.has("endpoint.0.port")}
          disabled={disabled || first === undefined}
          onChange={(event) =>
            onChange(withPublicPort(fields, event.target.value.trim()))
          }
        />
      </div>

      <Section
        title={m.node_listener_more_endpoints()}
        count={others.length}
        action={
          disabled ? null : (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="h-8 rounded-full px-3"
              disabled={
                first === undefined || fields.endpoints.length >= MAX_ENDPOINTS
              }
              onClick={() =>
                set("endpoints", [...fields.endpoints, { host: "", port: "" }])
              }
            >
              <PlusIcon />
              {m.node_listener_endpoint_add()}
            </Button>
          )
        }
      >
        {others.map((endpoint, offset) => {
          const index = offset + 1
          const n = String(index + 1)
          return (
            <div key={index} className={ENDPOINT_GRID}>
              <FieldInput
                className="font-mono"
                name={`endpoint-host-${index}`}
                aria-label={m.node_listener_endpoint_host({ n })}
                autoComplete="off"
                autoCapitalize="none"
                spellCheck={false}
                maxLength={253}
                placeholder={m.node_listener_public_host_placeholder()}
                value={endpoint.host}
                aria-invalid={
                  invalid.has(`endpoint.${index}.host`) || undefined
                }
                disabled={disabled}
                onChange={(event) =>
                  setEndpoint(index, "host", event.target.value)
                }
              />
              <FieldInput
                name={`endpoint-port-${index}`}
                aria-label={m.node_listener_endpoint_port({ n })}
                inputMode="numeric"
                autoComplete="off"
                maxLength={5}
                placeholder={m.node_listener_port_placeholder()}
                className="font-mono tabular-nums"
                value={endpoint.port}
                aria-invalid={
                  invalid.has(`endpoint.${index}.port`) || undefined
                }
                disabled={disabled}
                onChange={(event) =>
                  setEndpoint(index, "port", event.target.value.trim())
                }
              />
              {disabled ? null : (
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  className="justify-self-start rounded-full"
                  aria-label={m.node_listener_endpoint_remove({ n })}
                  onClick={() =>
                    set(
                      "endpoints",
                      fields.endpoints.filter((_, at) => at !== index)
                    )
                  }
                >
                  <XIcon />
                </Button>
              )}
            </div>
          )
        })}
      </Section>

      {vless ? (
        <div className={cn(LISTENER_GRID, "gap-y-4 [&>*]:col-span-2")}>
          <TextField
            id={field("server-name")}
            placeholder={m.node_listener_server_name_placeholder()}
            name="server-name"
            label={m.node_listener_server_name()}
            autoComplete="off"
            autoCapitalize="none"
            spellCheck={false}
            maxLength={253}
            className="font-mono"
            value={fields.serverName}
            invalid={invalid.has("server_name")}
            disabled={disabled}
            onChange={(event) => set("serverName", event.target.value)}
          />
          {ws ? (
            <>
              <TextField
                id={field("ws-path")}
                placeholder={m.node_listener_ws_path_placeholder()}
                name="websocket-path"
                label={m.node_listener_ws_path()}
                autoComplete="off"
                autoCapitalize="none"
                spellCheck={false}
                maxLength={1024}
                className="font-mono"
                value={fields.websocketPath}
                invalid={invalid.has("websocket_path")}
                disabled={disabled}
                onChange={(event) => set("websocketPath", event.target.value)}
              />
              <TextField
                id={field("ws-host")}
                placeholder={m.node_listener_ws_host_placeholder()}
                name="websocket-host"
                label={m.node_listener_ws_host()}
                autoComplete="off"
                autoCapitalize="none"
                spellCheck={false}
                maxLength={253}
                className="font-mono"
                value={fields.websocketHost}
                invalid={invalid.has("websocket_host")}
                disabled={disabled}
                onChange={(event) => set("websocketHost", event.target.value)}
              />
            </>
          ) : null}
        </div>
      ) : null}

      <div className={LISTENER_GRID}>
        <div
          className="col-span-2 flex min-w-0 flex-col gap-2"
          data-invalid={invalid.has("id") || undefined}
        >
          <span className={fieldLabelClass}>{m.node_listener_id()}</span>
          <span
            className={cn(
              "truncate font-mono text-row",
              invalid.has("id") && "text-warning"
            )}
          >
            {fields.id}
          </span>
        </div>
      </div>
    </div>
  )
}
