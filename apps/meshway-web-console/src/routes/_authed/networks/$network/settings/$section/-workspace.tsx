import type { NetworkSummary } from "@meshway/control-client"
import { LogOutIcon } from "lucide-react"
import { useState } from "react"

import { BusyLabel } from "@/components/chrome/busy-label"
import { CloseButton } from "@/components/chrome/close-button"
import { CopyButton } from "@/components/chrome/copy-button"
import { TextField } from "@/components/chrome/form"
import { DangerRow, Section } from "@/components/chrome/section"
import { SurfaceHeader } from "@/components/chrome/surface-header"
import { Tile } from "@/components/chrome/tile"
import { LeaveNetworkDialog } from "@/components/network/removal"
import { surfaceTitleId } from "@/components/surface/page-frame"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { calendarDate } from "@/lib/format"
import { trimWhiteSpace } from "@/lib/network-name"
import { roleLabel } from "@/lib/roles"
import { m } from "@/paraglide/messages.js"

import { sectionLabel } from "../-panel"
import type { SettingsSection } from "../-search"
import { ActivitySection } from "./-activity"
import { useRename } from "./-use-rename"

/** One settings section: the network (its name, the facts about it, leaving it) or its activity. */
export function SettingsWorkspace({
  section,
  network,
  onClose,
}: {
  section: SettingsSection
  /** null until read. */
  network: NetworkSummary | null
  onClose: () => void
}) {
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
          <div className="flex h-5 items-center text-row text-muted-foreground">
            {section === "network" && network && !network.actions.rename
              ? m.read_only()
              : null}
          </div>
        }
      />
      <div className="scroll-edge min-h-0 flex-1 overflow-y-auto px-6 pt-1 pb-6">
        <div className="flex max-w-workspace flex-col gap-8">
          {!network ? (
            <Placeholder />
          ) : section === "network" ? (
            // Keyed by network: switching networks starts from its own name.
            <NetworkSection key={network.id} network={network} />
          ) : (
            <ActivitySection key={network.id} network={network} />
          )}
        </div>
      </div>
    </div>
  )
}

function NetworkSection({ network }: { network: NetworkSummary }) {
  const [leaving, setLeaving] = useState(false)
  return (
    <>
      {network.actions.rename ? (
        <RenameForm network={network} />
      ) : (
        <Tile label={m.network_name()}>{network.name}</Tile>
      )}
      <Section title={m.settings_facts()}>
        <div className="grid gap-2 sm:grid-cols-2">
          <div className="sm:col-span-2">
            <Tile
              label={m.settings_network_id()}
              action={
                <CopyButton
                  value={network.id}
                  label={m.settings_network_id()}
                />
              }
            >
              <span className="font-mono font-normal">{network.id}</span>
            </Tile>
          </div>
          <Tile label={m.settings_created()}>
            {calendarDate(network.createdAt) ?? "—"}
          </Tile>
          <Tile label={m.settings_role()}>{roleLabel[network.role]()}</Tile>
        </div>
      </Section>
      <Section title={m.settings_membership()}>
        <DangerRow
          fact={m.members_leave_fact()}
          action={
            <Button
              variant="destructive"
              className="h-9 rounded-full px-4"
              onClick={() => setLeaving(true)}
            >
              <LogOutIcon />
              {m.members_leave()}
            </Button>
          }
        />
      </Section>
      <LeaveNetworkDialog
        networkId={network.id}
        networkName={network.name}
        open={leaving}
        onOpenChange={setLeaving}
      />
    </>
  )
}

/**
 * The name, saved by its own button. What is typed stays until it is saved;
 * until then the field follows the name the controller reports.
 */
function RenameForm({ network }: { network: NetworkSummary }) {
  const { pending, rename } = useRename(network)
  const [typed, setTyped] = useState<string | null>(null)
  const value = typed ?? network.name
  // An emptied field counts as a change: saving it says what is missing.
  const changed = trimWhiteSpace(value) !== network.name

  return (
    <form
      noValidate
      className="flex items-end gap-2"
      onSubmit={(event) => {
        event.preventDefault()
        if (!changed) return
        void rename(value).then((saved) => {
          if (saved) setTyped(null)
        })
      }}
    >
      <div className="min-w-0 flex-1">
        <TextField
          id="network-name"
          name="network-name"
          label={m.network_name()}
          placeholder={m.network_name_placeholder()}
          autoComplete="off"
          value={value}
          onChange={(event) => setTyped(event.target.value)}
        />
      </div>
      <Button
        type="submit"
        className="h-10 shrink-0 rounded-tile px-4"
        disabled={!changed || pending}
        aria-busy={pending}
      >
        <BusyLabel busy={pending}>{m.settings_save_name()}</BusyLabel>
      </Button>
    </form>
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
