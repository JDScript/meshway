"use client"

import type { AccountRole } from "@meshway/control-client"
import { ArrowUpRightIcon, CrownIcon, PlusIcon } from "lucide-react"
import Link from "next/link"
import { useActionState, useState } from "react"

import { OwnerClaimForm } from "@/components/auth/owner-claim-form"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import type { SignInKey } from "@/lib/action-scope"
import { roleLabels } from "@/lib/account-model"
import { createNetworkAction, type CreateNetworkState } from "@/lib/network-actions"
import { consoleRoutes } from "@/lib/routes"

export type SetupNetwork = {
  networkId: string
  name: string
  prefix: string
  members: { ok: true; list: { userId: string; email: string; role: AccountRole }[] } | { ok: false; error: string }
}

export function SetupNetworks({ signIn, networks }: { signIn: SignInKey; networks: SetupNetwork[] }) {
  const [creating, setCreating] = useState(networks.length === 0)
  return (
    <section aria-labelledby="setup-networks" className="flex flex-col gap-3">
      <div className="flex items-end justify-between gap-2">
        <h2 id="setup-networks" className="text-sm font-semibold">网络</h2>
        {!creating ? (
          <Button variant="outline" size="sm" onClick={() => setCreating(true)}>
            <PlusIcon data-icon="inline-start" />
            创建网络
          </Button>
        ) : null}
      </div>
      {creating ? <CreateNetwork signIn={signIn} onDone={() => setCreating(false)} cancellable={networks.length > 0} /> : null}
      <ul className="flex flex-col divide-y rounded-lg border">
        {networks.length === 0 ? <li className="px-4 py-6 text-center text-[0.8125rem] text-muted-foreground">还没有网络。</li> : null}
        {networks.map((network) => (
          <NetworkRow key={network.networkId} signIn={signIn} network={network} />
        ))}
      </ul>
    </section>
  )
}

function NetworkRow({ signIn, network }: { signIn: SignInKey; network: SetupNetwork }) {
  const owners = network.members.ok ? network.members.list.filter((member) => member.role === "owner") : []
  const needsOwner = network.members.ok && owners.length === 0
  const [claiming, setClaiming] = useState(false)
  return (
    <li className="flex flex-col gap-3 px-4 py-3.5">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
        <span className="min-w-0 truncate text-sm font-medium">{network.name}</span>
        <span className="font-mono text-xs text-muted-foreground">{network.prefix}</span>
        <span className="ml-auto flex items-center gap-1">
          {needsOwner && !claiming ? (
            <Button size="sm" onClick={() => setClaiming(true)}>
              <CrownIcon data-icon="inline-start" />
              指定所有者
            </Button>
          ) : null}
          <Link
            href={consoleRoutes(network.networkId).overview}
            className="inline-flex h-7 items-center gap-1 rounded-md px-2 text-xs text-muted-foreground hover:bg-muted hover:text-foreground"
            title="以实例管理员令牌打开（兼容入口）"
          >
            打开
            <ArrowUpRightIcon className="size-3.5" aria-hidden="true" />
          </Link>
        </span>
      </div>
      {!network.members.ok ? (
        <p role="alert" className="text-xs break-words text-destructive">无法读取成员：{network.members.error}</p>
      ) : (
        <p className="text-xs text-muted-foreground">
          {owners.length > 0 ? (
            <>
              {roleLabels.owner} <span className="break-all text-foreground">{owners.map((owner) => owner.email).join("、")}</span>
              {" · "}
            </>
          ) : (
            <span className="text-warning">没有所有者 · </span>
          )}
          {network.members.list.length} 位成员
        </p>
      )}
      {needsOwner && claiming ? (
        <div className="rounded-md border bg-muted/30 p-3.5">
          <OwnerClaimForm signIn={signIn} networkId={network.networkId} networkName={network.name} />
        </div>
      ) : null}
    </li>
  )
}

function CreateNetwork({ signIn, onDone, cancellable }: { signIn: SignInKey; onDone: () => void; cancellable: boolean }) {
  const [state, action, pending] = useActionState<CreateNetworkState, FormData>(createNetworkAction.bind(null, signIn), { error: null })
  if (state.networkId && !pending) {
    // The page re-renders with the new network listed (the action refreshes).
    return (
      <p role="status" className="rounded-md border bg-muted/30 px-3 py-2 text-[0.8125rem]">
        网络已创建。在下方为它指定第一个所有者。
        <Button variant="link" size="sm" className="ml-1 h-auto p-0" onClick={onDone}>
          关闭
        </Button>
      </p>
    )
  }
  return (
    <form action={action} className="flex flex-col gap-2 rounded-md border bg-muted/30 p-3.5">
      <Label htmlFor="setup-network-name">网络名称</Label>
      <div className="flex gap-2">
        <Input id="setup-network-name" name="name" required maxLength={128} autoComplete="off" disabled={pending} className="h-9 flex-1" />
        <Button type="submit" disabled={pending} className="h-9">
          {pending ? "正在创建" : "创建"}
        </Button>
        {cancellable ? (
          <Button type="button" variant="ghost" className="h-9" onClick={onDone} disabled={pending}>
            取消
          </Button>
        ) : null}
      </div>
      <p className="text-xs text-muted-foreground">新网络有独立的地址池、成员、DNS 与策略。</p>
      {state.error ? (
        <p role="alert" className="text-xs break-all text-destructive">
          {state.error}
        </p>
      ) : null}
    </form>
  )
}
