"use client"

import type { AccountRole, NodeSummary } from "@meshway/control-client"
import { CheckCircle2Icon, CircleAlertIcon, LogOutIcon, MonitorSmartphoneIcon } from "lucide-react"
import Link from "next/link"
import { useActionState, useId, useState, useTransition, type ReactNode } from "react"

import { signOutRendered } from "@/components/auth/sign-out"
import { StatusBadge } from "@/components/common/status-badge"
import { useDraftVault, useSignIn } from "@/components/network/network-workspace"
import { LastContact } from "@/components/nodes/node-status"
import { useRoutes } from "@/components/shell/network-context"
import { PageHeader } from "@/components/shell/page-header"
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Avatar, AvatarFallback } from "@/components/ui/avatar"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { changePasswordAction, revokeAllSessionsAction, type PasswordChangeState } from "@/lib/account-actions"
import { roleLabels } from "@/lib/account-model"
import { consoleRoutes } from "@/lib/routes"
import { cn } from "@/lib/utils"

export function AccountWorkspace({
  email,
  emailVerified,
  memberships,
  currentNetwork,
  devices,
  fetchedAt,
  changed,
}: {
  email: string
  emailVerified: boolean
  memberships: { networkId: string; role: AccountRole; name: string | null }[]
  currentNetwork: string
  devices: { ok: true; nodes: NodeSummary[] } | { ok: false; error: string }
  fetchedAt: number
  changed: boolean
}) {
  const routes = useRoutes()
  return (
    <>
      <div className="flex items-center gap-4">
        <Avatar className="size-12 rounded-full">
          <AvatarFallback className="bg-primary text-base font-semibold text-primary-foreground">{email.slice(0, 1).toUpperCase()}</AvatarFallback>
        </Avatar>
        <div className="min-w-0">
          <PageHeader title={email} meta={<span>{emailVerified ? "邮箱已验证" : "邮箱未验证（没有发送邮件的通道，邀请链接不证明邮箱所有权）"}</span>} />
        </div>
      </div>
      {changed ? (
        <p role="status" className="flex items-center gap-2 rounded-lg border border-success/30 bg-success/8 px-3 py-2 text-[0.8125rem]">
          <CheckCircle2Icon className="size-4 text-success" aria-hidden="true" />
          密码已修改。其他浏览器上的登录都已失效，这里已用新密码重新登录。
        </p>
      ) : null}

      <div className="grid items-start gap-6 lg:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
        <Block title="网络" detail="你加入的网络与角色。角色只在各自网络内有效。">
          <ul className="divide-y">
            {memberships.map((membership) => (
              <li key={membership.networkId} className="flex items-center gap-3 px-4 py-2.5 sm:px-5">
                <Link href={consoleRoutes(membership.networkId).overview} className="min-w-0 truncate text-[0.8125rem] font-medium underline-offset-4 hover:underline">
                  {membership.name ?? membership.networkId}
                </Link>
                {membership.networkId === currentNetwork ? <span className="text-xs text-muted-foreground">当前</span> : null}
                <StatusBadge tone={membership.role === "owner" ? "info" : "neutral"} className="ml-auto">{roleLabels[membership.role]}</StatusBadge>
              </li>
            ))}
          </ul>
        </Block>

        <Block
          title="我在本网络的设备"
          detail="用 Meshway App 或 CLI 登录本账户接入的设备。"
          action={<Link href={routes.members} className="text-xs text-muted-foreground underline-offset-4 hover:text-foreground hover:underline">所有成员</Link>}
        >
          {!devices.ok ? (
            <p className="px-4 py-3 text-xs text-destructive sm:px-5">{devices.error}</p>
          ) : devices.nodes.length === 0 ? (
            <p className="flex items-center gap-2 px-4 py-4 text-[0.8125rem] text-muted-foreground sm:px-5">
              <MonitorSmartphoneIcon className="size-4" aria-hidden="true" />
              还没有接入设备。
            </p>
          ) : (
            <ul className="divide-y">
              {devices.nodes.map((node) => (
                <li key={node.id} className="flex items-center gap-3 px-4 py-2.5 text-[0.8125rem] sm:px-5">
                  <Link href={routes.machine(node.id)} className={cn("min-w-0 truncate font-medium underline-offset-4 hover:underline", node.revoked_at !== null && "text-muted-foreground")}>
                    {node.name}
                  </Link>
                  <span className="font-mono text-xs text-muted-foreground">{node.mesh_ipv4}</span>
                  <span className="ml-auto text-xs text-muted-foreground">
                    {node.revoked_at !== null ? <StatusBadge tone="danger">已撤销</StatusBadge> : <LastContact node={node} fetchedAt={fetchedAt} />}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </Block>

        <Block title="修改密码" detail="修改后，这个账户在所有浏览器上的登录都会失效。">
          <PasswordForm email={email} />
        </Block>

        <Block title="登录会话" detail="会话最长 7 天，到期后需重新登录，不会自动续期。">
          <Sessions />
        </Block>
      </div>
    </>
  )
}

function Block({ title, detail, action, children }: { title: string; detail?: string; action?: ReactNode; children: ReactNode }) {
  return (
    <section className="overflow-hidden rounded-xl border bg-card shadow-xs">
      <header className="flex items-start justify-between gap-3 border-b px-4 py-3 sm:px-5">
        <div className="flex min-w-0 flex-col gap-0.5">
          <h2 className="text-sm font-semibold">{title}</h2>
          {detail ? <p className="text-xs text-muted-foreground">{detail}</p> : null}
        </div>
        {action}
      </header>
      {children}
    </section>
  )
}

function PasswordForm({ email }: { email: string }) {
  const signIn = useSignIn()
  const routes = useRoutes()
  const [state, action, pending] = useActionState<PasswordChangeState, FormData>(changePasswordAction.bind(null, signIn, routes.account), { status: "idle" })
  const ids = { current: useId(), next: useId(), confirmation: useId() }
  return (
    <form action={action} className="flex flex-col gap-3 px-4 py-4 sm:px-5" aria-label="修改密码">
      {/* Lets password managers attach the new password to the right account. */}
      <input type="email" name="username" autoComplete="username" value={email} readOnly hidden />
      <div className="flex flex-col gap-1.5">
        <Label htmlFor={ids.current}>当前密码</Label>
        <Input id={ids.current} name="current_password" type="password" required autoComplete="current-password" disabled={pending} className="h-9" />
      </div>
      <div className="grid gap-3 sm:grid-cols-2">
        <div className="flex flex-col gap-1.5">
          <Label htmlFor={ids.next}>新密码</Label>
          <Input id={ids.next} name="new_password" type="password" required autoComplete="new-password" disabled={pending} className="h-9" />
        </div>
        <div className="flex flex-col gap-1.5">
          <Label htmlFor={ids.confirmation}>确认新密码</Label>
          <Input id={ids.confirmation} name="new_password_confirmation" type="password" required autoComplete="new-password" disabled={pending} className="h-9" />
        </div>
      </div>
      <p className="text-xs text-muted-foreground">至少 12 个字节。</p>
      {state.status === "error" ? (
        <p role="alert" className="flex items-start gap-1.5 text-xs break-words text-destructive">
          <CircleAlertIcon className="mt-px size-3.5 shrink-0" />
          {state.message}
        </p>
      ) : null}
      <Button type="submit" disabled={pending} className="self-start">
        {pending ? "正在修改" : "修改密码"}
      </Button>
    </form>
  )
}

function Sessions() {
  const signIn = useSignIn()
  const vault = useDraftVault()
  const [pending, start] = useTransition()
  const [confirm, setConfirm] = useState(false)
  const [error, setError] = useState<string | null>(null)
  return (
    <div className="flex flex-col gap-3 px-4 py-4 sm:px-5">
      <div className="flex flex-wrap items-center gap-2">
        <Button
          variant="outline"
          disabled={pending}
          onClick={() => {
            vault.reset()
            start(() => signOutRendered(signIn))
          }}
        >
          <LogOutIcon data-icon="inline-start" />
          退出此浏览器
        </Button>
        <Button variant="ghost" disabled={pending} onClick={() => setConfirm(true)} className="text-destructive hover:text-destructive">
          退出所有会话…
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">控制面暂不提供会话列表，无法逐个查看或撤销其他浏览器的登录；「退出所有会话」会一次撤销全部。已接入的设备身份不受影响。</p>
      {error ? <p role="alert" className="text-xs text-destructive">{error}</p> : null}
      <AlertDialog open={confirm} onOpenChange={setConfirm}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>退出所有会话？</AlertDialogTitle>
            <AlertDialogDescription>这个账户在所有浏览器上的登录立即失效，包括这里，之后需要重新登录。未保存的草稿会丢失。</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={pending}>取消</AlertDialogCancel>
            <Button
              variant="destructive"
              disabled={pending}
              onClick={() =>
                start(async () => {
                  // Success navigates away and the sign-in's vault goes with it; a failure keeps the drafts.
                  const result = await revokeAllSessionsAction(signIn)
                  setError(result.message)
                  setConfirm(false)
                })
              }
            >
              {pending ? "正在退出" : "全部退出"}
            </Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
