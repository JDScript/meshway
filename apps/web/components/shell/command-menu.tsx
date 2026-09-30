"use client"

import { PlusIcon, SearchIcon, SunMoonIcon } from "lucide-react"
import { useTheme } from "next-themes"
import { useRouter } from "next/navigation"
import { useEffect, useState } from "react"

import { navItems } from "@/components/shell/nav"
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
  return (
    document.querySelector(
      '[role="dialog"][data-open], [role="alertdialog"][data-open]'
    ) !== null
  )
}

/**
 * Navigation and a few real actions. Node search hands the query to the
 * nodes page, which filters the live inventory; the menu itself holds no data.
 */
export function CommandMenu() {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState("")
  const router = useRouter()
  const { resolvedTheme, setTheme } = useTheme()

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key.toLowerCase() !== "k" || !(event.metaKey || event.ctrlKey)) return
      event.preventDefault()
      setOpen((current) => {
        if (current) return false
        // Another modal (e.g. a rule sheet with unapplied edits) owns the
        // page: it must be closed through its own flow, which confirms
        // discarding. Global navigation never escapes it.
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
        size="sm"
        onClick={() => setOpen(true)}
        className="w-9 justify-center px-0 font-normal text-muted-foreground sm:w-56 sm:justify-start sm:px-2"
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
          <CommandInput
            placeholder="输入页面、节点名称、IP 或操作…"
            value={query}
            onValueChange={setQuery}
          />
          <CommandList>
            <CommandEmpty>没有匹配的页面或操作。</CommandEmpty>
            {trimmed ? (
              <CommandGroup heading="节点">
                <CommandItem
                  value={`search-nodes ${trimmed}`}
                  onSelect={() =>
                    run(() => router.push(`/nodes?q=${encodeURIComponent(trimmed)}`))
                  }
                >
                  <SearchIcon />
                  在节点中搜索「{trimmed}」
                </CommandItem>
              </CommandGroup>
            ) : null}
            <CommandGroup heading="页面">
              {navItems.map((item) => (
                <CommandItem
                  key={item.href}
                  value={`${item.label} ${item.keywords.join(" ")}`}
                  onSelect={() => run(() => router.push(item.href))}
                >
                  <item.icon />
                  {item.label}
                </CommandItem>
              ))}
            </CommandGroup>
            <CommandSeparator />
            <CommandGroup heading="操作">
              <CommandItem
                value="create enrollment token 创建注册令牌 新节点"
                onSelect={() => run(() => router.push("/enrollment"))}
              >
                <PlusIcon />
                创建注册令牌
              </CommandItem>
              <CommandItem
                value="toggle theme 切换主题 深色 浅色"
                onSelect={() =>
                  run(() => setTheme(resolvedTheme === "dark" ? "light" : "dark"))
                }
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
