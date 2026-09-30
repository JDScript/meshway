"use client"

import { CircleAlertIcon } from "lucide-react"
import { useActionState, useId, useState } from "react"

import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import type { CountryState } from "@/lib/action-state"
import { setNodeCountryAction } from "@/lib/admin-actions"

const initialState: CountryState = { status: "idle" }

export function NodeCountryButton({
  nodeId,
  nodeName,
  country,
  expectedRevision,
}: {
  nodeId: string
  nodeName: string
  country: string | null
  /** Global configuration revision read together with this node. */
  expectedRevision: number
}) {
  const [open, setOpen] = useState(false)
  const inputId = useId()
  const hintId = useId()
  const [state, formAction, pending] = useActionState(
    async (previous: CountryState, formData: FormData) => {
      const result = await setNodeCountryAction(previous, formData)
      if (result.status === "saved") setOpen(false)
      return result
    },
    initialState
  )

  return (
    <AlertDialog
      open={open}
      onOpenChange={(next) => {
        if (!pending) setOpen(next)
      }}
    >
      <AlertDialogTrigger
        render={
          <Button variant="ghost" size="sm" aria-label={`设置节点 ${nodeName} 的国家`} />
        }
      >
        {country ?? "设置"}
      </AlertDialogTrigger>
      <AlertDialogContent>
        <form action={formAction} className="grid gap-3">
          <input type="hidden" name="node_id" value={nodeId} />
          <input type="hidden" name="expected_revision" value={expectedRevision} />
          <AlertDialogHeader>
            <AlertDialogTitle>节点「{nodeName}」的国家</AlertDialogTitle>
            <AlertDialogDescription>
              管理员标注的元数据，用于路由策略中的来源与 Mesh 目标国家匹配。它不是 GeoIP
              结果，也不证明节点身份。修改会使配置版本递增并重新下发给节点。
            </AlertDialogDescription>
          </AlertDialogHeader>
          <div className="flex flex-col gap-2">
            <Label htmlFor={inputId}>国家代码</Label>
            <Input
              id={inputId}
              name="country"
              defaultValue={country ?? ""}
              maxLength={2}
              autoComplete="off"
              spellCheck={false}
              placeholder="留空表示不设置"
              aria-describedby={hintId}
              className="w-32 font-mono uppercase"
            />
            <p id={hintId} className="text-xs text-muted-foreground">
              两位字母，如 CN、US、JP。基于配置版本 {expectedRevision} 条件保存。
            </p>
          </div>
          {state.status === "error" || state.status === "conflict" ? (
            <p role="alert" className="flex items-start gap-1.5 text-xs/relaxed text-destructive">
              <CircleAlertIcon className="mt-0.5 size-3.5 shrink-0" />
              {state.message}
            </p>
          ) : null}
          <AlertDialogFooter>
            <AlertDialogCancel disabled={pending}>取消</AlertDialogCancel>
            <Button type="submit" disabled={pending}>
              {pending ? "正在保存" : "保存国家"}
            </Button>
          </AlertDialogFooter>
        </form>
      </AlertDialogContent>
    </AlertDialog>
  )
}
