"use client"

import { CircleAlertIcon } from "lucide-react"
import { useActionState, useEffect, useId, useState } from "react"

import { SwitchAccountButton } from "@/components/auth/switch-account-button"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { invitationTokenFromHash, type InvitationState } from "@/lib/account-model"
import { acceptInvitationAction, registerInvitationAction } from "@/lib/invitation-actions"

const initialState: InvitationState = { status: "idle" }

/**
 * The invitation secret travels in the link's fragment, which browsers never
 * send to a server or put in a Referer. It is read once into memory, removed
 * from the address bar, and posted only with the form.
 */
function useInvitationToken(): string | null | undefined {
  const [token, setToken] = useState<string | null | undefined>(undefined)
  useEffect(() => {
    const value = invitationTokenFromHash(window.location.hash)
    if (window.location.hash) history.replaceState(history.state, "", window.location.pathname)
    // Reading the fragment is only possible after hydration.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setToken(value)
  }, [])
  return token
}

function Failure({ state }: { state: InvitationState }) {
  return state.status === "error" ? (
    <p role="alert" className="flex items-start gap-1.5 text-xs break-words text-destructive">
      <CircleAlertIcon className="mt-px size-3.5 shrink-0" />
      {state.message}
    </p>
  ) : null
}

export function InvitationForm({ signedInEmail, administrator }: { signedInEmail: string | null; administrator: boolean }) {
  const token = useInvitationToken()
  const [registerState, register, registering] = useActionState(registerInvitationAction, initialState)
  const [acceptState, accept, accepting] = useActionState(acceptInvitationAction, initialState)
  const ids = { email: useId(), password: useId(), confirmation: useId() }

  if (token === undefined) return <p className="text-[0.8125rem] text-muted-foreground">正在读取邀请…</p>
  if (token === null) {
    return (
      <p role="alert" className="text-[0.8125rem] text-destructive">
        链接中没有有效的邀请。请完整复制网络所有者发给你的邀请链接后重新打开。
      </p>
    )
  }

  if (signedInEmail !== null) {
    return (
      <div className="flex flex-col gap-3">
        <form action={accept} className="flex flex-col gap-3">
          <input type="hidden" name="invitation_token" value={token} />
          <p className="text-[0.8125rem] text-muted-foreground">
            以 <span className="font-medium break-all text-foreground">{signedInEmail}</span> 接受邀请。邀请绑定受邀邮箱，与当前账户不符时会被拒绝。
          </p>
          <Failure state={acceptState} />
          <Button type="submit" size="lg" disabled={accepting}>
            {accepting ? "正在加入" : "接受邀请"}
          </Button>
        </form>
        <SwitchAccountButton returnTo="/" label="不是这个账户？退出登录" variant="link" />
      </div>
    )
  }

  return (
    <form action={register} className="flex flex-col gap-3" aria-label="注册账户">
      <input type="hidden" name="invitation_token" value={token} />
      {administrator ? (
        <p className="text-xs text-muted-foreground">注册后会以新账户登录，并结束当前的管理员令牌会话。</p>
      ) : null}
      <div className="flex flex-col gap-1.5">
        <Label htmlFor={ids.email}>邮箱</Label>
        <Input id={ids.email} name="email" type="email" required autoComplete="username" autoCapitalize="off" spellCheck={false} disabled={registering} className="h-9" />
        <p className="text-xs text-muted-foreground">必须是收到邀请的邮箱。</p>
      </div>
      <div className="flex flex-col gap-1.5">
        <Label htmlFor={ids.password}>设置密码</Label>
        <Input id={ids.password} name="password" type="password" required autoComplete="new-password" disabled={registering} className="h-9" />
        <p className="text-xs text-muted-foreground">至少 12 个字节。</p>
      </div>
      <div className="flex flex-col gap-1.5">
        <Label htmlFor={ids.confirmation}>确认密码</Label>
        <Input id={ids.confirmation} name="password_confirmation" type="password" required autoComplete="new-password" disabled={registering} className="h-9" />
      </div>
      <Failure state={registerState} />
      <Button type="submit" size="lg" disabled={registering}>
        {registering ? "正在注册" : "注册并加入"}
      </Button>
      <p className="text-xs text-muted-foreground">已经有账户？先登录，再重新打开这个邀请链接。</p>
    </form>
  )
}
