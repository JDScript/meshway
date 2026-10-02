"use client"

import type { AccountRole } from "@meshway/control-client"
import { CircleAlertIcon } from "lucide-react"
import { useActionState, useId, useSyncExternalStore } from "react"

import { CopyButton } from "@/components/common/copy-button"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select"
import { SIGN_IN_FIELD } from "@/lib/action-scope"
import { roleLabels, type InvitationIssueState } from "@/lib/account-model"
import { createInvitationAction } from "@/lib/invitation-actions"

const initialState: InvitationIssueState = { status: "idle" }
const ttls = [
  { seconds: 86_400, label: "1 天" },
  { seconds: 3 * 86_400, label: "3 天" },
  { seconds: 7 * 86_400, label: "7 天" },
]
const origin = () => window.location.origin
const noOrigin = () => ""

/** One invitation link; shown once, to be handed over manually (no e-mail is sent). */
export function InvitationIssueForm({ signIn, networkId, roles }: { signIn: string; networkId: string; roles: AccountRole[] }) {
  const [state, action, pending] = useActionState(createInvitationAction, initialState)
  const base = useSyncExternalStore(() => () => undefined, origin, noOrigin)
  const ids = { email: useId(), role: useId(), ttl: useId(), link: useId() }
  return (
    <div className="flex flex-col gap-4">
      <form action={action} className="flex flex-col gap-3" aria-label="创建邀请链接">
        <input type="hidden" name="network_id" value={networkId} />
        <input type="hidden" name={SIGN_IN_FIELD} value={signIn} />
        <div className="flex flex-col gap-1.5">
          <Label htmlFor={ids.email}>受邀邮箱</Label>
          <Input id={ids.email} name="email" type="email" required autoComplete="off" autoCapitalize="off" spellCheck={false} disabled={pending} className="h-9" />
        </div>
        <div className="grid grid-cols-2 gap-3">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor={ids.role}>角色</Label>
            <NativeSelect id={ids.role} name="role" className="w-full" defaultValue="member" disabled={pending}>
              {roles.map((role) => (
                <NativeSelectOption key={role} value={role}>{roleLabels[role]}</NativeSelectOption>
              ))}
            </NativeSelect>
          </div>
          <div className="flex flex-col gap-1.5">
            <Label htmlFor={ids.ttl}>有效期</Label>
            <NativeSelect id={ids.ttl} name="expires_in_seconds" className="w-full" defaultValue={String(86_400)} disabled={pending}>
              {ttls.map((ttl) => (
                <NativeSelectOption key={ttl.seconds} value={String(ttl.seconds)}>{ttl.label}</NativeSelectOption>
              ))}
            </NativeSelect>
          </div>
        </div>
        {state.status === "error" ? (
          <p role="alert" className="flex items-start gap-1.5 text-xs break-words text-destructive">
            <CircleAlertIcon className="mt-px size-3.5 shrink-0" />
            {state.message}
          </p>
        ) : null}
        <Button type="submit" size="lg" disabled={pending}>
          {pending ? "正在创建" : "创建邀请链接"}
        </Button>
      </form>
      {state.status === "created" && base ? (
        <div role="status" className="flex flex-col gap-2 rounded-lg border bg-muted/40 p-3.5 text-[0.8125rem]">
          <Label htmlFor={ids.link}>
            {state.email} · {roleLabels[state.role]} · {new Date(state.expiresAt * 1000).toLocaleString("zh-CN")} 前有效
          </Label>
          <Input id={ids.link} readOnly value={`${base}${state.path}`} className="h-9 font-mono text-xs" onFocus={(event) => event.currentTarget.select()} />
          <div className="flex items-center justify-between gap-2">
            <p className="text-xs text-muted-foreground">只显示这一次，一次有效。请手动转交给受邀人。</p>
            <CopyButton value={`${base}${state.path}`} label="复制链接" />
          </div>
        </div>
      ) : null}
    </div>
  )
}
