"use client"

import { ChevronsUpDownIcon, NetworkIcon } from "lucide-react"
import type { NetworkList, NetworkSnapshot } from "@meshway/control-client"
import Link from "next/link"
import { useActionState, useState, useTransition } from "react"
import { createNetworkAction, switchNetworkAction } from "@/lib/network-actions"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog"

import { BrandMark } from "@/components/shell/brand"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuLabel,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import {
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  useSidebar,
} from "@/components/ui/sidebar"
import type { ControllerResult } from "@/lib/controller"

export function ContextSwitcher({ controllerHost, network, networks }: {
  controllerHost: string
  /** Supplied once by the server shell; this component never duplicates the request. */
  network?: ControllerResult<NetworkSnapshot>
  networks: ControllerResult<NetworkList>
}) {
  const { isMobile } = useSidebar()
  const [creating, setCreating] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [pending, start] = useTransition()

  return (
    <SidebarMenu>
      <SidebarMenuItem>
        <DropdownMenu>
          <DropdownMenuTrigger
            render={
              <SidebarMenuButton
                size="lg"
                aria-label="当前网络"
                className="data-popup-open:bg-sidebar-accent"
              />
            }
          >
            <BrandMark className="size-8" />
            <div className="grid min-w-0 flex-1 text-left leading-tight">
              <span className="truncate text-sm font-semibold tracking-tight">{network?.ok ? network.data.name : "Meshway"}</span>
              <span className="truncate font-mono text-[0.6875rem] text-muted-foreground">
                {controllerHost}
              </span>
            </div>
            <ChevronsUpDownIcon className="ml-auto text-muted-foreground" />
          </DropdownMenuTrigger>
          <DropdownMenuContent
            className="w-(--anchor-width) min-w-64"
            side={isMobile ? "bottom" : "right"}
            align="start"
            sideOffset={6}
          >
            <DropdownMenuGroup>
              <DropdownMenuLabel>当前网络</DropdownMenuLabel>
              {networks.ok ? networks.data.networks.map(item => <DropdownMenuItem
                key={item.network_id} disabled={pending}
                onClick={() => start(async () => { const result = await switchNetworkAction(item.network_id); setError(result.error) })}
              >{item.name}{network?.ok && network.data.network_id === item.network_id ? " ✓" : ""}</DropdownMenuItem>) : <p className="px-2 text-xs text-destructive">{networks.error}</p>}
              <DropdownMenuItem onClick={() => setCreating(true)}>创建网络</DropdownMenuItem>
              <div className="flex items-start gap-2 px-2 py-1.5 text-sm">
                <NetworkIcon className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
                <span className="min-w-0 break-all font-mono text-xs">{controllerHost}</span>
              </div>
              {network?.ok ? (
                <div className="flex flex-col gap-1 px-2 py-1.5 text-xs">
                  <span className="break-all font-medium">{network.data.name}</span>
                  <code className="break-all font-mono text-muted-foreground">{network.data.network_id}</code>
                </div>
              ) : (
                <p className="break-all px-2 py-1.5 text-xs text-muted-foreground">{network ? `网络信息读取失败：${network.error}` : "网络信息未读取"}</p>
              )}
              <Link href="/network" className="mx-2 inline-flex py-1.5 text-xs underline underline-offset-4">网络设置</Link>
              {error ? <p role="alert" className="px-2 text-xs text-destructive">{error}</p> : null}
            </DropdownMenuGroup>
          </DropdownMenuContent>
        </DropdownMenu>
        <Dialog open={creating} onOpenChange={setCreating}>
          <DialogContent><DialogHeader><DialogTitle>创建网络</DialogTitle></DialogHeader>
            <CreateNetworkForm />
          </DialogContent>
        </Dialog>
      </SidebarMenuItem>
    </SidebarMenu>
  )
}

function CreateNetworkForm() {
  const [state, action, pending] = useActionState(createNetworkAction, { error: null })
  return <form action={action} className="flex flex-col gap-4">
    <label className="flex flex-col gap-2 text-sm">网络名称<Input name="name" required maxLength={128} autoFocus /></label>
    <p className="text-sm text-muted-foreground">新网络从空成员清单开始，地址、DNS、标签和策略独立管理。</p>
    {state.error ? <p role="alert" className="text-sm text-destructive">{state.error}</p> : null}
    <Button disabled={pending} type="submit">{pending ? "创建中…" : "创建网络"}</Button>
  </form>
}
