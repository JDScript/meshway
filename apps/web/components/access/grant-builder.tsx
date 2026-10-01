"use client"

import type { ConnectionNode, MeshAccessGrant, PolicyDiagnostic } from "@meshway/control-client"
import {
  ArrowDownIcon,
  ArrowUpIcon,
  CircleAlertIcon,
  EraserIcon,
  HistoryIcon,
  InfoIcon,
  PlusIcon,
  Trash2Icon,
  XIcon,
} from "lucide-react"
import { useId, useMemo, useState, type ReactNode } from "react"

import { AccessNodePicker } from "@/components/access/access-node-picker"
import { GrantPath } from "@/components/access/grant-path"
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Label } from "@/components/ui/label"
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet"
import {
  ACCESS_LIMITS,
  accessFieldLabel,
  formPath,
  grantsFromForm,
  reverseExists,
  sameGrantForm,
  validateGrantForm,
  type AccessIssue,
  type GrantForm,
  type GrantSession,
} from "@/lib/access-model"
import { cn } from "@/lib/utils"

const MAX_VIA = ACCESS_LIMITS.pathMax - 2

/**
 * Focused editor for one grant's exact path. Edits stay in the form until
 * "apply"; unapplied edits are kept by the editor in the console-scoped
 * draft store. Closing with unapplied edits asks first.
 */
export function GrantBuilder({
  session,
  open,
  dirty,
  onWorkingChange,
  onClose,
  onApply,
  onDelete,
  others,
  inventory,
  diagnostics,
  diagnosticsStale,
  disabled,
}: {
  session: GrantSession | null
  open: boolean
  dirty: boolean
  onWorkingChange: (form: GrantForm | null) => void
  onClose: () => void
  onApply: (grants: MeshAccessGrant[]) => void
  onDelete: () => void
  others: MeshAccessGrant[]
  inventory: ConnectionNode[]
  diagnostics: PolicyDiagnostic[]
  diagnosticsStale: boolean
  disabled: boolean
}) {
  const [confirmDiscard, setConfirmDiscard] = useState(false)
  const requestClose = () => {
    if (dirty) setConfirmDiscard(true)
    else onClose()
  }

  return (
    <>
      <Sheet
        open={open}
        onOpenChange={(next) => {
          if (!next) requestClose()
        }}
      >
        <SheetContent
          data-testid="grant-builder"
          className="w-full gap-0 data-[side=right]:w-full sm:max-w-lg data-[side=right]:sm:max-w-lg"
        >
          {session ? (
            <BuilderForm
              key={session.id}
              session={session}
              others={others}
              inventory={inventory}
              diagnostics={diagnostics}
              diagnosticsStale={diagnosticsStale}
              disabled={disabled}
              onWorkingChange={onWorkingChange}
              onCancel={requestClose}
              onApply={onApply}
              onDelete={onDelete}
            />
          ) : null}
        </SheetContent>
      </Sheet>
      <AlertDialog open={confirmDiscard} onOpenChange={setConfirmDiscard}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>放弃对这条授权的修改？</AlertDialogTitle>
            <AlertDialogDescription>
              表单中的修改尚未应用到草稿。授权草稿本身不受影响。
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>继续编辑</AlertDialogCancel>
            <Button
              variant="destructive"
              onClick={() => {
                setConfirmDiscard(false)
                onClose()
              }}
            >
              放弃修改
            </Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}

function BuilderForm({
  session,
  others,
  inventory,
  diagnostics,
  diagnosticsStale,
  disabled,
  onWorkingChange,
  onCancel,
  onApply,
  onDelete,
}: {
  session: GrantSession
  others: MeshAccessGrant[]
  inventory: ConnectionNode[]
  diagnostics: PolicyDiagnostic[]
  diagnosticsStale: boolean
  disabled: boolean
  onWorkingChange: (form: GrantForm | null) => void
  onCancel: () => void
  onApply: (grants: MeshAccessGrant[]) => void
  onDelete: () => void
}) {
  const [form, setFormState] = useState<GrantForm>(session.working ?? session.initial)
  const [attempted, setAttempted] = useState(false)
  const baseId = useId()

  const setForm = (next: GrantForm) => {
    setFormState(next)
    onWorkingChange(sameGrantForm(next, session.initial) ? null : next)
  }

  const issues = useMemo(() => validateGrantForm(form, others, inventory), [form, others, inventory])
  const path = formPath(form)
  const visible = attempted || session.mode === "edit"
  const fieldIssues = (field: string) => (visible ? issues.filter((issue) => issue.field === field) : [])
  const invalidAt = (index: number) => fieldIssues(`path[${index}]`).length > 0
  const usedExcept = (index: number) => path.filter((id, i) => i !== index && id !== "")
  const reversePresent = reverseExists(form, others)
  const lastIndex = path.length - 1

  const moveVia = (position: number, offset: -1 | 1) => {
    const via = [...form.via]
    const target = position + offset
    const [moved] = via.splice(position, 1)
    if (moved === undefined || target < 0 || target > via.length) return
    via.splice(target, 0, moved)
    setForm({ ...form, via })
  }

  const apply = () => {
    setAttempted(true)
    if (issues.length > 0) return
    onApply(grantsFromForm(form, others))
  }

  return (
    <form
      className="flex min-h-0 flex-1 flex-col"
      onSubmit={(event) => {
        event.preventDefault()
        apply()
      }}
    >
      <SheetHeader className="gap-1 border-b p-4 pr-12">
        <SheetTitle className="text-base">
          {session.mode === "new" ? "新建访问授权" : "编辑访问授权"}
        </SheetTitle>
        <SheetDescription>
          按顺序选择完整路径。只有路由编译出的路径与它完全一致时，这条授权才会被使用。
        </SheetDescription>
      </SheetHeader>

      <div className="flex-1 overflow-y-auto">
        <div className="flex flex-col gap-2 border-b bg-muted/30 px-4 py-3">
          <GrantPath path={path} inventory={inventory} />
          <p className="text-xs text-muted-foreground">
            单向：只允许从来源发往目标的 Mesh IPv4 报文，不自动允许回包或反向发起。
          </p>
        </div>

        <div className="flex flex-col divide-y">
          <Section title="路径" description={`含来源和目标共 ${ACCESS_LIMITS.pathMin}–${ACCESS_LIMITS.pathMax} 个节点，同一节点不能重复。`}>
            <ol className="flex flex-col">
              <PathRow marker="来" label="来源节点">
                <AccessNodePicker
                  id={`${baseId}-source`}
                  aria-label="来源节点"
                  data-testid="grant-source"
                  inventory={inventory}
                  value={form.source}
                  used={usedExcept(0)}
                  onValueChange={(source) => setForm({ ...form, source })}
                  placeholder="选择来源"
                  invalid={invalidAt(0)}
                  className="w-full"
                />
              </PathRow>
              <FieldMessages item issues={fieldIssues("path[0]")} />
              {form.via.map((nodeId, position) => {
                const index = position + 1
                return (
                  <li key={position} className="flex flex-col">
                    <div className="relative flex items-center gap-1.5 py-1 pl-7">
                      <Rail />
                      <span
                        aria-hidden="true"
                        className="absolute left-1 flex size-4 items-center justify-center rounded-full bg-muted font-mono text-[0.5625rem] text-muted-foreground"
                      >
                        {position + 1}
                      </span>
                      <Label htmlFor={`${baseId}-via-${position}`} className="sr-only">
                        经由节点 {position + 1}
                      </Label>
                      <AccessNodePicker
                        id={`${baseId}-via-${position}`}
                        data-testid={`grant-via-${position}`}
                        inventory={inventory}
                        value={nodeId}
                        used={usedExcept(index)}
                        onValueChange={(value) =>
                          setForm({
                            ...form,
                            via: form.via.map((existing, i) => (i === position ? value : existing)),
                          })
                        }
                        placeholder="选择经由节点"
                        invalid={invalidAt(index)}
                        className="min-w-0 flex-1"
                      />
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon-sm"
                        aria-label={`经由节点 ${position + 1} 上移`}
                        disabled={position === 0}
                        onClick={() => moveVia(position, -1)}
                      >
                        <ArrowUpIcon />
                      </Button>
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon-sm"
                        aria-label={`经由节点 ${position + 1} 下移`}
                        disabled={position === form.via.length - 1}
                        onClick={() => moveVia(position, 1)}
                      >
                        <ArrowDownIcon />
                      </Button>
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon-sm"
                        aria-label={`移除经由节点 ${position + 1}`}
                        onClick={() => setForm({ ...form, via: form.via.filter((_, i) => i !== position) })}
                      >
                        <XIcon />
                      </Button>
                    </div>
                    <FieldMessages issues={fieldIssues(`path[${index}]`)} />
                  </li>
                )
              })}
              <li className="relative py-1 pl-7">
                <Rail />
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  data-testid="grant-add-via"
                  disabled={form.via.length >= MAX_VIA}
                  onClick={() => setForm({ ...form, via: [...form.via, ""] })}
                >
                  <PlusIcon data-icon="inline-start" />
                  添加经由节点（{form.via.length}/{MAX_VIA}）
                </Button>
              </li>
              <PathRow marker="目" label="目标节点">
                <AccessNodePicker
                  id={`${baseId}-destination`}
                  aria-label="目标节点"
                  data-testid="grant-destination"
                  inventory={inventory}
                  value={form.destination}
                  used={usedExcept(lastIndex)}
                  onValueChange={(destination) => setForm({ ...form, destination })}
                  placeholder="选择目标"
                  invalid={invalidAt(lastIndex)}
                  className="w-full"
                />
              </PathRow>
              <FieldMessages item issues={fieldIssues(`path[${lastIndex}]`)} />
            </ol>
            <FieldMessages issues={[...fieldIssues("path"), ...fieldIssues("grants")]} />
          </Section>

          <Section title="反向">
            <label className="flex cursor-pointer items-start gap-2.5 text-sm">
              <Checkbox
                data-testid="grant-add-reverse"
                checked={form.addReverse}
                disabled={reversePresent}
                onCheckedChange={(checked) => setForm({ ...form, addReverse: checked === true })}
                className="mt-0.5"
              />
              <span className="flex flex-col gap-0.5">
                <span>同时添加反向授权</span>
                <span className="text-xs text-muted-foreground">
                  {reversePresent
                    ? "草稿中已有完全相反路径的授权。"
                    : "另建一条按相反顺序经过同样节点的授权。ping 回复、TCP 回包与反向发起都需要它，并且仍需要一条把反向流量精确路由到这条路径的路由规则。"}
                </span>
              </span>
            </label>
          </Section>

          {diagnostics.length > 0 ? (
            <Section title="Controller 诊断">
              <ul
                aria-label="授权的诊断"
                className={cn(
                  "flex flex-col gap-1.5 text-xs",
                  diagnosticsStale ? "text-muted-foreground" : "text-destructive"
                )}
              >
                {diagnostics.map((diagnostic, index) => {
                  const Icon = diagnosticsStale ? HistoryIcon : CircleAlertIcon
                  return (
                    <li key={index} className="flex items-start gap-1.5">
                      <Icon className="mt-0.5 size-3.5 shrink-0" />
                      <span className="min-w-0 break-words">
                        <span className="font-medium">{accessFieldLabel(diagnostic.field)}</span>：{diagnostic.message}
                      </span>
                    </li>
                  )
                })}
              </ul>
              {diagnosticsStale ? (
                <p className="text-xs text-muted-foreground">诊断来自已保存的版本，草稿已修改。</p>
              ) : null}
            </Section>
          ) : null}

          <p className="flex items-start gap-1.5 px-4 py-3 text-xs text-muted-foreground">
            <InfoIcon className="mt-0.5 size-3.5 shrink-0" />
            授权放行两个端点 Mesh IPv4 地址之间的全部 IPv4 报文，不区分协议或端口，也不是互联网出口。
          </p>
        </div>
      </div>

      <SheetFooter className="flex-row items-center gap-2 border-t p-3">
        {session.mode === "edit" ? (
          <Button
            type="button"
            variant="ghost"
            className="text-destructive"
            data-testid="grant-delete"
            onClick={onDelete}
            disabled={disabled}
          >
            <Trash2Icon data-icon="inline-start" />
            删除
          </Button>
        ) : null}
        <Button
          type="button"
          variant="ghost"
          disabled={form.source === "" && form.destination === "" && form.via.length === 0}
          onClick={() => setForm({ ...form, source: "", via: [], destination: "" })}
        >
          <EraserIcon data-icon="inline-start" />
          清空路径
        </Button>
        <div className="ml-auto flex items-center gap-2">
          <Button type="button" variant="outline" onClick={onCancel}>
            取消
          </Button>
          <Button type="submit" data-testid="grant-apply" disabled={disabled}>
            {session.mode === "new" ? "添加到草稿" : "应用到草稿"}
          </Button>
        </div>
      </SheetFooter>
    </form>
  )
}

function Rail() {
  return <span aria-hidden="true" className="absolute top-0 bottom-0 left-3 border-l border-dashed border-brand/50" />
}

function PathRow({ marker, label, children }: { marker: string; label: string; children: ReactNode }) {
  return (
    <li className="flex flex-col gap-1 py-1">
      <span className="flex items-center gap-2 text-xs font-medium">
        <span
          aria-hidden="true"
          className="flex size-6 items-center justify-center rounded-full bg-brand text-[0.625rem] font-semibold text-brand-foreground"
        >
          {marker}
        </span>
        {label}
      </span>
      {children}
    </li>
  )
}

function FieldMessages({ issues, item = false }: { issues: AccessIssue[]; item?: boolean }) {
  if (issues.length === 0) return null
  const Element = item ? "li" : "p"
  return (
    <Element role="alert" className="py-0.5 text-xs text-destructive">
      {issues.map((issue) => issue.message).join(" ")}
    </Element>
  )
}

function Section({
  title,
  description,
  children,
}: {
  title: string
  description?: string
  children: ReactNode
}) {
  return (
    <section className="flex flex-col gap-3 px-4 py-4">
      <div className="flex flex-col gap-0.5">
        <h3 className="text-xs font-semibold tracking-wide text-muted-foreground uppercase">{title}</h3>
        {description ? <p className="text-xs text-muted-foreground">{description}</p> : null}
      </div>
      {children}
    </section>
  )
}
