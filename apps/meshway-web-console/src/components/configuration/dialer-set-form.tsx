import type { DialerSetUpdate, DialerStream } from "@meshway/control-client"
import { cn } from "@meshway/ui/cn"

import { FieldOption, FieldSelect } from "@/components/chrome/field-select"
import { FieldInput } from "@/components/chrome/form"
import { Switch } from "@/components/ui/switch"
import {
  DEFAULT_CONNECT_TIMEOUT,
  DIALER_PROTOCOLS,
  dialerProblems,
  dialerRows,
  withEntryField,
  withProtocol,
  type DialerProtocol,
} from "@/lib/configuration/dialer-model"
import { m } from "@/paraglide/messages.js"

/** A transport protocol's name, wherever one is chosen (dialers, an edge's transports). */
export const protocolLabel: Record<DialerProtocol, () => string> = {
  native_udp: () => m.configuration_dialer_native_udp(),
  native_tcp: () => m.configuration_dialer_native_tcp(),
  vless: () => m.configuration_dialer_vless(),
  trojan: () => m.configuration_dialer_trojan(),
}

const streamLabel: Record<DialerStream, () => string> = {
  auto: () => m.configuration_dialer_stream_auto(),
  tcp: () => m.configuration_dialer_stream_tcp(),
  websocket: () => m.configuration_dialer_stream_websocket(),
}

const STREAMS: readonly DialerStream[] = ["auto", "tcp", "websocket"]

/**
 * A dialer set: the protocols in it, displayed by key, each with its
 * connect timeout; VLESS also its stream. Protocols the controller's
 * catalog offers can be added; one in the set the catalog no longer offers
 * stays listed. Read-only, the rows say the same without controls.
 */
export function DialerSetForm({
  id,
  value,
  offered,
  editable,
  onChange,
}: {
  /** Prefixes the fields' IDs. */
  id: string
  value: DialerSetUpdate
  /** The protocols the transport catalog offers; null while unread. */
  offered: readonly DialerProtocol[] | null
  editable: boolean
  onChange: (value: DialerSetUpdate) => void
}) {
  const rows = dialerRows(value)
  const refused = new Set(dialerProblems(value))
  const present = new Set(rows.map((row) => row.protocol))
  const absent = DIALER_PROTOCOLS.filter(
    (protocol) => !present.has(protocol) && (offered ?? []).includes(protocol)
  )

  return (
    <div className="flex flex-col gap-1.5">
      {rows.map((row) => {
        const field = `${id}-${row.protocol}`
        return (
          <div
            key={row.protocol}
            role="group"
            aria-label={protocolLabel[row.protocol]()}
            data-invalid={refused.has(row.protocol) || undefined}
            className="flex min-h-12 flex-wrap items-center gap-x-3 gap-y-2 rounded-tile bg-fill-subtle py-2 pr-2 pl-3.5 data-invalid:ring-1 data-invalid:ring-destructive/50"
          >
            <span className="flex min-w-0 flex-[1_1_8rem] items-center gap-2 text-row font-medium">
              <span className="truncate">{protocolLabel[row.protocol]()}</span>
              {row.enabled ? null : (
                <span className="shrink-0 rounded-full bg-fill-raised px-1.5 text-meta font-normal text-muted-foreground">
                  {m.configuration_dialer_off()}
                </span>
              )}
            </span>
            <label className="flex items-center gap-1.5 text-meta text-muted-foreground">
              {m.configuration_dialer_timeout()}
              {editable ? (
                <FieldInput
                  id={`${field}-timeout`}
                  name={`${field}-timeout`}
                  autoComplete="off"
                  spellCheck={false}
                  maxLength={24}
                  aria-invalid={refused.has(row.protocol) || undefined}
                  className="h-8 w-20 font-mono"
                  placeholder={DEFAULT_CONNECT_TIMEOUT}
                  value={row.connectTimeout ?? ""}
                  onChange={(event) =>
                    onChange(
                      withEntryField(
                        value,
                        row.protocol,
                        "connectTimeout",
                        event.target.value.trim() === ""
                          ? undefined
                          : event.target.value.trim()
                      )
                    )
                  }
                />
              ) : (
                <span className="font-mono text-row text-foreground">
                  {row.connectTimeout ?? DEFAULT_CONNECT_TIMEOUT}
                </span>
              )}
            </label>
            {row.protocol === "vless" ? (
              <label className="flex items-center gap-1.5 text-meta text-muted-foreground">
                {m.configuration_dialer_stream()}
                {editable ? (
                  <span className="w-28">
                    <FieldSelect
                      id={`${field}-stream`}
                      name={`${field}-stream`}
                      className="h-8 text-meta md:text-meta"
                      value={row.stream ?? "auto"}
                      onChange={(event) => {
                        const stream = STREAMS.find(
                          (known) => known === event.target.value
                        )
                        onChange(
                          withEntryField(
                            value,
                            "vless",
                            "stream",
                            stream === "auto" ? undefined : stream
                          )
                        )
                      }}
                    >
                      {STREAMS.map((stream) => (
                        <FieldOption key={stream} value={stream}>
                          {streamLabel[stream]()}
                        </FieldOption>
                      ))}
                    </FieldSelect>
                  </span>
                ) : (
                  <span className="text-row text-foreground">
                    {streamLabel[row.stream ?? "auto"]()}
                  </span>
                )}
              </label>
            ) : null}
            {editable ? (
              <span className="flex items-center gap-1">
                {row.enabled ? null : (
                  <button
                    type="button"
                    className="h-8 rounded-full px-3 text-meta font-medium hover:bg-fill-hover"
                    onClick={() =>
                      onChange(
                        withEntryField(
                          value,
                          row.protocol,
                          "enabled",
                          undefined
                        )
                      )
                    }
                  >
                    {m.configuration_dialer_turn_on()}
                  </button>
                )}
                <Switch
                  aria-label={m.configuration_dialer_use({
                    protocol: protocolLabel[row.protocol](),
                  })}
                  checked
                  onCheckedChange={() =>
                    onChange(withProtocol(value, row.protocol, false))
                  }
                />
              </span>
            ) : null}
          </div>
        )
      })}
      {editable
        ? absent.map((protocol) => (
            <div
              key={protocol}
              className={cn(
                "flex h-12 items-center justify-between gap-3 rounded-tile border border-dashed border-divider pr-2 pl-3.5 text-row text-muted-foreground"
              )}
            >
              {protocolLabel[protocol]()}
              <Switch
                className="data-unchecked:bg-foreground/20"
                aria-label={m.configuration_dialer_use({
                  protocol: protocolLabel[protocol](),
                })}
                checked={false}
                onCheckedChange={() =>
                  onChange(withProtocol(value, protocol, true))
                }
              />
            </div>
          ))
        : null}
      {rows.length === 0 ? (
        <p className="flex h-12 items-center rounded-tile bg-fill-subtle px-3.5 text-row text-muted-foreground">
          {m.configuration_dialer_none()}
        </p>
      ) : null}
    </div>
  )
}
