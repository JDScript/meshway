"use client"

import { PlusIcon, SearchIcon, SunMoonIcon } from "lucide-react"
import { useTheme } from "next-themes"
import { useRouter } from "next/navigation"
import { useEffect, useState } from "react"

import { navGroups, settingsItem } from "@/components/shell/nav"
import { useRoutes } from "@/components/shell/network-context"
import { Button } from "@/components/ui/button"
import {
  Command,
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
} from "@/components/ui/command"
import { Kbd } from "@/components/ui/kbd"

function otherModalOpen(): boolean {
  return document.querySelector('[role="dialog"][data-open], [role="alertdialog"][data-open]') !== null
}

/**
 * Navigation and a few real actions within the current network. Node search
 * hands the query to the nodes page, which filters the live inventory.
 */
export function CommandMenu() {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState("")
  const router = useRouter()
  const routes = useRoutes()
  const { resolvedTheme, setTheme } = useTheme()

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key.toLowerCase() !== "k" || !(event.metaKey || event.ctrlKey)) return
      event.preventDefault()
      setOpen((current) => {
        if (current) return false
        // Another modal (e.g. a rule sheet with unapplied edits) owns the
        // page and must be closed through its own flow.
        return !otherModalOpen()
      })
    }
    window.addEventListener("keydown", onKeyDown)
    return () => window.removeEventListener("keydown", onKeyDown)
  }, [])

  const run = (action: () => void) => {
    setOpen(false)
    setQuery("")
    action()
  }
  const trimmed = query.trim()

  return (
    <>
      <Button
        variant="outline"
        onClick={() => setOpen(true)}
        className="w-8 justify-center px-0 font-normal text-muted-foreground shadow-none sm:w-56 sm:justify-start sm:px-2.5"
        aria-label="搜索与跳转"
      >
        <SearchIcon data-icon="inline-start" />
        <span className="hidden flex-1 text-left sm:inline">搜索或跳转…</span>
        <Kbd className="hidden sm:inline-flex">⌘K</Kbd>
      </Button>
      <CommandDialog
        open={open}
        onOpenChange={(next) => {
          setOpen(next)
          if (!next) setQuery("")
        }}
        title="搜索与跳转"
        description="跳转到页面、搜索节点或执行操作"
      >
        <Command>
          <CommandInput placeholder="输入页面、节点名称、IP 或操作…" value={query} onValueChange={setQuery} />
          <CommandList>
            <CommandEmpty>没有匹配的页面或操作。</CommandEmpty>
            {trimmed ? (
              <CommandGroup heading="节点">
                <CommandItem
                  value={`search-nodes ${trimmed}`}
                  onSelect={() => run(() => router.push(routes.machines({ q: trimmed })))}
                >
                  <SearchIcon />
                  在节点中搜索「{trimmed}」
                </CommandItem>
              </CommandGroup>
            ) : null}
            <CommandGroup heading="页面">
              {navGroups.flatMap((group) =>
                group.items.map((item) => (
                  <CommandItem
                    key={item.segment}
                    value={`${group.label ?? ""} ${item.label} ${item.keywords.join(" ")}`}
                    onSelect={() => run(() => router.push(item.href(routes)))}
                  >
                    <item.icon />
                    {item.label}
                    {group.label ? <span className="ml-auto text-xs text-muted-foreground">{group.label}</span> : null}
                  </CommandItem>
                ))
              )}
              <CommandItem
                value={`${settingsItem.label} ${settingsItem.keywords.join(" ")}`}
                onSelect={() => run(() => router.push(settingsItem.href(routes)))}
              >
                <settingsItem.icon />
                {settingsItem.label}
              </CommandItem>
            </CommandGroup>
            <CommandSeparator />
            <CommandGroup heading="操作">
              <CommandItem
                value="add node enrollment token 添加节点 注册令牌"
                onSelect={() => run(() => router.push(routes.addMachine))}
              >
                <PlusIcon />
                添加节点
              </CommandItem>
              <CommandItem
                value="toggle theme 切换主题 深色 浅色"
                onSelect={() => run(() => setTheme(resolvedTheme === "dark" ? "light" : "dark"))}
              >
                <SunMoonIcon />
                切换浅色 / 深色
              </CommandItem>
            </CommandGroup>
          </CommandList>
        </Command>
      </CommandDialog>
    </>
  )
}
