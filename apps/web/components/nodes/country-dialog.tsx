"use client"

import type { NodeSummary } from "@meshway/control-client"
import { CircleAlertIcon } from "lucide-react"
import { useActionState, useId } from "react"

import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Field, FieldDescription, FieldLabel } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import type { CountryState } from "@/lib/action-state"
import { setNodeCountryAction } from "@/lib/admin-actions"

const initialState: CountryState = { status: "idle" }

/**
 * Administrator-managed country label, saved conditionally on the global
 * configuration revision the inventory was read at.
 */
export function CountryDialog({
  node,
  open,
  onOpenChange,
}: {
  node: NodeSummary
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        {/* Mounted only while open, so each opening starts from fresh state. */}
        <CountryForm node={node} onSaved={() => onOpenChange(false)} />
      </DialogContent>
    </Dialog>
  )
}

function CountryForm({ node, onSaved }: { node: NodeSummary; onSaved: () => void }) {
  const inputId = useId()
  const hintId = useId()
  const [state, formAction, pending] = useActionState(
    async (previous: CountryState, formData: FormData) => {
      const result = await setNodeCountryAction(previous, formData)
      if (result.status === "saved") onSaved()
      return result
    },
    initialState
  )

  const error = state.status === "error" || state.status === "conflict" ? state.message : null

  return (
    <form action={formAction} className="grid gap-4">
      <input type="hidden" name="node_id" value={node.id} />
      <input type="hidden" name="expected_revision" value={node.configuration_revision} />
      <DialogHeader>
        <DialogTitle>设置「{node.name}」的国家</DialogTitle>
        <DialogDescription>
          管理员标注的元数据，用于路由规则的来源与 Mesh 目标国家匹配；不是 GeoIP 结果。保存会使配置版本递增。
        </DialogDescription>
      </DialogHeader>
      <Field data-invalid={error !== null || undefined}>
        <FieldLabel htmlFor={inputId}>国家代码</FieldLabel>
        <Input
          id={inputId}
          name="country"
          defaultValue={node.country ?? ""}
          maxLength={2}
          autoComplete="off"
          spellCheck={false}
          placeholder="留空以清除"
          aria-describedby={hintId}
          aria-invalid={error !== null || undefined}
          className="w-28 font-mono uppercase"
          autoFocus
        />
        <FieldDescription id={hintId}>
          两位字母，如 CN、US、JP。基于配置版本 r{node.configuration_revision} 条件保存。
        </FieldDescription>
      </Field>
      {error !== null ? (
        <p role="alert" className="flex items-start gap-1.5 text-xs/relaxed text-destructive">
          <CircleAlertIcon className="mt-0.5 size-3.5 shrink-0" />
          {error}
        </p>
      ) : null}
      <DialogFooter>
        <DialogClose render={<Button variant="outline" disabled={pending} />}>取消</DialogClose>
        <Button type="submit" disabled={pending}>
          {pending ? "正在保存" : "保存国家"}
        </Button>
      </DialogFooter>
    </form>
  )
}
