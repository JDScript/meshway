"use client"

import { CheckIcon, ChevronsUpDownIcon, CircleAlertIcon, PlusIcon, SettingsIcon } from "lucide-react"
import Link from "next/link"
import { usePathname, useRouter } from "next/navigation"
import { useActionState, useEffect, useState, useTransition } from "react"

import { useDraftVault } from "@/components/network/network-workspace"
import { useNetwork } from "@/components/shell/network-context"
import { sectionSegment } from "@/components/shell/nav"
import { Button } from "@/components/ui/button"
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { createNetworkAction, switchNetworkAction, type CreateNetworkState } from "@/lib/network-actions"
import { consoleRoutes } from "@/lib/routes"

/** Serializable network list for the shell. */
export type NetworkChoices =
  | { ok: true; networks: { networkId: string; name: string; ipv4Prefix: string }[] }
  | { ok: false; error: string }

/** Same section in another network; detail pages fall back to their list. */
function sameSection(pathname: string, fromBase: string, networkId: string): string {
  const segment = sectionSegment(pathname, fromBase)
  const base = consoleRoutes(networkId).base
  return segment ? `${base}/${segment}` : base
}

/**
 * The network context, like choosing a tailnet. Switching navigates to the
 * other network's URL; pages, drafts and actions follow the URL, while the
 * selection cookie only remembers where `/` should land next time.
 */
export function NetworkSwitcher({ networks, controllerHost }: { networks: NetworkChoices; controllerHost: string }) {
  const { networkId, name, routes } = useNetwork()
  const vault = useDraftVault()
  const router = useRouter()
  const pathname = usePathname()
  const [pending, startTransition] = useTransition()
  const [error, setError] = useState<string | null>(null)
  const [creating, setCreating] = useState(false)
  const label = name ?? "网络"

  const open = (target: string) => {
    if (target === networkId) return
    setError(null)
    startTransition(async () => {
      const result = await switchNetworkAction(target)
      if (result.error) setError(result.error)
      else router.push(sameSection(pathname, routes.base, target))
    })
  }

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger
          aria-label="当前网络"
          className="flex h-8 min-w-0 items-center gap-2 rounded-md px-2 text-left text-[0.8125rem] outline-none hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring/40 aria-expanded:bg-accent"
        >
          <span
            aria-hidden="true"
            className="flex size-5 shrink-0 items-center justify-center rounded bg-brand/15 text-[0.6875rem] font-semibold text-brand"
          >
            {label.slice(0, 1).toUpperCase()}
          </span>
          <span className="max-w-36 truncate font-medium sm:max-w-56">{label}</span>
          {!networks.ok || error ? <CircleAlertIcon className="size-3.5 text-destructive" aria-hidden="true" /> : null}
          <ChevronsUpDownIcon className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" sideOffset={6} className="w-72">
          <DropdownMenuGroup>
            <DropdownMenuLabel>网络</DropdownMenuLabel>
            {networks.ok ? (
              networks.networks.map((network) => {
                const current = network.networkId === networkId
                const unsaved = vault.hasDrafts(network.networkId)
                return (
                  <DropdownMenuItem
                    key={network.networkId}
                    disabled={pending}
                    aria-label={network.name}
                    onClick={() => open(network.networkId)}
                    className="items-start"
                  >
                    <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                      <span className="flex items-center gap-1.5 truncate font-medium">
                        {network.name}
                        {unsaved ? <span className="size-1.5 rounded-full bg-warning" title="有未保存的草稿" /> : null}
                      </span>
                      <span className="truncate font-mono text-[0.6875rem] text-muted-foreground">{network.ipv4Prefix}</span>
                    </span>
                    {current ? <CheckIcon className="mt-0.5 size-4 text-brand" aria-label="当前网络" /> : null}
                  </DropdownMenuItem>
                )
              })
            ) : (
              <p role="alert" className="px-2 py-1.5 text-xs break-all text-destructive">
                网络列表读取失败：{networks.error}
              </p>
            )}
          </DropdownMenuGroup>
          {error ? (
            <p role="alert" className="px-2 py-1.5 text-xs break-all text-destructive">
              {error}
            </p>
          ) : null}
          <DropdownMenuSeparator />
          <DropdownMenuItem onClick={() => setCreating(true)}>
            <PlusIcon />
            创建网络
          </DropdownMenuItem>
          <DropdownMenuItem render={<Link href={routes.settings} />}>
            <SettingsIcon />
            网络设置
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <p className="px-2 py-1.5 text-[0.6875rem] leading-relaxed text-muted-foreground">
            Controller <span className="font-mono text-foreground/80">{controllerHost}</span>。网络之间的成员、地址、DNS 与策略相互隔离。
          </p>
        </DropdownMenuContent>
      </DropdownMenu>
      <Dialog open={creating} onOpenChange={setCreating}>
        <DialogContent className="sm:max-w-md">
          {creating ? <CreateNetworkForm onCancel={() => setCreating(false)} /> : null}
        </DialogContent>
      </Dialog>
    </>
  )
}

function CreateNetworkForm({ onCancel }: { onCancel: () => void }) {
  const [state, action, pending] = useActionState<CreateNetworkState, FormData>(createNetworkAction, { error: null })
  const router = useRouter()
  useEffect(() => {
    if (state.networkId) router.push(consoleRoutes(state.networkId).overview)
  }, [state.networkId, router])
  return (
    <form action={action} className="flex flex-col gap-4">
      <DialogHeader>
        <DialogTitle>创建网络</DialogTitle>
        <DialogDescription>
          新网络从空成员清单开始，拥有独立的地址池、DNS、标签与策略。节点需要用这张网络的接入令牌加入。
        </DialogDescription>
      </DialogHeader>
      <div className="flex flex-col gap-1.5">
        <Label htmlFor="network-name">网络名称</Label>
        <Input id="network-name" name="name" required maxLength={128} autoFocus autoComplete="off" />
      </div>
      {state.error ? (
        <p role="alert" className="text-xs break-all text-destructive">
          {state.error}
        </p>
      ) : null}
      <DialogFooter>
        <Button type="button" variant="outline" onClick={onCancel}>
          取消
        </Button>
        <Button type="submit" disabled={pending || Boolean(state.networkId)}>
          {pending ? "创建中…" : "创建网络"}
        </Button>
      </DialogFooter>
    </form>
  )
}
