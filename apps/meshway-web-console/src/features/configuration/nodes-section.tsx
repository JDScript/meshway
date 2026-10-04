import type {
  ConfigurationOperation,
  ConfigurationView,
} from "@meshway/control-client"
import { useNavigate } from "@tanstack/react-router"
import { TagIcon, UserIcon } from "lucide-react"

import { DraftMark } from "@/components/chrome/draft-mark"
import { RefList, RefRow } from "@/components/chrome/ref-row"
import { EmptyLine } from "@/components/chrome/section"
import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"

import { dialerMode } from "./dialer-model"
import { notesWithin, type Note } from "./form-kit"
import { pointerOf } from "./jsonc"
import {
  addressesOf,
  editsNode,
  isDeleted,
  listenerKeys,
  settingsOf,
  tagsOf,
} from "./node-operations"

/**
 * The configuration's nodes, as the draft has them: each node's hostname,
 * identity, address, listeners and dialers, with what stops or marks it;
 * each opens its workspace, where its settings are edited.
 */
export function NodesSection({
  networkId,
  base,
  ops,
  notes,
}: {
  networkId: string
  base: ConfigurationView
  ops: readonly ConfigurationOperation[]
  notes: readonly Note[]
}) {
  const navigate = useNavigate()
  const ids = Object.keys(base.nodes).sort((a, b) => {
    const x = settingsOf(ops, base, a)?.hostname ?? a
    const y = settingsOf(ops, base, b)?.hostname ?? b
    return x.localeCompare(y) || a.localeCompare(b)
  })
  if (ids.length === 0)
    return <EmptyLine>{m.configuration_nodes_none()}</EmptyLine>

  return (
    <RefList>
      {ids.map((id) => {
        const settings = settingsOf(ops, base, id)
        if (!settings) return null
        const tags = tagsOf(ops, base, id)
        const identity = base.nodes[id]?.identity
        const addresses = addressesOf(ops, base, id)
        const deleted = isDeleted(ops, id)
        const listeners = listenerKeys(ops, base, id).length
        const mode = dialerMode(settings.dialers)
        const problems = notesWithin(notes, pointerOf(["nodes", id])).filter(
          (note) => note.origin !== "stale"
        ).length
        const Glyph = tags !== null ? TagIcon : UserIcon
        return (
          <RefRow
            key={id}
            lead={
              <Glyph
                aria-hidden
                className={cn(
                  "size-4",
                  (deleted || !settings.enabled) && "text-muted-foreground"
                )}
              />
            }
            title={
              <span className="inline-flex max-w-full items-center gap-2">
                <span className={cn("truncate", deleted && "line-through")}>
                  {settings.hostname}
                </span>
                {editsNode(ops, id) ? <DraftMark /> : null}
              </span>
            }
            subtitle={[
              tags !== null
                ? tags.join(", ")
                : identity?.type === "user"
                  ? identity.user
                  : "",
              m.configuration_nodes_listeners({ count: listeners }),
              mode === "inherit"
                ? m.configuration_nodes_dialers_inherit()
                : mode === "none"
                  ? m.configuration_nodes_dialers_none()
                  : m.configuration_nodes_dialers_own(),
            ]
              .filter((part) => part !== "")
              .join(" · ")}
            detail={addresses?.ipv4 ?? addresses?.ipv6 ?? "—"}
            state={
              deleted
                ? m.configuration_nodes_deleting()
                : problems > 0
                  ? m.configuration_nodes_problems({ count: problems })
                  : !settings.enabled
                    ? m.configuration_nodes_disabled()
                    : settings.exitEnabled
                      ? m.configuration_nodes_exit()
                      : m.configuration_nodes_enabled()
            }
            onOpen={() =>
              void navigate({
                to: "/networks/$network/nodes/$node",
                params: { network: networkId, node: id },
                search: { tab: "settings" },
              })
            }
          />
        )
      })}
    </RefList>
  )
}
