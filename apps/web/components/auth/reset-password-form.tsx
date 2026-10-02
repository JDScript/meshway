"use client"

import { CircleAlertIcon } from "lucide-react"
import Link from "next/link"
import { useActionState, useEffect, useId, useState } from "react"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { resetTokenFromHash } from "@/lib/members-model"
import { redeemResetAction, type ResetRedeemState } from "@/lib/reset-actions"

const initialState: ResetRedeemState = { status: "idle" }

/** The reset secret lives in the link's fragment: read once, removed from the address bar, posted only with the form. */
export function ResetPasswordForm() {
  const [token, setToken] = useState<string | null | undefined>(undefined)
  useEffect(() => {
    const value = resetTokenFromHash(window.location.hash)
    if (window.location.hash) history.replaceState(history.state, "", window.location.pathname)
    // Reading the fragment is only possible after hydration.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setToken(value)
  }, [])
  const [state, action, pending] = useActionState(redeemResetAction, initialState)
  const ids = { password: useId(), confirmation: useId() }

  if (token === undefined) return <p className="text-[0.8125rem] text-muted-foreground">正在读取链接…</p>
  if (token === null) {
    return (
      <div className="flex flex-col gap-3">
        <p role="alert" className="text-[0.8125rem] text-destructive">链接不完整或无效。请向实例管理员索取新的重置链接，并完整复制。</p>
        <Link href="/login" className="text-xs text-muted-foreground underline underline-offset-4">返回登录</Link>
      </div>
    )
  }
  return (
    <form action={action} className="flex flex-col gap-4" aria-label="设置新密码">
      <input type="hidden" name="reset_token" value={token} />
      <div className="flex flex-col gap-1.5">
        <Label htmlFor={ids.password}>新密码</Label>
        <Input id={ids.password} name="new_password" type="password" required autoComplete="new-password" disabled={pending} className="h-9" autoFocus />
        <p className="text-xs text-muted-foreground">至少 12 个字节。</p>
      </div>
      <div className="flex flex-col gap-1.5">
        <Label htmlFor={ids.confirmation}>确认新密码</Label>
        <Input id={ids.confirmation} name="new_password_confirmation" type="password" required autoComplete="new-password" disabled={pending} className="h-9" />
      </div>
      {state.status === "error" ? (
        <p role="alert" className="flex items-start gap-1.5 text-xs break-words text-destructive">
          <CircleAlertIcon className="mt-px size-3.5 shrink-0" />
          {state.message}
        </p>
      ) : null}
      <Button type="submit" size="lg" disabled={pending}>
        {pending ? "正在保存" : "设置新密码"}
      </Button>
    </form>
  )
}
