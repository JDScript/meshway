import type {
  DnsConfiguration,
  DnsServer,
  DnsTransport,
} from "@meshway/control-client"
import { useId, useState } from "react"
import { toast } from "sonner"

import { FieldInput, SwitchRow, TextField } from "@/components/chrome/form"
import { Section } from "@/components/chrome/section"
import { SegmentedControl } from "@/components/chrome/segmented-control"
import { ChipSetField } from "@/components/configuration/chips"
import {
  domainChips,
  Notes,
  notesWithin,
  refusedAt,
  RemoveButton,
  resolverChips,
  RuleRow,
  type Note,
} from "@/components/configuration/form-kit"
import { Switch } from "@/components/ui/switch"
import { pointerOf } from "@/lib/configuration/jsonc"
import { renamed, withEntry, without } from "@/lib/configuration/keyed"
import { parsePort } from "@/lib/configuration/selectors"
import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"

import { AddKey, KeyedRows, KeyInput, type KeyRule } from "./keyed-form"

const DEFAULT_PORT = 53

const serverKey = (): KeyRule => ({
  prefix: "",
  placeholder: m.configuration_dns_server_key_placeholder(),
})
const routeKey = (): KeyRule => ({
  prefix: "",
  placeholder: m.configuration_dns_suffix_placeholder(),
})

/** The network's DNS: MagicDNS and its domain, the resolvers, defaults, split routes and search domains. */
export function DnsForm({
  dns,
  editable,
  notes,
  onChange,
}: {
  dns: DnsConfiguration
  editable: boolean
  notes: readonly Note[]
  onChange: (dns: DnsConfiguration) => void
}) {
  const id = useId()
  const servers = dns.servers ?? {}
  const routes = dns.routes ?? {}
  const keys = Object.keys(servers)
  const resolvers = resolverChips(keys)
  const domains = domainChips()
  const set = <K extends keyof DnsConfiguration>(
    key: K,
    value: DnsConfiguration[K]
  ) => onChange({ ...dns, [key]: value })

  const renameServer = (from: string, to: string) =>
    onChange({
      ...dns,
      servers: renamed(servers, from, to),
      // References follow the resolver they name.
      default: (dns.default ?? []).map((key) => (key === from ? to : key)),
      routes: Object.fromEntries(
        Object.entries(routes).map(([suffix, list]) => [
          suffix,
          list.map((key) => (key === from ? to : key)),
        ])
      ),
    })

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-col gap-2">
        <SwitchRow
          id={`${id}-magic`}
          label={m.configuration_dns_magic()}
          checked={dns.magicDNS ?? true}
          disabled={!editable}
          onCheckedChange={(magicDNS) => set("magicDNS", magicDNS)}
        />
        <TextField
          id={`${id}-domain`}
          name="domain"
          label={m.configuration_dns_domain()}
          placeholder={m.configuration_dns_domain_placeholder()}
          autoComplete="off"
          autoCapitalize="none"
          spellCheck={false}
          className="font-mono"
          readOnly={!editable}
          invalid={refusedAt(notes, "/dns/domain")}
          value={dns.domain}
          onChange={(event) => set("domain", event.target.value)}
        />
        <Notes notes={notesWithin(notes, "/dns/domain")} />
        <SwitchRow
          id={`${id}-override`}
          label={m.configuration_dns_override()}
          checked={dns.overrideLocal ?? false}
          disabled={!editable}
          onCheckedChange={(overrideLocal) =>
            set("overrideLocal", overrideLocal)
          }
        />
      </div>

      <Section title={m.configuration_dns_servers()} count={keys.length}>
        <div className="flex flex-col gap-2">
          <KeyedRows
            count={keys.length}
            none={m.configuration_dns_servers_none()}
            label={m.configuration_dns_servers()}
          >
            {Object.entries(servers).map(([key, server], index) => (
              <li key={index}>
                <ServerRow
                  map={servers}
                  name={key}
                  server={server}
                  editable={editable}
                  notes={notesWithin(notes, pointerOf(["dns", "servers", key]))}
                  onRename={(next) => renameServer(key, next)}
                  onChange={(next) =>
                    set("servers", withEntry(servers, key, next))
                  }
                  onRemove={() => set("servers", without(servers, key))}
                />
              </li>
            ))}
          </KeyedRows>
          {editable ? (
            <AddKey
              map={servers}
              rule={serverKey()}
              label={m.configuration_dns_server_new()}
              addLabel={m.configuration_key_add()}
              onAdd={(key) =>
                set(
                  "servers",
                  withEntry<DnsServer>(servers, key, {
                    type: "udp",
                    server: "",
                  })
                )
              }
            />
          ) : null}
        </div>
      </Section>

      <Section title={m.configuration_dns_default()}>
        <ChipSetField
          values={dns.default ?? []}
          kind={resolvers}
          label={m.configuration_dns_default()}
          addLabel={m.configuration_dns_default_add()}
          editable={editable}
          invalid={refusedAt(notes, "/dns/default")}
          onChange={(next) => set("default", next)}
        />
        <Notes notes={notesWithin(notes, "/dns/default")} />
      </Section>

      <Section
        title={m.configuration_dns_routes()}
        count={Object.keys(routes).length}
      >
        <div className="flex flex-col gap-2">
          <KeyedRows
            count={Object.keys(routes).length}
            none={m.configuration_dns_routes_none()}
            label={m.configuration_dns_routes()}
          >
            {Object.entries(routes).map(([suffix, list], index) => {
              const at = pointerOf(["dns", "routes", suffix])
              return (
                <li key={index}>
                  <RuleRow
                    label={suffix}
                    notes={notesWithin(notes, at)}
                    end={
                      editable ? (
                        <RemoveButton
                          label={m.configuration_key_remove({ name: suffix })}
                          onRemove={() =>
                            set("routes", without(routes, suffix))
                          }
                        />
                      ) : null
                    }
                  >
                    <KeyInput
                      map={routes}
                      value={suffix}
                      rule={routeKey()}
                      label={m.configuration_dns_route_suffix()}
                      editable={editable}
                      onRename={(next) =>
                        set("routes", renamed(routes, suffix, next))
                      }
                      className="w-44 shrink-0"
                    />
                    <ChipSetField
                      values={list}
                      kind={resolvers}
                      label={m.configuration_dns_route_resolvers()}
                      addLabel={m.configuration_dns_default_add()}
                      editable={editable}
                      invalid={refusedAt(notes, at)}
                      onChange={(next) =>
                        set("routes", withEntry(routes, suffix, next))
                      }
                    />
                  </RuleRow>
                </li>
              )
            })}
          </KeyedRows>
          {editable ? (
            <AddKey
              map={routes}
              rule={routeKey()}
              label={m.configuration_dns_route_new()}
              addLabel={m.configuration_key_add()}
              onAdd={(suffix) =>
                set("routes", withEntry<string[]>(routes, suffix, []))
              }
            />
          ) : null}
        </div>
      </Section>

      <Section title={m.configuration_dns_search()}>
        <ChipSetField
          values={dns.searchDomains ?? []}
          kind={domains}
          label={m.configuration_dns_search()}
          addLabel={m.configuration_dns_search_add()}
          editable={editable}
          invalid={refusedAt(notes, "/dns/searchDomains")}
          onChange={(next) => set("searchDomains", next)}
        />
        <Notes notes={notesWithin(notes, "/dns/searchDomains")} />
      </Section>
    </div>
  )
}

function ServerRow({
  map,
  name,
  server,
  editable,
  notes,
  onRename,
  onChange,
  onRemove,
}: {
  map: Readonly<Record<string, unknown>>
  name: string
  server: DnsServer
  editable: boolean
  notes: readonly Note[]
  onRename: (name: string) => void
  onChange: (server: DnsServer) => void
  onRemove: () => void
}) {
  const id = useId()
  return (
    <RuleRow
      label={name}
      notes={notes}
      end={
        editable ? (
          <RemoveButton
            label={m.configuration_key_remove({ name })}
            onRemove={onRemove}
          />
        ) : null
      }
    >
      <KeyInput
        map={map}
        value={name}
        rule={serverKey()}
        label={m.configuration_dns_server_key()}
        editable={editable}
        onRename={onRename}
        className="w-28 shrink-0"
      />
      {editable ? (
        <SegmentedControl<DnsTransport>
          size="sm"
          label={m.configuration_dns_server_type()}
          value={server.type}
          options={[
            { value: "udp", label: "UDP" },
            { value: "tcp", label: "TCP" },
          ]}
          onChange={(type) => onChange({ ...server, type })}
        />
      ) : (
        <span className="font-mono text-meta uppercase">{server.type}</span>
      )}
      <FieldInput
        aria-label={m.configuration_dns_server_address()}
        name="server"
        autoComplete="off"
        autoCapitalize="none"
        spellCheck={false}
        placeholder={m.configuration_dns_server_address_placeholder()}
        readOnly={!editable}
        aria-invalid={
          notes.some((note) => note.origin !== "stale") || undefined
        }
        value={server.server}
        onChange={(event) =>
          onChange({ ...server, server: event.target.value })
        }
        className="h-7 w-40 rounded-full px-2.5 font-mono md:text-meta"
      />
      <PortInput
        value={server.serverPort ?? DEFAULT_PORT}
        editable={editable}
        onChange={(serverPort) => onChange({ ...server, serverPort })}
      />
      <label
        htmlFor={`${id}-exit`}
        className={cn(
          "inline-flex h-7 items-center gap-2 rounded-full px-2.5 text-meta text-muted-foreground",
          editable && "hover:bg-fill-hover"
        )}
      >
        <Switch
          id={`${id}-exit`}
          size="sm"
          className="data-unchecked:bg-foreground/20"
          checked={server.useWithExit ?? false}
          disabled={!editable}
          onCheckedChange={(useWithExit) =>
            onChange({ ...server, useWithExit })
          }
        />
        {m.configuration_dns_server_exit()}
      </label>
    </RuleRow>
  )
}

/** A port, taken when the field is left or Enter is pressed; a refused one goes back. */
function PortInput({
  value,
  editable,
  onChange,
}: {
  value: number
  editable: boolean
  onChange: (port: number) => void
}) {
  const [typed, setTyped] = useState<string | null>(null)
  const commit = () => {
    if (typed === null) return
    setTyped(null)
    const port = parsePort(typed.trim())
    if (port === null)
      toast.error(m.configuration_port_invalid(), { id: "configuration-key" })
    else if (port !== value) onChange(port)
  }
  return (
    <FieldInput
      aria-label={m.configuration_dns_server_port()}
      name="port"
      inputMode="numeric"
      autoComplete="off"
      placeholder={String(DEFAULT_PORT)}
      readOnly={!editable}
      value={typed ?? String(value)}
      onChange={(event) => setTyped(event.target.value)}
      onBlur={commit}
      onKeyDown={(event) => {
        if (event.key === "Enter") {
          event.preventDefault()
          commit()
        }
      }}
      className="h-7 w-16 rounded-full px-2.5 font-mono tabular-nums md:text-meta"
    />
  )
}
