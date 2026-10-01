"use client"

import type { ConnectionNode, MeshAccessRule, PolicyDiagnostic, TagSnapshot } from "@meshway/control-client"
import { ArrowLeftRightIcon, ArrowRightIcon, Trash2Icon } from "lucide-react"
import { useEffect, useId, useState } from "react"

import { SelectorPicker } from "@/components/access/selector-picker"
import { Segmented } from "@/components/common/segmented"
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
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Sheet, SheetContent, SheetDescription, SheetFooter, SheetHeader, SheetTitle } from "@/components/ui/sheet"
import { Switch } from "@/components/ui/switch"
import { accessFieldLabel } from "@/lib/access-model"
import {
  sameRuleForm,
  toAccessRule,
  updateRuleForm,
  validateRuleForm,
  type AccessRuleForm,
  type AccessRuleSession,
} from "@/lib/access-rule-form"

/**
 * Focused editor for one access rule. Edits stay in the form until "apply";
 * unapplied edits are reported through `onWorkingChange` so the page keeps
 * them across navigation. Closing with unapplied edits asks first.
 */
export function AccessRuleBuilder({
  session,
  open,
  dirty,
  onWorkingChange,
  onClose,
  onApply,
  onDelete,
  others,
  inventory,
  tags,
  diagnostics,
  diagnosticsStale,
  disabled,
}: {
  session: AccessRuleSession | null
  open: boolean
  dirty: boolean
  onWorkingChange: (form: AccessRuleForm | null) => void
  onClose: () => void
  onApply: (rule: MeshAccessRule) => void
  onDelete: () => void
  others: MeshAccessRule[]
  inventory: ConnectionNode[]
  tags: TagSnapshot
  diagnostics: PolicyDiagnostic[]
  diagnosticsStale: boolean
  disabled: boolean
}) {
  const [confirmDiscard, setConfirmDiscard] = useState(false)
  const requestClose = () => (dirty ? setConfirmDiscard(true) : onClose())

  return (
    <>
      <Sheet
        open={open}
        onOpenChange={(next) => {
          if (!next) requestClose()
        }}
      >
        <SheetContent className="w-full gap-0 data-[side=right]:w-full sm:max-w-xl data-[side=right]:sm:max-w-xl">
          {session ? (
            <BuilderForm
              key={session.id}
              session={session}
              others={others}
              inventory={inventory}
              tags={tags}
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
            <AlertDialogTitle>放弃这条规则的未应用修改？</AlertDialogTitle>
            <AlertDialogDescription>草稿中的其他规则不受影响。</AlertDialogDescription>
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
  tags,
  diagnostics,
  diagnosticsStale,
  disabled,
  onWorkingChange,
  onCancel,
  onApply,
  onDelete,
}: {
  session: AccessRuleSession
  others: MeshAccessRule[]
  inventory: ConnectionNode[]
  tags: TagSnapshot
  diagnostics: PolicyDiagnostic[]
  diagnosticsStale: boolean
  disabled: boolean
  onWorkingChange: (form: AccessRuleForm | null) => void
  onCancel: () => void
  onApply: (rule: MeshAccessRule) => void
  onDelete: () => void
}) {
  const [form, setForm] = useState<AccessRuleForm>(session.working ?? session.initial)
  const [touched, setTouched] = useState(false)
  const idField = useId()
  const directionField = useId()
  const taken = others.map((rule) => rule.id)
  const issues = validateRuleForm(form, others, inventory, tags)
  const fieldIssues = (prefix: string) => issues.filter((issue) => issue.field === prefix || issue.field.startsWith(`${prefix}[`))

  useEffect(() => {
    onWorkingChange(sameRuleForm(form, session.initial) ? null : form)
    // Report only when the form itself changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [form])

  const change = (patch: Partial<Omit<AccessRuleForm, "idEdited">>) => setForm((current) => updateRuleForm(current, patch, taken, inventory))
  const showIssues = touched || session.mode === "edit"

  return (
    <form
      data-testid="access-rule-builder"
      className="flex min-h-0 flex-1 flex-col"
      onSubmit={(event) => {
        event.preventDefault()
        setTouched(true)
        if (issues.length === 0) onApply(toAccessRule(form))
      }}
    >
      <SheetHeader className="border-b">
        <SheetTitle>{session.mode === "new" ? "添加访问规则" : "编辑访问规则"}</SheetTitle>
        <SheetDescription>
          规则按顺序匹配，第一条命中的规则决定允许或拒绝；都不命中时使用默认动作。路径由路由策略决定，这里只决定端点之间能否访问。
        </SheetDescription>
      </SheetHeader>

      <div className="flex min-h-0 flex-1 flex-col gap-6 overflow-y-auto p-4">
        <div className="flex flex-col gap-2">
          <span className="text-sm font-medium">动作</span>
          <Segmented
            aria-label="动作"
            value={form.action}
            onValueChange={(action) => change({ action })}
            options={[
              { value: "allow", label: "允许" },
              { value: "reject", label: "拒绝" },
            ]}
            disabled={disabled}
          />
        </div>

        <SelectorPicker
          legend="来源"
          testId="access-rule-source"
          value={form.source}
          onChange={(source) => change({ source })}
          inventory={inventory}
          tags={tags}
          disabled={disabled}
          invalid={showIssues && fieldIssues("source").length > 0}
        />
        <FieldIssues issues={showIssues ? fieldIssues("source") : []} />

        <div className="flex items-start gap-3 rounded-lg border p-3">
          <Switch
            id={directionField}
            data-testid="access-rule-bidirectional"
            checked={form.bidirectional}
            onCheckedChange={(bidirectional) => change({ bidirectional })}
            disabled={disabled}
          />
          <Label htmlFor={directionField} className="flex flex-col items-start gap-1 font-normal">
            <span className="flex items-center gap-1.5 text-sm font-medium">
              {form.bidirectional ? <ArrowLeftRightIcon className="size-4" /> : <ArrowRightIcon className="size-4" />}
              {form.bidirectional ? "双向" : "单向"}
            </span>
            <span className="text-xs text-muted-foreground">
              {form.bidirectional
                ? "同一条规则在同一位置也匹配目标 → 来源的流量。两个方向各自检查，不是状态型回包跟踪。"
                : "只匹配来源 → 目标。回程流量需要另一条规则或默认允许。"}
            </span>
          </Label>
        </div>

        <SelectorPicker
          legend="目标"
          testId="access-rule-destination"
          value={form.destination}
          onChange={(destination) => change({ destination })}
          inventory={inventory}
          tags={tags}
          disabled={disabled}
          invalid={showIssues && fieldIssues("destination").length > 0}
        />
        <FieldIssues issues={showIssues ? fieldIssues("destination") : []} />

        <div className="flex flex-col gap-1.5">
          <Label htmlFor={idField}>规则 ID</Label>
          <Input
            id={idField}
            data-testid="access-rule-id"
            value={form.id}
            onChange={(event) => change({ id: event.target.value })}
            aria-invalid={(showIssues && fieldIssues("id").length > 0) || undefined}
            className="font-mono"
            disabled={disabled}
          />
          <p className="text-xs text-muted-foreground">
            {form.idEdited ? "用于诊断和流量预览中指明命中的规则。" : "按动作和端点自动生成；输入后不再自动更新。"}
          </p>
          <FieldIssues issues={showIssues ? fieldIssues("id") : []} />
        </div>

        {diagnostics.length > 0 ? (
          <div className="flex flex-col gap-1 rounded-lg border border-destructive/40 p-3 text-xs">
            <span className="font-medium text-destructive">
              Controller 诊断{diagnosticsStale ? "（针对已保存版本）" : ""}
            </span>
            <ul className="flex flex-col gap-1">
              {diagnostics.map((diagnostic, index) => (
                <li key={`${diagnostic.field}-${index}`} className="break-words">
                  {accessFieldLabel(diagnostic.field)}：{diagnostic.message}
                </li>
              ))}
            </ul>
          </div>
        ) : null}
      </div>

      <SheetFooter className="flex-row items-center gap-2 border-t">
        {session.mode === "edit" ? (
          <Button type="button" variant="ghost" className="mr-auto text-destructive" disabled={disabled} onClick={onDelete}>
            <Trash2Icon data-icon="inline-start" />
            删除规则
          </Button>
        ) : (
          <span className="mr-auto" />
        )}
        <Button type="button" variant="outline" onClick={onCancel}>
          取消
        </Button>
        <Button type="submit" data-testid="access-rule-apply" disabled={disabled || (showIssues && issues.length > 0)}>
          {session.mode === "new" ? "添加到草稿" : "应用到草稿"}
        </Button>
      </SheetFooter>
    </form>
  )
}

function FieldIssues({ issues }: { issues: { message: string }[] }) {
  if (issues.length === 0) return null
  return (
    <ul className="-mt-4 flex flex-col gap-0.5 text-xs text-destructive">
      {issues.map((issue, index) => (
        <li key={index}>{issue.message}</li>
      ))}
    </ul>
  )
}
