"use client"

import type { ConnectionDiagnostic, ConnectionNode } from "@meshway/control-client"
import { InfoIcon, PlusIcon, Trash2Icon, XIcon } from "lucide-react"
import { useId, useMemo, useState } from "react"

import { ConnectionDiagnosticList } from "@/components/connections/connection-diagnostics"
import { ConnectionNodePicker } from "@/components/connections/connection-node-picker"
import {
  FieldError,
  ItemSheet,
  Section,
  SheetFooter,
  SheetHeading,
  type SheetGuard,
} from "@/components/connections/item-sheet"
import { Segmented } from "@/components/common/segmented"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  CONNECTION_LIMITS,
  needsProfile,
  newEndpointKey,
  sameItem,
  transportHints,
  transportLabels,
  transports,
  validateListener,
  type ConnectionIssue,
  type ListenerDraft,
  type ListenerSession,
} from "@/lib/connections-model"

const transportOptions = transports.map((value) => ({ value, label: transportLabels[value] }))

export function ListenerSheet({
  session,
  open,
  dirty,
  onWorkingChange,
  onClose,
  onApply,
  onDelete,
  others,
  nodes,
  referencedBy,
  diagnostics,
  diagnosticsStale,
  disabled,
}: {
  session: ListenerSession | null
  open: boolean
  /** The open session holds unapplied edits. */
  dirty: boolean
  /** Unapplied form contents, or null when the form matches its initial value. */
  onWorkingChange: (form: ListenerDraft | null) => void
  onClose: () => void
  onApply: (listener: ListenerDraft) => void
  onDelete: () => void
  /** Every other listener in the draft, for ID uniqueness. */
  others: ListenerDraft[]
  nodes: ConnectionNode[]
  /** Candidates in the draft that dial this listener. */
  referencedBy: number
  diagnostics: ConnectionDiagnostic[]
  diagnosticsStale: boolean
  disabled: boolean
}) {
  return (
    <ItemSheet open={open} dirty={dirty} onClose={onClose} discardTitle="放弃对这个监听器的修改？">
      {(guard) =>
        session ? (
          <ListenerForm
            key={session.id}
            session={session}
            guard={guard}
            onWorkingChange={onWorkingChange}
            others={others}
            nodes={nodes}
            referencedBy={referencedBy}
            diagnostics={diagnostics}
            diagnosticsStale={diagnosticsStale}
            disabled={disabled}
            onApply={onApply}
            onDelete={onDelete}
          />
        ) : null
      }
    </ItemSheet>
  )
}

function ListenerForm({
  session,
  guard,
  onWorkingChange,
  others,
  nodes,
  referencedBy,
  diagnostics,
  diagnosticsStale,
  disabled,
  onApply,
  onDelete,
}: {
  session: ListenerSession
  guard: SheetGuard
  onWorkingChange: (form: ListenerDraft | null) => void
  others: ListenerDraft[]
  nodes: ConnectionNode[]
  referencedBy: number
  diagnostics: ConnectionDiagnostic[]
  diagnosticsStale: boolean
  disabled: boolean
  onApply: (listener: ListenerDraft) => void
  onDelete: () => void
}) {
  const [form, setFormState] = useState<ListenerDraft>(session.working ?? session.initial)
  const [attempted, setAttempted] = useState(false)
  const baseId = useId()

  const setForm = (next: ListenerDraft) => {
    setFormState(next)
    onWorkingChange(sameItem(next, session.initial) ? null : next)
  }

  const issues = useMemo(() => validateListener(form, others, nodes), [form, others, nodes])
  const visible = attempted || session.mode === "edit"
  const messages = (field: string) =>
    visible
      ? issues
          .filter((issue) => issue.field === field || issue.field.startsWith(`${field}[`))
          .map((issue) => issue.message)
      : []
  const has = (field: string) => messages(field).length > 0
  const endpointIssue = (index: number, part: "host" | "port") =>
    visible &&
    issues.some(
      (issue: ConnectionIssue) =>
        issue.field === `listeners.advertised_endpoints[${index}].${part}` ||
        issue.field === `listeners.advertised_endpoints[${index}]`
    )

  const renamed =
    session.mode === "edit" && form.id.trim() !== session.initial.id.trim() && referencedBy > 0
  const profile = needsProfile(form.transport)

  const apply = () => {
    setAttempted(true)
    if (issues.length > 0) return
    onApply({
      ...form,
      id: form.id.trim(),
      bind: form.bind.trim(),
      credentialRef: form.credentialRef.trim(),
      endpoints: form.endpoints.map((endpoint) => ({
        ...endpoint,
        host: endpoint.host.trim(),
        port: endpoint.port.trim(),
      })),
    })
  }

  const setEndpoint = (index: number, patch: Partial<{ host: string; port: string }>) =>
    setForm({
      ...form,
      endpoints: form.endpoints.map((endpoint, i) => (i === index ? { ...endpoint, ...patch } : endpoint)),
    })

  return (
    <form
      className="flex min-h-0 flex-1 flex-col"
      onSubmit={(event) => {
        event.preventDefault()
        apply()
      }}
    >
      <SheetHeading
        title={session.mode === "new" ? "新建监听器" : "编辑监听器"}
        description="监听器声明「谁在哪里等待连接」。它本身不授权任何节点连接，也不授权流量。"
      />

      <div className="flex-1 overflow-y-auto">
        <div className="flex flex-col divide-y">
          <Section title="标识与载体">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor={`${baseId}-id`}>监听器 ID</Label>
              <Input
                id={`${baseId}-id`}
                value={form.id}
                onChange={(event) => setForm({ ...form, id: event.target.value })}
                autoComplete="off"
                spellCheck={false}
                aria-invalid={has("listeners.id") || undefined}
                className="max-w-xs font-mono"
              />
              <FieldError messages={messages("listeners.id")} />
              {renamed ? (
                <p className="flex items-start gap-1.5 text-xs text-muted-foreground">
                  <InfoIcon className="mt-0.5 size-3.5 shrink-0" />
                  应用后，草稿中引用此监听器的 {referencedBy} 个拨号候选会一并改用新 ID。
                </p>
              ) : null}
            </div>
            <div className="flex flex-col gap-1.5">
              <span id={`${baseId}-transport`} className="text-xs font-medium">
                载体
              </span>
              <Segmented
                value={form.transport}
                options={transportOptions}
                onValueChange={(transport) => setForm({ ...form, transport })}
                aria-labelledby={`${baseId}-transport`}
              />
              <span className="text-xs text-muted-foreground">{transportHints[form.transport]}</span>
            </div>
          </Section>

          <Section
            title="谁监听"
            description="只能选择已发布公钥、并声明可监听此载体的节点。"
          >
            <div className="flex flex-col gap-1.5">
              <Label htmlFor={`${baseId}-node`}>监听节点</Label>
              <ConnectionNodePicker
                id={`${baseId}-node`}
                nodes={nodes}
                value={form.nodeId}
                onValueChange={(nodeId) => setForm({ ...form, nodeId })}
                transport={form.transport}
                role="accept"
                placeholder="选择监听节点"
                invalid={has("listeners.node_id")}
                className="w-full max-w-sm"
              />
              <FieldError messages={messages("listeners.node_id")} />
            </div>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor={`${baseId}-bind`}>本机监听地址</Label>
              <Input
                id={`${baseId}-bind`}
                value={form.bind}
                onChange={(event) => setForm({ ...form, bind: event.target.value })}
                placeholder="0.0.0.0:51820 或 [::]:51820"
                autoComplete="off"
                spellCheck={false}
                aria-invalid={has("listeners.bind") || undefined}
                className="max-w-xs font-mono"
              />
              <FieldError messages={messages("listeners.bind")} />
              <span className="text-xs text-muted-foreground">
                节点进程绑定的 socket 地址，可以是 0.0.0.0 / [::]；它不会作为对外地址发给其他节点。
              </span>
            </div>
          </Section>

          <Section
            title="对外地址"
            description="其他节点拨号时使用的 IP 或域名与端口（例如 NAT 映射后的公网地址）。不能填 0.0.0.0 / ::。"
          >
            <ol className="flex flex-col gap-2">
              {form.endpoints.map((endpoint, index) => (
                <li key={endpoint.key} className="flex items-center gap-2">
                  <div className="min-w-0 flex-1">
                    <Label htmlFor={`${baseId}-host-${endpoint.key}`} className="sr-only">
                      对外地址 {index + 1} 主机
                    </Label>
                    <Input
                      id={`${baseId}-host-${endpoint.key}`}
                      value={endpoint.host}
                      onChange={(event) => setEndpoint(index, { host: event.target.value })}
                      placeholder="lax.example.net 或 203.0.113.7"
                      autoComplete="off"
                      spellCheck={false}
                      aria-invalid={endpointIssue(index, "host") || undefined}
                      className="font-mono"
                    />
                  </div>
                  <div className="w-24 shrink-0">
                    <Label htmlFor={`${baseId}-port-${endpoint.key}`} className="sr-only">
                      对外地址 {index + 1} 端口
                    </Label>
                    <Input
                      id={`${baseId}-port-${endpoint.key}`}
                      value={endpoint.port}
                      onChange={(event) => setEndpoint(index, { port: event.target.value })}
                      placeholder="端口"
                      inputMode="numeric"
                      autoComplete="off"
                      aria-invalid={endpointIssue(index, "port") || undefined}
                      className="font-mono tabular-nums"
                    />
                  </div>
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    aria-label={`移除对外地址 ${index + 1}`}
                    onClick={() =>
                      setForm({ ...form, endpoints: form.endpoints.filter((_, i) => i !== index) })
                    }
                  >
                    <XIcon />
                  </Button>
                </li>
              ))}
            </ol>
            <div className="flex flex-wrap items-center gap-3">
              <Button
                type="button"
                variant="outline"
                disabled={form.endpoints.length >= CONNECTION_LIMITS.endpointsPerListener}
                onClick={() =>
                  setForm({
                    ...form,
                    endpoints: [
                      ...form.endpoints,
                      {
                        key: newEndpointKey(form),
                        host: "",
                        port: bindPort(form.bind),
                      },
                    ],
                  })
                }
              >
                <PlusIcon data-icon="inline-start" />
                添加对外地址
              </Button>
              <span className="text-[0.6875rem] text-muted-foreground tabular-nums">
                {form.endpoints.length}/{CONNECTION_LIMITS.endpointsPerListener}
              </span>
            </div>
            <FieldError messages={messages("listeners.advertised_endpoints")} />
          </Section>

          {profile || form.credentialRef !== "" ? (
            <Section
              title="本地配置档引用"
              description="只填写监听节点上已有 VLESS 配置档的名称。UUID、密码、TLS 证书等凭据保存在节点本地，从不经过控制台。"
            >
              <div className="flex flex-col gap-1.5">
                <Label htmlFor={`${baseId}-profile`}>监听方配置档引用</Label>
                <Input
                  id={`${baseId}-profile`}
                  value={form.credentialRef}
                  onChange={(event) => setForm({ ...form, credentialRef: event.target.value })}
                  placeholder={profile ? "例如 vless-lax-in" : ""}
                  autoComplete="off"
                  spellCheck={false}
                  aria-invalid={has("listeners.credential_ref") || undefined}
                  className="max-w-xs font-mono"
                />
                <FieldError messages={messages("listeners.credential_ref")} />
                {!profile ? (
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    className="self-start"
                    onClick={() => setForm({ ...form, credentialRef: "" })}
                  >
                    清除引用
                  </Button>
                ) : null}
              </div>
            </Section>
          ) : null}

          {diagnostics.length > 0 ? (
            <Section title={diagnosticsStale ? "已保存配置的诊断（草稿已修改）" : "Controller 诊断"}>
              <ConnectionDiagnosticList
                label="监听器的 controller 诊断"
                diagnostics={diagnostics}
                stale={diagnosticsStale}
              />
            </Section>
          ) : null}
        </div>
      </div>

      <SheetFooter className="flex-row items-center gap-2 border-t p-3">
        {session.mode === "edit" ? (
          <Button
            type="button"
            variant="ghost"
            className="text-destructive"
            onClick={onDelete}
            disabled={disabled}
          >
            <Trash2Icon data-icon="inline-start" />
            删除监听器
          </Button>
        ) : null}
        <div className="ml-auto flex items-center gap-2">
          <Button type="button" variant="outline" onClick={guard.requestClose}>
            取消
          </Button>
          <Button type="submit" disabled={disabled}>
            {session.mode === "new" ? "添加到草稿" : "应用到草稿"}
          </Button>
        </div>
      </SheetFooter>
    </form>
  )
}

function bindPort(bind: string): string {
  return /:([0-9]{1,5})$/.exec(bind.trim())?.[1] ?? ""
}
