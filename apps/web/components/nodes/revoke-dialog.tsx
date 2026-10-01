"use client"

import { useNetworkId } from "@/components/network/network-workspace"

import type { NodeSummary } from "@meshway/control-client"
import { CircleAlertIcon } from "lucide-react"
import { useActionState } from "react"

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
import type { RevokeState } from "@/lib/action-state"
import { revokeNodeAction } from "@/lib/admin-actions"

const initialState: RevokeState = { status: "idle" }

export function RevokeDialog({
  node,
  open,
  onOpenChange,
}: {
  node: NodeSummary
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <RevokeForm node={node} onRevoked={() => onOpenChange(false)} />
      </AlertDialogContent>
    </AlertDialog>
  )
}

function RevokeForm({ node, onRevoked }: { node: NodeSummary; onRevoked: () => void }) {
  const networkId = useNetworkId()
  const [state, formAction, pending] = useActionState(
    async (previous: RevokeState, formData: FormData) => {
      const result = await revokeNodeAction(networkId, previous, formData)
      if (result.status === "revoked") onRevoked()
      return result
    },
    initialState
  )

  return (
    <form action={formAction} className="grid gap-4">
      <input type="hidden" name="node_id" value={node.id} />
      <AlertDialogHeader>
        <AlertDialogTitle>撤销节点「{node.name}」？</AlertDialogTitle>
        <AlertDialogDescription>
          Controller 将拒绝该节点的控制面凭据，记录保留并标记为已撤销。不能在控制台中恢复；节点需要新的注册令牌重新注册。引用该节点的路由规则会变为无效。
        </AlertDialogDescription>
      </AlertDialogHeader>
      {state.status === "error" ? (
        <p role="alert" className="flex items-start gap-1.5 text-xs/relaxed text-destructive">
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
  )
}
