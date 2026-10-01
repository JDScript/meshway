"use client"

import type { ConnectionDiagnostic, ConnectionNode } from "@meshway/control-client"
import { ArrowRightIcon, InfoIcon, Trash2Icon } from "lucide-react"
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
import { OptionSelect } from "@/components/common/option-select"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  endpointLabel,
  listenerHasEndpoint,
  needsProfile,
  nodeName,
  sameItem,
  transportLabels,
  validateCandidate,
  type CandidateDraft,
  type CandidateSession,
  type ListenerDraft,
} from "@/lib/connections-model"
import { cn } from "@/lib/utils"

// Option values for an endpoint; JSON keeps IPv6 colons unambiguous.
const endpointValue = (host: string, port: string) => JSON.stringify([host.trim(), port.trim()])

export function CandidateSheet({
  session,
  open,
  dirty,
  onWorkingChange,
  onClose,
  onApply,
  onDelete,
  others,
  listeners,
  nodes,
  diagnostics,
  diagnosticsStale,
  disabled,
}: {
  session: CandidateSession | null
  open: boolean
  dirty: boolean
  onWorkingChange: (form: CandidateDraft | null) => void
  onClose: () => void
  onApply: (candidate: CandidateDraft) => void
  onDelete: () => void
  others: CandidateDraft[]
  listeners: ListenerDraft[]
  nodes: ConnectionNode[]
  diagnostics: ConnectionDiagnostic[]
  diagnosticsStale: boolean
  disabled: boolean
}) {
  return (
    <ItemSheet open={open} dirty={dirty} onClose={onClose} discardTitle="放弃对这个拨号候选的修改？">
      {(guard) =>
        session ? (
          <CandidateForm
            key={session.id}
            session={session}
            guard={guard}
            onWorkingChange={onWorkingChange}
            others={others}
            listeners={listeners}
            nodes={nodes}
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

function CandidateForm({
  session,
  guard,
  onWorkingChange,
  others,
  listeners,
  nodes,
  diagnostics,
  diagnosticsStale,
  disabled,
  onApply,
  onDelete,
}: {
  session: CandidateSession
  guard: SheetGuard
  onWorkingChange: (form: CandidateDraft | null) => void
  others: CandidateDraft[]
  listeners: ListenerDraft[]
  nodes: ConnectionNode[]
  diagnostics: ConnectionDiagnostic[]
  diagnosticsStale: boolean
  disabled: boolean
  onApply: (candidate: CandidateDraft) => void
  onDelete: () => void
}) {
  const [form, setFormState] = useState<CandidateDraft>(session.working ?? session.initial)
  const [attempted, setAttempted] = useState(false)
  const baseId = useId()

  const setForm = (next: CandidateDraft) => {
    setFormState(next)
    onWorkingChange(sameItem(next, session.initial) ? null : next)
  }

  const issues = useMemo(
    () => validateCandidate(form, others, listeners, nodes),
    [form, others, listeners, nodes]
  )
  const visible = attempted || session.mode === "edit"
  const messages = (field: string) =>
    visible ? issues.filter((issue) => issue.field === field).map((issue) => issue.message) : []
  const has = (field: string) => messages(field).length > 0

  const listener = listeners.find((existing) => existing.id.trim() === form.listenerId)
  const transport = listener?.transport ?? "native_udp"
  const profile = listener ? needsProfile(listener.transport) : false

  const listenerOptions = [
    ...(form.listenerId !== "" && !listener
      ? [{ value: form.listenerId, label: `${form.listenerId}（不在草稿中）` }]
      : []),
    ...listeners.map((existing) => ({
      value: existing.id.trim(),
      label: existing.id.trim() || "（空 ID）",
      hint: `${nodeName(nodes, existing.nodeId)} · ${transportLabels[existing.transport]}`,
    })),
  ]

  const chosenEndpoint = form.host.trim() === "" ? null : endpointValue(form.host, form.port)
  const endpointOptions = [
    ...(chosenEndpoint !== null && (!listener || !listenerHasEndpoint(listener, form.host, form.port))
      ? [{ value: chosenEndpoint, label: `${endpointLabel(form.host, form.port)}（不在广播列表中）` }]
      : []),
    ...(listener?.endpoints ?? []).map((endpoint) => ({
      value: endpointValue(endpoint.host, endpoint.port),
      label: endpointLabel(endpoint.host, endpoint.port),
    })),
  ]

  const chooseListener = (listenerId: string) => {
    const next = listeners.find((existing) => existing.id.trim() === listenerId)
    const keepEndpoint = next ? listenerHasEndpoint(next, form.host, form.port) : false
    const [first] = next?.endpoints ?? []
    setForm({
      ...form,
      listenerId,
      host: keepEndpoint ? form.host : next?.endpoints.length === 1 && first ? first.host : "",
      port: keepEndpoint ? form.port : next?.endpoints.length === 1 && first ? first.port : "",
    })
  }

  const apply = () => {
    setAttempted(true)
    if (issues.length > 0) return
    onApply({
      ...form,
      id: form.id.trim(),
      priority: form.priority.trim(),
      credentialRef: form.credentialRef.trim(),
    })
  }

  return (
    <form
      className="flex min-h-0 flex-1 flex-col"
      onSubmit={(event) => {
        event.preventDefault()
        apply()
      }}
    >
      <SheetHeading
        title={session.mode === "new" ? "新建拨号候选" : "编辑拨号候选"}
        description="拨号候选声明「谁主动连接哪个监听器」，只授权这两个节点相邻，不授权 IP 流量、中转或出口。"
      />

      <div className="flex-1 overflow-y-auto">
        <div className="border-b bg-muted/30 px-4 py-3">
          <DirectionPreview
            dialer={form.dialerNodeId === "" ? "发起节点" : nodeName(nodes, form.dialerNodeId)}
            acceptor={listener ? nodeName(nodes, listener.nodeId) : "监听节点"}
            listenerId={form.listenerId}
            endpoint={form.host.trim() === "" ? null : endpointLabel(form.host, form.port)}
            transport={listener ? transportLabels[listener.transport] : null}
          />
        </div>

        <div className="flex flex-col divide-y">
          <Section title="候选 ID">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor={`${baseId}-id`} className="sr-only">
                候选 ID
              </Label>
              <Input
                id={`${baseId}-id`}
                value={form.id}
                onChange={(event) => setForm({ ...form, id: event.target.value })}
                autoComplete="off"
                spellCheck={false}
                aria-invalid={has("candidates.id") || undefined}
                className="max-w-xs font-mono"
              />
              <FieldError messages={messages("candidates.id")} />
            </div>
          </Section>

          <Section title="被连接方" description="接受连接的节点由监听器的所属节点决定。">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor={`${baseId}-listener`}>目标监听器</Label>
              <OptionSelect
                id={`${baseId}-listener`}
                value={form.listenerId === "" ? null : form.listenerId}
                options={listenerOptions}
                onValueChange={chooseListener}
                placeholder={listeners.length === 0 ? "草稿中还没有监听器" : "选择监听器"}
                invalid={has("candidates.listener_id")}
                className={cn("w-full max-w-sm", form.listenerId !== "" && !listener && "text-destructive")}
              />
              <FieldError messages={messages("candidates.listener_id")} />
            </div>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor={`${baseId}-endpoint`}>拨号地址</Label>
              <OptionSelect
                id={`${baseId}-endpoint`}
                value={chosenEndpoint}
                options={endpointOptions}
                onValueChange={(value) => {
                  const parsed: unknown = JSON.parse(value)
                  if (Array.isArray(parsed) && typeof parsed[0] === "string" && typeof parsed[1] === "string") {
                    setForm({ ...form, host: parsed[0], port: parsed[1] })
                  }
                }}
                placeholder={listener ? "选择监听器的对外地址" : "先选择监听器"}
                disabled={!listener && chosenEndpoint === null}
                invalid={has("candidates.endpoint")}
                className="w-full max-w-sm font-mono"
              />
              <FieldError messages={messages("candidates.endpoint")} />
            </div>
          </Section>

          <Section
            title="谁发起连接"
            description="只能选择已发布公钥、并声明可发起此载体连接的节点。"
          >
            <div className="flex flex-col gap-1.5">
              <Label htmlFor={`${baseId}-dialer`}>发起节点</Label>
              <ConnectionNodePicker
                id={`${baseId}-dialer`}
                nodes={nodes}
                value={form.dialerNodeId}
                onValueChange={(dialerNodeId) => setForm({ ...form, dialerNodeId })}
                transport={transport}
                role="dial"
                exclude={listener ? [listener.nodeId] : []}
                placeholder={listener ? "选择发起节点" : "先选择监听器"}
                disabled={!listener && form.dialerNodeId === ""}
                invalid={has("candidates.dialer_node_id")}
                className="w-full max-w-sm"
              />
              <FieldError messages={messages("candidates.dialer_node_id")} />
            </div>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor={`${baseId}-priority`}>优先级</Label>
              <Input
                id={`${baseId}-priority`}
                value={form.priority}
                onChange={(event) => setForm({ ...form, priority: event.target.value })}
                inputMode="numeric"
                autoComplete="off"
                aria-invalid={has("candidates.priority") || undefined}
                className="w-28 font-mono tabular-nums"
              />
              <FieldError messages={messages("candidates.priority")} />
              <span className="text-xs text-muted-foreground">
                0–65535，数值越小越优先；同一对节点的多个候选按优先级主备，不做负载分担。
              </span>
            </div>
          </Section>

          {profile || form.credentialRef !== "" ? (
            <Section
              title="本地配置档引用"
              description="填写发起节点上已有 VLESS 配置档的名称。实际凭据只保存在节点本地，从不经过控制台。"
            >
              <div className="flex flex-col gap-1.5">
                <Label htmlFor={`${baseId}-profile`}>发起方配置档引用</Label>
                <Input
                  id={`${baseId}-profile`}
                  value={form.credentialRef}
                  onChange={(event) => setForm({ ...form, credentialRef: event.target.value })}
                  placeholder={profile ? "例如 vless-sha-out" : ""}
                  autoComplete="off"
                  spellCheck={false}
                  aria-invalid={has("candidates.credential_ref") || undefined}
                  className="max-w-xs font-mono"
                />
                <FieldError messages={messages("candidates.credential_ref")} />
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

          {visible && messages("candidates").length > 0 ? (
            <Section title="需要修复">
              <FieldError messages={messages("candidates")} />
            </Section>
          ) : null}

          {diagnostics.length > 0 ? (
            <Section title={diagnosticsStale ? "已保存配置的诊断（草稿已修改）" : "Controller 诊断"}>
              <ConnectionDiagnosticList
                label="拨号候选的 controller 诊断"
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
            删除候选
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

export function DirectionPreview({
  dialer,
  acceptor,
  listenerId,
  endpoint,
  transport,
}: {
  dialer: string
  acceptor: string
  listenerId: string
  endpoint: string | null
  transport: string | null
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex flex-wrap items-center gap-2 text-sm">
        <span className="font-medium">{dialer}</span>
        <span className="flex items-center gap-1 text-xs text-muted-foreground">
          <span className="hidden sm:inline">发起</span>
          <ArrowRightIcon className="size-4" />
        </span>
        <span className="font-medium">{acceptor}</span>
        {listenerId !== "" ? (
          <span className="font-mono text-xs text-muted-foreground">
            {listenerId}
            {endpoint ? ` @ ${endpoint}` : ""}
          </span>
        ) : null}
        {transport ? <span className="text-xs text-muted-foreground">· {transport}</span> : null}
      </div>
      <p className="flex items-start gap-1.5 text-[0.6875rem] text-muted-foreground">
        <InfoIcon className="mt-0.5 size-3 shrink-0" />
        箭头只表示连接发起方向。建立后的认证通道可供两个方向复用，业务路径与访问授权不由它推导。
      </p>
    </div>
  )
}
