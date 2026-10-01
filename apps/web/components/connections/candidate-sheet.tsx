"use client"

import type { ConnectionDiagnostic, ConnectionNode } from "@meshway/control-client"
import { ArrowRightIcon, InfoIcon, Trash2Icon } from "lucide-react"
import { useId, useMemo, useState } from "react"

import { AdvancedSection } from "@/components/common/advanced-section"
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
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group"
import {
  autofillCandidate,
  dialEntries,
  dialRole,
  dialRoleLabel,
  endpointLabel,
  entryValue,
  fromCandidateForm,
  listeningNodeIds,
  needsProfile,
  nodeName,
  resolveConnectionNode,
  sameItem,
  transportLabels,
  validateCandidate,
  type CandidateDraft,
  type CandidateForm,
  type CandidateSession,
  type ConnectionsDraft,
} from "@/lib/connections-model"
import { cn } from "@/lib/utils"

export function CandidateSheet({
  session,
  open,
  dirty,
  onWorkingChange,
  onClose,
  onApply,
  onDelete,
  draft,
  nodes,
  diagnostics,
  diagnosticsStale,
  disabled,
}: {
  session: CandidateSession | null
  open: boolean
  dirty: boolean
  onWorkingChange: (form: CandidateForm | null) => void
  onClose: () => void
  onApply: (candidate: CandidateDraft) => void
  onDelete: () => void
  /** The whole current draft: entries, duplicates and primary / backup order. */
  draft: ConnectionsDraft
  nodes: ConnectionNode[]
  diagnostics: ConnectionDiagnostic[]
  diagnosticsStale: boolean
  disabled: boolean
}) {
  return (
    <ItemSheet open={open} dirty={dirty} onClose={onClose} discardTitle="放弃对这个连接的修改？">
      {(guard) =>
        session ? (
          <CandidateFormView
            key={session.id}
            session={session}
            guard={guard}
            onWorkingChange={onWorkingChange}
            draft={draft}
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

function CandidateFormView({
  session,
  guard,
  onWorkingChange,
  draft,
  nodes,
  diagnostics,
  diagnosticsStale,
  disabled,
  onApply,
  onDelete,
}: {
  session: CandidateSession
  guard: SheetGuard
  onWorkingChange: (form: CandidateForm | null) => void
  draft: ConnectionsDraft
  nodes: ConnectionNode[]
  diagnostics: ConnectionDiagnostic[]
  diagnosticsStale: boolean
  disabled: boolean
  onApply: (candidate: CandidateDraft) => void
  onDelete: () => void
}) {
  const [form, setFormState] = useState<CandidateForm>(session.working ?? session.initial)
  const [attempted, setAttempted] = useState(false)
  const baseId = useId()

  const setForm = (next: CandidateForm) => {
    const filled = autofillCandidate(next, form, session.mode, draft, nodes)
    setFormState(filled)
    onWorkingChange(sameItem(filled, session.initial) ? null : filled)
  }

  const others = useMemo(() => draft.candidates.filter((candidate) => candidate.key !== form.key), [draft, form.key])
  const candidate = fromCandidateForm(form)
  const issues = validateCandidate(candidate, others, draft.listeners, nodes)
  const visible = attempted || session.mode === "edit"
  const messages = (field: string) =>
    visible ? issues.filter((issue) => issue.field === field).map((issue) => issue.message) : []
  const has = (field: string) => messages(field).length > 0

  const listener = draft.listeners.find((existing) => existing.id.trim() === form.listenerId)
  const profile = listener ? needsProfile(listener.transport) : false
  const fixedDialer = session.fixed === "dialer" && form.dialerNodeId === session.initial.dialerNodeId
  const fixedTarget = session.fixed === "target" && form.targetNodeId === session.initial.targetNodeId

  const entries = dialEntries(draft, nodes, form.dialerNodeId, form.targetNodeId, form.key)
  const chosen = form.host.trim() === "" ? null : entryValue(form.listenerId, form.host, form.port)
  const chosenListed = entries.some((entry) => entry.value === chosen)

  const listening = new Set(listeningNodeIds(draft, ""))
  const target = form.targetNodeId === "" ? null : resolveConnectionNode(nodes, form.targetNodeId)
  const targetOptions = [
    ...(target && target.kind !== "active" ? [{ value: form.targetNodeId, label: nodeName(nodes, form.targetNodeId) }] : []),
    ...nodes
      .filter((entry) => entry.node.revoked_at === null)
      .map((entry) => {
        const count = draft.listeners.filter((existing) => existing.nodeId === entry.node.id).length
        const self = entry.node.id === form.dialerNodeId
        return {
          value: entry.node.id,
          label: entry.node.name,
          hint: self ? "发起节点本身" : count > 0 ? `${count} 个监听` : "没有监听",
          disabled: entry.node.id !== form.targetNodeId && (self || !listening.has(entry.node.id)),
        }
      }),
  ]

  const role = dialRoleLabel(
    dialRole(candidate, { ...draft, candidates: [...others, candidate] })
  )

  const apply = () => {
    setAttempted(true)
    if (issues.length > 0) return
    onApply({
      ...candidate,
      id: candidate.id.trim(),
      priority: candidate.priority.trim(),
      credentialRef: candidate.credentialRef.trim(),
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
        title={session.mode === "new" ? "主动连接节点" : "编辑主动连接"}
        description="一个节点主动拨号另一个节点的监听。建立的认证通道两个方向都能用，但不授权 IP 流量、中转或出口。"
      />

      <div className="flex-1 overflow-y-auto">
        <div className="border-b bg-muted/30 px-4 py-3">
          <DirectionPreview
            dialer={form.dialerNodeId === "" ? "发起节点" : nodeName(nodes, form.dialerNodeId)}
            acceptor={form.targetNodeId === "" ? "目标节点" : nodeName(nodes, form.targetNodeId)}
            endpoint={form.host.trim() === "" ? null : endpointLabel(form.host, form.port)}
            transport={listener ? transportLabels[listener.transport] : null}
            role={role}
          />
        </div>

        <div className="flex flex-col divide-y">
          <Section title="谁主动发起">
            {fixedDialer ? (
              <p className="text-sm font-medium">{nodeName(nodes, form.dialerNodeId)}</p>
            ) : (
              <div className="flex flex-col gap-1.5">
                <Label htmlFor={`${baseId}-dialer`}>发起节点</Label>
                <ConnectionNodePicker
                  id={`${baseId}-dialer`}
                  nodes={nodes}
                  value={form.dialerNodeId}
                  onValueChange={(dialerNodeId) => setForm({ ...form, dialerNodeId })}
                  transport={listener?.transport ?? null}
                  role="dial"
                  exclude={form.targetNodeId ? [form.targetNodeId] : []}
                  placeholder="选择发起节点"
                  invalid={has("candidates.dialer_node_id")}
                  className="w-full max-w-sm"
                />
              </div>
            )}
            <FieldError messages={messages("candidates.dialer_node_id")} />
            <span className="text-xs text-muted-foreground">发起方不需要公网地址或监听，适合在 NAT / 防火墙后的节点。</span>
          </Section>

          <Section title="连接到哪个节点">
            {fixedTarget ? (
              <p className="text-sm font-medium">{nodeName(nodes, form.targetNodeId)}</p>
            ) : (
              <div className="flex flex-col gap-1.5">
                <Label htmlFor={`${baseId}-target`}>目标节点</Label>
                <OptionSelect
                  id={`${baseId}-target`}
                  value={form.targetNodeId === "" ? null : form.targetNodeId}
                  options={targetOptions}
                  onValueChange={(targetNodeId) => setForm({ ...form, targetNodeId })}
                  placeholder={listening.size === 0 ? "还没有节点在接收连接" : "选择目标节点"}
                  className={cn("w-full max-w-sm", target && target.kind !== "active" && "text-destructive")}
                />
              </div>
            )}

            <div className="flex flex-col gap-1.5">
              <span id={`${baseId}-entry`} className="text-xs font-medium">
                连接入口
              </span>
              {form.targetNodeId === "" ? (
                <p className="text-xs text-muted-foreground">先选择目标节点，再选择它对外公布的地址。</p>
              ) : entries.length === 0 && !chosen ? (
                <p className="text-xs text-muted-foreground">
                  {nodeName(nodes, form.targetNodeId)} 还没有公布任何入口。先让它「接收其他节点连接」。
                </p>
              ) : (
                <RadioGroup
                  aria-labelledby={`${baseId}-entry`}
                  value={chosen}
                  onValueChange={(value) => {
                    const entry = entries.find((existing) => existing.value === value)
                    if (entry) setForm({ ...form, listenerId: entry.listener.id.trim(), host: entry.host, port: entry.port })
                  }}
                  className="gap-1.5"
                >
                  {chosen && !chosenListed ? (
                    <EntryOption
                      value={chosen}
                      transport={listener ? transportLabels[listener.transport] : "未知"}
                      endpoint={endpointLabel(form.host, form.port)}
                      listenerId={form.listenerId}
                      blocker="已不在目标节点的公布地址中，请重新选择"
                      danger
                    />
                  ) : null}
                  {entries.map((entry) => (
                    <EntryOption
                      key={entry.value}
                      value={entry.value}
                      transport={transportLabels[entry.listener.transport]}
                      endpoint={endpointLabel(entry.host, entry.port)}
                      listenerId={entry.listener.id.trim()}
                      blocker={entry.value === chosen ? null : entry.blocker}
                    />
                  ))}
                </RadioGroup>
              )}
              <FieldError messages={[...messages("candidates.listener_id"), ...messages("candidates.endpoint")]} />
            </div>
          </Section>

          {profile || form.credentialRef !== "" ? (
            <Section
              title="本地配置档"
              description="填写发起节点上已有 VLESS 配置档的名称。实际凭据只保存在节点本地。"
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

          <AdvancedSection
            summary={`ID ${form.id.trim() || "—"} · 优先级 ${form.priority.trim() || "—"}`}
            attention={has("candidates.id") || has("candidates.priority")}
          >
            <div className="grid gap-3 sm:grid-cols-[minmax(0,1fr)_8rem]">
              <div className="flex min-w-0 flex-col gap-1.5">
                <Label htmlFor={`${baseId}-id`}>候选 ID</Label>
                <Input
                  id={`${baseId}-id`}
                  value={form.id}
                  onChange={(event) => setForm({ ...form, id: event.target.value })}
                  autoComplete="off"
                  spellCheck={false}
                  aria-invalid={has("candidates.id") || undefined}
                  className="font-mono"
                />
                <FieldError messages={messages("candidates.id")} />
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
                  className="font-mono tabular-nums"
                />
                <FieldError messages={messages("candidates.priority")} />
              </div>
            </div>
            <span className="text-xs text-muted-foreground">
              同一对节点的多个入口按优先级主备（0–65535，越小越优先），不做负载分担。新入口自动排在已有入口之后。
            </span>
          </AdvancedSection>

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
          <Button type="button" variant="ghost" className="text-destructive" onClick={onDelete} disabled={disabled}>
            <Trash2Icon data-icon="inline-start" />
            删除
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

function EntryOption({
  value,
  transport,
  endpoint,
  listenerId,
  blocker,
  danger = false,
}: {
  value: string
  transport: string
  endpoint: string
  listenerId: string
  blocker: string | null
  danger?: boolean
}) {
  const id = useId()
  return (
    <label
      htmlFor={id}
      className={cn(
        "flex min-w-0 cursor-pointer items-start gap-3 rounded-lg border px-3 py-2.5 transition-colors hover:bg-muted/40 has-data-checked:border-primary/60 has-data-checked:bg-muted/40",
        blocker !== null && !danger && "cursor-not-allowed opacity-60",
        danger && "border-destructive/50"
      )}
    >
      <RadioGroupItem id={id} value={value} disabled={blocker !== null && !danger} className="mt-0.5" />
      <span className="flex min-w-0 flex-1 flex-col gap-1">
        <span className="flex min-w-0 flex-wrap items-center gap-2">
          <Badge variant="secondary" className="font-normal">
            {transport}
          </Badge>
          <span className="min-w-0 truncate font-mono text-sm">{endpoint}</span>
        </span>
        <span className={cn("text-[0.6875rem]", blocker ? (danger ? "text-destructive" : "text-warning") : "text-muted-foreground")}>
          {blocker ?? <span className="font-mono">监听 {listenerId}</span>}
        </span>
      </span>
    </label>
  )
}

export function DirectionPreview({
  dialer,
  acceptor,
  endpoint,
  transport,
  role,
}: {
  dialer: string
  acceptor: string
  endpoint: string | null
  transport: string | null
  role: string | null
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex min-w-0 flex-wrap items-center gap-2 text-sm">
        <span className="font-medium">{dialer}</span>
        <span className="flex items-center gap-1 text-xs text-muted-foreground">
          主动连接
          <ArrowRightIcon className="size-4" />
        </span>
        <span className="font-medium">{acceptor}</span>
        {endpoint ? <code className="min-w-0 truncate font-mono text-xs text-muted-foreground">{endpoint}</code> : null}
        {transport ? <span className="text-xs text-muted-foreground">· {transport}</span> : null}
        {role ? <Badge variant="outline" className="font-normal">{role}</Badge> : null}
      </div>
      <p className="flex items-start gap-1.5 text-[0.6875rem] text-muted-foreground">
        <InfoIcon className="mt-0.5 size-3 shrink-0" />
        箭头只表示谁发起连接。通道建立后两个方向都能复用；路由路径与访问授权另由路由策略决定。
      </p>
    </div>
  )
}
