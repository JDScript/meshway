import type {
  ConfigurationOperation,
  ConfigurationView,
  NetworkSummary,
  NodeIdentity,
} from "@meshway/control-client"
import { cn } from "@meshway/ui/cn"
import { useQuery } from "@tanstack/react-query"
import { CheckIcon, TagIcon } from "lucide-react"
import type { ReactNode } from "react"

import { failureText } from "@/api/failure"
import { transportCatalogQuery } from "@/api/queries/nodes"
import { EmptyState } from "@/components/chrome/empty-state"
import { SwitchRow, TextField } from "@/components/chrome/form"
import { EmptyLine, Section } from "@/components/chrome/section"
import { SegmentedControl } from "@/components/chrome/segmented-control"
import { Tile } from "@/components/chrome/tile"
import { DialerSetForm } from "@/components/configuration/dialer-set-form"
import {
  Notes,
  notesWithin,
  refusedAt,
  type Note,
} from "@/components/configuration/form-kit"
import { DraftNotice } from "@/components/configuration/notice"
import { Skeleton } from "@/components/ui/skeleton"
import {
  DIALER_PROTOCOLS,
  dialerMode,
  withDialerMode,
  type DialerMode,
  type DialerProtocol,
} from "@/lib/configuration/dialer-model"
import { pointerOf } from "@/lib/configuration/jsonc"
import {
  addressesOf,
  isDeleted,
  settingsBase,
  settingsOf,
  tagsOf,
  withAddresses,
  withSettings,
  withTags,
  type AddressesValue,
  type NodeSettingsValue,
} from "@/lib/configuration/node-operations"
import { dialerUpdate, valueOf } from "@/lib/configuration/operations"
import { m } from "@/paraglide/messages.js"

import { useNodeConfiguration } from "./-draft"

type Change = (
  edit: (
    ops: readonly ConfigurationOperation[],
    base: ConfigurationView
  ) => ConfigurationOperation[]
) => void

/** An address of the pool's family to show as the field's format. */
function addressExample(pool: string | undefined, family: "ipv4" | "ipv6") {
  const network = pool?.split("/")[0]
  if (family === "ipv4") {
    const parts = network?.split(".")
    return parts && parts.length === 4
      ? `${parts.slice(0, 3).join(".")}.10`
      : "100.64.0.10"
  }
  return network?.endsWith("::") ? `${network}10` : "fd42::10"
}

/**
 * The node's settings (nodes.md) on one page: hostname, remark, whether it
 * runs and may be an exit, its tags, its Mesh addresses, its listeners
 * (`listeners`) and the dialers it uses. Edits go to the configuration
 * draft as typed operations; the commit bar applies them with the rest of
 * the draft.
 */
export function NodeSettings({
  networkId,
  nodeId,
  network,
  onEditSource,
  listeners,
}: {
  networkId: string
  nodeId: string
  network: NetworkSummary | null
  onEditSource: () => void
  /** The listeners' editor, placed before the dialers. */
  listeners: ReactNode
}) {
  const { state, change, discard } = useNodeConfiguration(
    networkId,
    nodeId,
    network
  )
  const catalog = useQuery(transportCatalogQuery())
  const offeredFailure =
    catalog.isError && !catalog.data
      ? (failureText(catalog.error) ?? m.nodes_catalog_failed())
      : null
  const offered: DialerProtocol[] | null = catalog.data
    ? DIALER_PROTOCOLS.filter((protocol) =>
        catalog.data.items.some((item) => item.protocol === protocol)
      )
    : null

  switch (state.kind) {
    case "loading":
      return (
        <div
          className="flex max-w-workspace flex-col gap-8"
          aria-label={m.nodes_settings_basics()}
        >
          {[2, 1, 2].map((fields, index) => (
            <div key={index} className="flex flex-col gap-3">
              <Skeleton className="h-3 w-16 rounded-full" />
              <div className="grid gap-4 sm:grid-cols-2">
                {Array.from({ length: fields }, (_, field) => (
                  <Skeleton key={field} className="h-10 rounded-tile" />
                ))}
              </div>
            </div>
          ))}
        </div>
      )
    case "failed":
      return (
        <EmptyLine>
          {failureText(state.error) ?? m.nodes_listeners_failed()}
        </EmptyLine>
      )
    case "missing":
      return <EmptyLine>{m.nodes_listeners_missing()}</EmptyLine>
    case "unreadable":
      return <EmptyLine>{m.nodes_settings_unreadable()}</EmptyLine>
    case "member":
      return (
        <SettingsBody
          listeners={listeners}
          value={settingsBase(state.settings)}
          identity={state.settings.identity}
          tags={
            state.settings.identity.type === "tag"
              ? state.settings.identity.tags
              : null
          }
          tagNames={null}
          addresses={state.settings.addresses}
          pools={null}
          defaults={null}
          offered={offered}
          offeredFailure={offeredFailure}
          notes={[]}
          nodeId={nodeId}
          change={null}
        />
      )
    case "manager": {
      const { base, ops } = state
      const value = settingsOf(ops, base, nodeId)
      const identity = base.nodes[nodeId]?.identity
      if (!value || !identity) return null
      const deleted = isDeleted(ops, nodeId)
      return (
        <div className="flex flex-col gap-6">
          {network?.actions.editConfiguration && state.document ? (
            <DraftNotice
              text={m.configuration_notice_document()}
              primaryLabel={m.configuration_notice_open_source()}
              onPrimary={onEditSource}
              onDiscard={discard}
            />
          ) : null}
          {deleted ? (
            <EmptyLine>{m.nodes_settings_deleting()}</EmptyLine>
          ) : null}
          <SettingsBody
            listeners={listeners}
            value={value}
            identity={identity}
            tags={tagsOf(ops, base, nodeId)}
            tagNames={Object.keys(valueOf(ops, base, "tagsOwners")).sort()}
            addresses={addressesOf(ops, base, nodeId) ?? identityless}
            pools={valueOf(ops, base, "addressPools")}
            defaults={dialerUpdate(base.defaults.dialers)}
            offered={offered}
            offeredFailure={offeredFailure}
            notes={state.notes}
            nodeId={nodeId}
            change={state.editable && !deleted ? change : null}
          />
        </div>
      )
    }
  }
}

const identityless: AddressesValue = { ipv4: null, ipv6: null }

function SettingsBody({
  listeners,
  value,
  identity,
  tags,
  tagNames,
  addresses,
  pools,
  defaults,
  offered,
  offeredFailure,
  notes,
  nodeId,
  change,
}: {
  listeners: ReactNode
  value: NodeSettingsValue
  identity: NodeIdentity
  /** The tags in the draft; null for a User identity no edit gives tags. */
  tags: readonly string[] | null
  /** The configuration's tags; null when not readable (a member). */
  tagNames: readonly string[] | null
  addresses: AddressesValue
  pools: { ipv4: readonly string[]; ipv6: readonly string[] } | null
  /** The network's dialers; null when not readable (a member). */
  defaults: ReturnType<typeof dialerUpdate> | null
  offered: readonly DialerProtocol[] | null
  offeredFailure: string | null
  notes: readonly Note[]
  nodeId: string
  /** null: read only. */
  change: Change | null
}) {
  const editable = change !== null
  const at = (...path: string[]) => pointerOf(["nodes", nodeId, ...path])
  const setSettings = (next: NodeSettingsValue) =>
    change?.((ops, base) => withSettings(ops, base, nodeId, next))
  const setAddresses = (next: AddressesValue) =>
    change?.((ops, base) => withAddresses(ops, base, nodeId, next))
  const setTags = (next: readonly string[] | null) =>
    change?.((ops, base) => withTags(ops, base, nodeId, next))
  const mode = dialerMode(value.dialers)
  const field = (key: string) => `node-${nodeId.slice(0, 8)}-${key}`

  return (
    <div className="flex max-w-workspace flex-col gap-8">
      <>
        <Section title={m.nodes_settings_basics()}>
          <div className="flex flex-col gap-4">
            <div className="grid gap-4 sm:grid-cols-2">
              <TextField
                id={field("hostname")}
                name="hostname"
                label={m.nodes_hostname()}
                autoComplete="off"
                autoCapitalize="none"
                spellCheck={false}
                maxLength={63}
                className="font-mono"
                placeholder="backend-1"
                disabled={!editable}
                invalid={refusedAt(notes, at("hostname"))}
                value={value.hostname}
                onChange={(event) =>
                  setSettings({ ...value, hostname: event.target.value })
                }
              />
              <TextField
                id={field("remark")}
                name="remark"
                label={m.nodes_settings_remark()}
                autoComplete="off"
                maxLength={256}
                placeholder={m.nodes_settings_remark_placeholder()}
                disabled={!editable}
                value={value.remark}
                onChange={(event) =>
                  setSettings({ ...value, remark: event.target.value })
                }
              />
            </div>
            <Notes notes={notesWithin(notes, at("hostname"))} />
            <div className="grid gap-2 sm:grid-cols-2">
              <SwitchRow
                id={field("enabled")}
                label={m.nodes_settings_enabled()}
                checked={value.enabled}
                disabled={!editable}
                onCheckedChange={(enabled) =>
                  setSettings({ ...value, enabled })
                }
              />
              <SwitchRow
                id={field("exit")}
                label={m.nodes_settings_exit()}
                checked={value.exitEnabled}
                disabled={!editable}
                onCheckedChange={(exitEnabled) =>
                  setSettings({ ...value, exitEnabled })
                }
              />
              <SwitchRow
                id={field("exit-ipv6")}
                label={m.nodes_settings_exit_ipv6()}
                checked={value.exitIpv6}
                disabled={!editable}
                onCheckedChange={(exitIpv6) =>
                  setSettings({ ...value, exitIpv6 })
                }
              />
            </div>
          </div>
        </Section>

        <Section
          title={m.nodes_settings_identity()}
          count={tags !== null ? tags.length : undefined}
        >
          <div className="flex flex-col gap-2">
            {identity.type === "user" ? (
              <Tile label={m.nodes_identity_user()}>{identity.user}</Tile>
            ) : null}
            {tagNames === null ? (
              tags !== null ? (
                <TagPills names={tags} chosen={tags} onToggle={null} />
              ) : null
            ) : tagNames.length === 0 ? (
              <EmptyState
                compact
                icon={TagIcon}
                title={m.nodes_add_tags_none()}
              >
                {m.nodes_tags_none_body()}
              </EmptyState>
            ) : (
              <TagPills
                names={[...new Set([...tagNames, ...(tags ?? [])])].sort()}
                chosen={tags ?? []}
                onToggle={
                  editable
                    ? (tag) => {
                        const current = tags ?? []
                        const next = current.includes(tag)
                          ? current.filter((entry) => entry !== tag)
                          : [...current, tag]
                        setTags(
                          identity.type === "user" && next.length === 0
                            ? null
                            : next
                        )
                      }
                    : null
                }
              />
            )}
            {identity.type === "user" && tags !== null && tags.length > 0 ? (
              <p className="text-meta text-warning">
                {m.nodes_settings_identity_switch()}
              </p>
            ) : null}
            <Notes notes={notesWithin(notes, at("identity"))} />
          </div>
        </Section>

        <Section title={m.nodes_settings_addresses()}>
          <div className="flex flex-col gap-2">
            <div className="grid gap-4 sm:grid-cols-2">
              {(["ipv4", "ipv6"] as const).map((family) => (
                <TextField
                  key={family}
                  id={field(family)}
                  name={family}
                  label={family === "ipv4" ? "IPv4" : "IPv6"}
                  autoComplete="off"
                  autoCapitalize="none"
                  spellCheck={false}
                  maxLength={45}
                  className="font-mono"
                  placeholder={addressExample(pools?.[family][0], family)}
                  disabled={!editable}
                  invalid={refusedAt(notes, at("addresses", family))}
                  value={addresses[family] ?? ""}
                  onChange={(event) => {
                    const text = event.target.value.trim()
                    setAddresses({
                      ...addresses,
                      [family]: text === "" ? null : text,
                    })
                  }}
                />
              ))}
            </div>
            <p className="text-meta text-muted-foreground">
              {pools
                ? m.nodes_settings_pools({
                    pools:
                      [...pools.ipv4, ...pools.ipv6].join(", ") ||
                      m.nodes_settings_pools_none(),
                  })
                : m.nodes_settings_addresses_empty()}
            </p>
            <Notes notes={notesWithin(notes, at("addresses"))} />
          </div>
        </Section>
      </>
      {listeners}
      <Section title={m.nodes_settings_dialers()}>
        <div className="flex flex-col gap-3">
          {editable ? (
            <SegmentedControl<DialerMode>
              label={m.nodes_settings_dialers()}
              value={mode}
              options={[
                { value: "inherit", label: m.nodes_dialers_inherit() },
                { value: "own", label: m.nodes_dialers_own() },
                { value: "none", label: m.nodes_dialers_none() },
              ]}
              onChange={(next) =>
                setSettings({
                  ...value,
                  dialers: withDialerMode(value.dialers, next, defaults ?? {}),
                })
              }
              className="self-start"
            />
          ) : (
            <Tile label={m.nodes_settings_dialers_mode()}>
              {mode === "inherit"
                ? m.nodes_dialers_inherit()
                : mode === "own"
                  ? m.nodes_dialers_own()
                  : m.nodes_dialers_none()}
            </Tile>
          )}
          {mode === "inherit" ? (
            defaults ? (
              <>
                <p className="text-meta text-muted-foreground">
                  {m.nodes_dialers_inherited()}
                </p>
                <DialerSetForm
                  id={field("inherited")}
                  value={defaults}
                  offered={offered}
                  editable={false}
                  onChange={() => undefined}
                />
              </>
            ) : null
          ) : mode === "none" ? (
            <EmptyLine>{m.nodes_dialers_none_fact()}</EmptyLine>
          ) : (
            <DialerSetForm
              id={field("dialers")}
              value={value.dialers ?? {}}
              offered={offered}
              offeredFailure={offeredFailure}
              editable={editable}
              onChange={(dialers) => setSettings({ ...value, dialers })}
            />
          )}
          <Notes notes={notesWithin(notes, at("dialers"))} />
        </div>
      </Section>
    </div>
  )
}

function TagPills({
  names,
  chosen,
  onToggle,
}: {
  names: readonly string[]
  chosen: readonly string[]
  onToggle: ((tag: string) => void) | null
}) {
  const pills: ReactNode[] = names.map((tag) => {
    const on = chosen.includes(tag)
    return (
      <button
        key={tag}
        type="button"
        aria-pressed={on}
        disabled={onToggle === null}
        onClick={() => onToggle?.(tag)}
        className={cn(
          "inline-flex h-8 items-center gap-1.5 rounded-full border border-border px-3 font-mono text-meta transition-colors outline-none hover:bg-fill-hover focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-default disabled:hover:bg-transparent aria-pressed:border-transparent aria-pressed:bg-fill-selected aria-pressed:text-foreground aria-pressed:hover:bg-fill-selected [&:not([aria-pressed=true])]:text-muted-foreground"
        )}
      >
        {on ? <CheckIcon className="-ml-0.5 size-3.5" /> : null}
        {tag}
      </button>
    )
  })
  return (
    <fieldset className="flex flex-wrap gap-1.5">
      <legend className="sr-only">{m.nodes_add_tags()}</legend>
      {pills}
    </fieldset>
  )
}
