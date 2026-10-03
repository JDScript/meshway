import { cn } from "@/lib/utils"
import { PlusIcon, XIcon } from "lucide-react"

import {
  FIELD_SIZE,
  FieldInput,
  SwitchRow,
  TextField,
} from "@/components/chrome/form"
import { EmptyLine, Section } from "@/components/chrome/section"
import { SegmentedControl } from "@/components/chrome/segmented-control"
import { Button } from "@/components/ui/button"
import { m } from "@/paraglide/messages.js"

import {
  MAX_ENDPOINTS,
  MAX_ID,
  transportOf,
  withPreset,
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

/**
 * A managed listener's fields. Bind (what the node opens) and advertised
 * endpoints (what is announced to dialers) are separate; nothing is filled
 * in as a public address. The ID and transport are fixed once saved.
 */
export function ListenerForm({
  idPrefix,
  fields,
  presets,
  creating,
  invalid,
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
  invalid: ReadonlySet<string>
  disabled: boolean
  showEnabled: boolean
  onChange: (fields: ListenerFields) => void
}) {
  const set = <K extends keyof ListenerFields>(
    key: K,
    value: ListenerFields[K]
  ) => onChange({ ...fields, [key]: value })
  const vless = transportOf(fields.preset) === "vless"
  const ws = fields.preset === "vless_websocket"
  const field = (name: string) => `${idPrefix}-${name}`

  return (
    <div className="flex flex-col gap-6">
      <div className={cn(LISTENER_GRID, "gap-y-4")}>
        {creating === null ? (
          <div className="col-span-2 flex min-w-0 flex-col gap-2">
            <span className={fieldLabelClass}>{m.node_listener_id()}</span>
            <span className="flex h-10 items-center truncate rounded-tile bg-fill-subtle px-3.5 font-mono text-row">
              {fields.id}
            </span>
          </div>
        ) : (
          <div className="col-span-2">
            <TextField
              id={field("id")}
              placeholder={m.node_listener_id_placeholder()}
              name="listener-id"
              label={m.node_listener_id()}
              autoComplete="off"
              autoCapitalize="none"
              spellCheck={false}
              maxLength={MAX_ID}
              className="font-mono"
              value={fields.id}
              invalid={invalid.has("id")}
              disabled={disabled}
              onChange={(event) => set("id", event.target.value)}
            />
          </div>
        )}
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
          id={field("enabled")}
          label={m.node_listener_enabled()}
          checked={fields.enabled}
          disabled={disabled}
          onCheckedChange={(next) => set("enabled", next)}
        />
      ) : null}

      <Section title={m.node_listener_bind()}>
        <div className={LISTENER_GRID}>
          <div className="col-span-2">
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
          </div>
          <TextField
            id={field("bind-port")}
            placeholder={m.node_listener_port_placeholder()}
            name="bind-port"
            label={m.node_listener_port()}
            inputMode="numeric"
            autoComplete="off"
            maxLength={5}
            className="font-mono tabular-nums"
            value={fields.bindPort}
            invalid={invalid.has("bind_port")}
            disabled={disabled}
            onChange={(event) => set("bindPort", event.target.value.trim())}
          />
        </div>
      </Section>

      <Section title={m.node_listener_advertised()}>
        <div className="flex flex-col gap-2">
          {fields.endpoints.length === 0 ? (
            <EmptyLine size="sm" dashed>
              {m.node_listener_advertised_none()}
            </EmptyLine>
          ) : (
            fields.endpoints.map((endpoint, index) => (
              <div key={index} className={cn(LISTENER_GRID, "items-center")}>
                <FieldInput
                  className="col-span-2 font-mono"
                  name={`endpoint-host-${index}`}
                  aria-label={m.node_listener_endpoint_host({
                    n: String(index + 1),
                  })}
                  autoComplete="off"
                  autoCapitalize="none"
                  spellCheck={false}
                  maxLength={253}
                  placeholder={m.node_listener_endpoint_host_placeholder()}
                  value={endpoint.host}
                  aria-invalid={
                    invalid.has(`endpoint.${index}.host`) || undefined
                  }
                  disabled={disabled}
                  onChange={(event) =>
                    set(
                      "endpoints",
                      fields.endpoints.map((current, at) =>
                        at === index
                          ? { ...current, host: event.target.value }
                          : current
                      )
                    )
                  }
                />
                <FieldInput
                  name={`endpoint-port-${index}`}
                  aria-label={m.node_listener_endpoint_port({
                    n: String(index + 1),
                  })}
                  inputMode="numeric"
                  autoComplete="off"
                  maxLength={5}
                  placeholder={m.node_listener_port()}
                  className="font-mono tabular-nums"
                  value={endpoint.port}
                  aria-invalid={
                    invalid.has(`endpoint.${index}.port`) || undefined
                  }
                  disabled={disabled}
                  onChange={(event) =>
                    set(
                      "endpoints",
                      fields.endpoints.map((current, at) =>
                        at === index
                          ? { ...current, port: event.target.value.trim() }
                          : current
                      )
                    )
                  }
                />
                {disabled ? null : (
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    className="rounded-full"
                    aria-label={m.node_listener_endpoint_remove({
                      n: String(index + 1),
                    })}
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
            ))
          )}
          {disabled ? null : (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="h-8 self-start rounded-full px-3"
              disabled={fields.endpoints.length >= MAX_ENDPOINTS}
              onClick={() =>
                set("endpoints", [...fields.endpoints, { host: "", port: "" }])
              }
            >
              <PlusIcon />
              {m.node_listener_endpoint_add()}
            </Button>
          )}
        </div>
      </Section>

      {vless ? (
        <Section title={ws ? m.node_listener_tls_ws() : m.node_listener_tls()}>
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
        </Section>
      ) : null}
    </div>
  )
}
