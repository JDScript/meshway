"use client"

import { CircleAlertIcon } from "lucide-react"
import { useActionState, useState } from "react"

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
import type { RevokeState } from "@/lib/action-state"
import { revokeNodeAction } from "@/lib/admin-actions"

const initialState: RevokeState = { status: "idle" }

export function RevokeNodeButton({
  nodeId,
  nodeName,
}: {
  nodeId: string
  nodeName: string
}) {
  const [open, setOpen] = useState(false)
  const [state, formAction, pending] = useActionState(
    async (previous: RevokeState, formData: FormData) => {
      const result = await revokeNodeAction(previous, formData)
      if (result.status === "revoked") setOpen(false)
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
          <Button
            variant="destructive"
            size="sm"
            aria-label={`撤销节点 ${nodeName}`}
          />
        }
      >
        撤销
      </AlertDialogTrigger>
      <AlertDialogContent>
        <form action={formAction} className="grid gap-3">
          <input type="hidden" name="node_id" value={nodeId} />
          <AlertDialogHeader>
            <AlertDialogTitle>撤销节点「{nodeName}」？</AlertDialogTitle>
            <AlertDialogDescription>
              撤销后 controller 会拒绝该节点的控制面凭据，心跳不再被接受。记录会保留并标记为已撤销；此操作不能在控制台中恢复，节点需要用新的注册令牌重新注册。
            </AlertDialogDescription>
          </AlertDialogHeader>
          {state.status === "error" ? (
            <p
              role="alert"
              className="flex items-start gap-1.5 text-xs/relaxed text-destructive"
            >
              <CircleAlertIcon className="mt-0.5 size-3.5 shrink-0" />
              {state.message}
            </p>
          ) : null}
          <AlertDialogFooter>
            <AlertDialogCancel disabled={pending}>取消</AlertDialogCancel>
            <Button type="submit" variant="destructive" disabled={pending}>
              {pending ? "正在撤销" : "确认撤销"}
            </Button>
          </AlertDialogFooter>
        </form>
      </AlertDialogContent>
    </AlertDialog>
  )
}
