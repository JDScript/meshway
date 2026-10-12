import type { NodeInventoryView, StateView } from "@meshway/control-client"
import type { UseQueryResult } from "@tanstack/react-query"

import { failureText } from "@/api/failure"
import { CopyButton } from "@/components/chrome/copy-button"
import { useNow } from "@/hooks/use-now"
import { agoTime, dateTime } from "@/lib/format"
import { fullReport, stateOf } from "@/lib/nodes/state-model"
import { serverKeyLabel } from "@/lib/state-labels"
import { m } from "@/paraglide/messages.js"

import { Card, LoadingRow, Row, Rows } from "./-kit"

/** Who the device belongs to, what it runs, and when it joined. */
export function NodeDevice({
  node,
  state,
}: {
  node: NodeInventoryView
  state: UseQueryResult<StateView>
}) {
  const report = fullReport(stateOf(state.data, node.id))
  const now = useNow(1_000)
  const loading = state.data === undefined && !state.isError
  const unreported =
    state.data === undefined
      ? (failureText(state.error) ?? m.nodes_state_failed())
      : m.nodes_tile_unreported()
  return (
    <Card title={m.nodes_facts()}>
      <Rows>
        <Row
          label={
            node.identity.type === "tag"
              ? m.nodes_identity_tags()
              : m.nodes_identity_user()
          }
        >
          {node.identity.type === "tag"
            ? node.identity.tags.join(", ")
            : node.identity.user}
        </Row>
        {loading ? (
          <>
            <LoadingRow label={m.nodes_system_os()} />
            <LoadingRow label={m.nodes_system_hostname()} />
            <LoadingRow label={m.nodes_system_selection()} />
          </>
        ) : (
          <>
            <Row
              label={m.nodes_system_os()}
              tone={report ? undefined : "muted"}
              detail={
                report
                  ? m.nodes_device_agent({
                      version: report.system.agentVersion,
                    })
                  : undefined
              }
            >
              {report
                ? `${report.system.os} · ${report.system.arch}`
                : unreported}
            </Row>
            <Row
              label={m.nodes_system_hostname()}
              tone={report ? undefined : "muted"}
            >
              {report?.system.hostname ?? unreported}
            </Row>
            <Row
              label={m.nodes_system_selection()}
              tone={report ? undefined : "muted"}
            >
              {report
                ? (report.selection.appliedProfile ??
                  m.nodes_system_selection_none())
                : unreported}
            </Row>
          </>
        )}
        <Row
          label={m.nodes_server_key()}
          tone={
            node.serverKey === null
              ? "muted"
              : node.serverKey.status === "keyMismatch"
                ? "destructive"
                : undefined
          }
          detail={
            node.serverKey?.status === "keyMismatch"
              ? m.nodes_server_key_mismatch_detail()
              : undefined
          }
        >
          {node.serverKey
            ? m.nodes_server_key_generation({
                generation: node.serverKey.keyGeneration,
                status: serverKeyLabel[node.serverKey.status](),
              })
            : m.nodes_server_key_none()}
        </Row>
        <Row
          label={m.nodes_registered()}
          detail={agoTime(node.registeredAt, now)}
        >
          {dateTime(node.registeredAt)}
        </Row>
        <Row
          label={m.nodes_id()}
          mono
          action={<CopyButton label={m.nodes_id()} value={node.id} />}
        >
          {node.id}
        </Row>
      </Rows>
    </Card>
  )
}
