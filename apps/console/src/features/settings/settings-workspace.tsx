import { saveNetwork, type NetworkSnapshot } from "@meshway/control-client"
import { useQuery, useQueryClient } from "@tanstack/react-query"
import { useMemo } from "react"

import { CloseButton } from "@/components/chrome/close-button"
import { RevisionLine } from "@/components/chrome/draft-mark"
import { CopyButton } from "@/components/chrome/copy-button"
import { SwitchRow, TextField } from "@/components/chrome/form"
import { Section } from "@/components/chrome/section"
import { SurfaceHeader } from "@/components/chrome/surface-header"
import { Tile } from "@/components/chrome/tile"
import { Skeleton } from "@/components/ui/skeleton"
import { networkQuery, networksQuery } from "@/features/network/queries"
import { useCanManage } from "@/features/network/role"
import { useDraftCommit } from "@/features/shell/draft-commit"
import { surfaceTitleId } from "@/features/shell/page-frame"
import { authed } from "@/lib/api"
import { m } from "@/paraglide/messages.js"

import {
  invalidNetworkFields,
  networkFieldProblem,
  networkRequest,
  savedNetwork,
  useNetworkDraft,
  type NetworkFields,
} from "./network-draft"
import { sectionLabel } from "./settings-panel"
import type { SettingsSection } from "./url"

/**
 * One settings section, edited in place. Both sections edit the network's
 * one record, so they share one draft (and one revision): an edit made in
 * one is still pending in the other.
 */
export function SettingsWorkspace({
  networkId,
  section,
  onClose,
}: {
  networkId: string
  section: SettingsSection
  onClose: () => void
}) {
  const queryClient = useQueryClient()
  const network = useQuery(networkQuery(networkId))
  const canManage = useCanManage(networkId)
  const saved = useMemo(
    () => (network.data ? savedNetwork(network.data) : undefined),
    [network.data]
  )
  const draft = useNetworkDraft(networkId, saved)
  const invalid = draft.value ? invalidNetworkFields(draft.value) : []
  const firstInvalid = invalid[0]

  const commit = useDraftCommit({
    page: "settings",
    draft,
    saved,
    problem: firstInvalid ? networkFieldProblem[firstInvalid]() : null,
    save: (value, revision) =>
      authed((base, token) =>
        saveNetwork(base, token, networkId, networkRequest(value, revision))
      ),
    onSaved: (snapshot) => {
      queryClient.setQueryData(networkQuery(networkId).queryKey, snapshot)
      void queryClient.invalidateQueries({ queryKey: networksQuery().queryKey })
    },
    refresh: () => void network.refetch(),
  })

  const set = <K extends keyof NetworkFields>(
    key: K,
    value: NetworkFields[K]
  ) => draft.edit((current) => ({ ...current, [key]: value }))

  return (
    <div className="swap-in flex min-h-0 flex-1 flex-col" key={section}>
      <SurfaceHeader
        size="lg"
        className="px-6 pt-6 pb-4"
        title={(className) => (
          <h2
            id={surfaceTitleId("settings", "workspace")}
            className={className}
          >
            {sectionLabel[section]()}
          </h2>
        )}
        action={<CloseButton onClick={onClose} />}
        meta={
          <RevisionLine
            revision={network.data?.revision ?? null}
            draftBase={draft.dirty ? (draft.baseRevision ?? null) : null}
            facts={
              network.data && !canManage
                ? [<span key="read-only">{m.read_only()}</span>]
                : []
            }
          />
        }
      />
      <form
        className="min-h-0 flex-1 overflow-y-auto px-6 pt-1 pb-(--commit-space)"
        onSubmit={(event) => {
          event.preventDefault()
          commit.submit()
        }}
      >
        {/* Enter in a field submits; the commit bar is the visible control. */}
        <button type="submit" hidden />
        <div className="flex max-w-workspace flex-col gap-8">
          {draft.value && network.data ? (
            section === "network" ? (
              <NetworkSection
                fields={draft.value}
                snapshot={network.data}
                invalid={invalid}
                disabled={!canManage}
                onChange={set}
              />
            ) : (
              <DnsSection
                fields={draft.value}
                invalid={invalid}
                disabled={!canManage}
                onChange={set}
              />
            )
          ) : (
            <Placeholder />
          )}
        </div>
      </form>
    </div>
  )
}

type SectionProps = {
  fields: NetworkFields
  invalid: readonly string[]
  disabled: boolean
  onChange: <K extends keyof NetworkFields>(
    key: K,
    value: NetworkFields[K]
  ) => void
}

function NetworkSection({
  fields,
  snapshot,
  invalid,
  disabled,
  onChange,
}: SectionProps & { snapshot: NetworkSnapshot }) {
  return (
    <>
      <TextField
        id="network-name"
        placeholder={m.settings_name_placeholder()}
        name="network-name"
        label={m.settings_name()}
        autoComplete="off"
        maxLength={128}
        value={fields.name}
        invalid={invalid.includes("name")}
        disabled={disabled}
        onChange={(event) => onChange("name", event.target.value)}
      />
      <Section title={m.settings_identity()}>
        <div className="grid gap-2 sm:grid-cols-2">
          <div className="sm:col-span-2">
            <Tile
              label={m.settings_network_id()}
              action={
                <CopyButton
                  value={snapshot.network_id}
                  label={m.settings_network_id()}
                />
              }
            >
              <span className="font-mono font-normal">
                {snapshot.network_id}
              </span>
            </Tile>
          </div>
          <Tile label={m.settings_ipv4_pool()}>
            <span className="font-mono font-normal">
              {snapshot.network.ipv4_prefix}
            </span>
          </Tile>
          <Tile label={m.settings_ipv6_pool()}>
            <span className="font-mono font-normal">
              {snapshot.network.ipv6_prefix}
            </span>
          </Tile>
        </div>
      </Section>
    </>
  )
}

function DnsSection({ fields, invalid, disabled, onChange }: SectionProps) {
  return (
    <>
      <SwitchRow
        id="dns-enabled"
        label={m.settings_dns_enabled()}
        checked={fields.dnsEnabled}
        disabled={disabled}
        onCheckedChange={(next) => onChange("dnsEnabled", next)}
      />
      <div className="grid gap-4 sm:grid-cols-[1fr_8rem]">
        <TextField
          id="dns-zone"
          placeholder={m.settings_dns_zone_placeholder()}
          name="dns-zone"
          label={m.settings_dns_zone()}
          autoComplete="off"
          autoCapitalize="none"
          spellCheck={false}
          maxLength={128}
          className="font-mono"
          value={fields.zone}
          invalid={invalid.includes("zone")}
          disabled={disabled}
          onChange={(event) => onChange("zone", event.target.value)}
        />
        <TextField
          id="dns-ttl"
          placeholder={m.settings_dns_ttl_placeholder()}
          name="dns-ttl"
          label={m.settings_dns_ttl()}
          inputMode="numeric"
          autoComplete="off"
          maxLength={3}
          className="font-mono tabular-nums"
          value={fields.ttl}
          invalid={invalid.includes("ttl")}
          disabled={disabled}
          onChange={(event) => onChange("ttl", event.target.value.trim())}
        />
      </div>
    </>
  )
}

function Placeholder() {
  return (
    <div className="flex flex-col gap-3">
      <Skeleton className="h-3 w-20 rounded-full" />
      <Skeleton className="h-10 w-full rounded-tile" />
    </div>
  )
}
