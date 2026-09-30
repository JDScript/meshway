import type { ReactNode } from "react"

import { Brand } from "@/components/brand"
import { LogoutButton } from "@/components/logout-button"
import { NavLinks } from "@/components/nav-links"
import { controllerUrl } from "@/lib/controller"

// Authentication is enforced by every page and Server Action, not here:
// layouts are not re-rendered on every navigation.
export default function ConsoleLayout({ children }: { children: ReactNode }) {
  return (
    <div className="flex min-h-svh flex-col md:flex-row">
      <aside className="hidden w-60 shrink-0 flex-col border-r bg-sidebar text-sidebar-foreground md:sticky md:top-0 md:flex md:h-svh">
        <div className="px-5 pt-6 pb-5">
          <Brand />
        </div>
        <div className="flex-1 px-3">
          <NavLinks orientation="vertical" />
        </div>
        <div className="flex flex-col gap-3 border-t px-3 py-4">
          <div className="flex flex-col gap-1 px-2">
            <span className="text-xs text-muted-foreground">Controller</span>
            <code className="font-mono text-xs break-all">{controllerUrl()}</code>
          </div>
          <LogoutButton />
        </div>
      </aside>
      <header className="flex flex-col gap-3 border-b px-4 pt-4 pb-2 md:hidden">
        <div className="flex items-center justify-between gap-3">
          <Brand />
          <div className="w-auto">
            <LogoutButton />
          </div>
        </div>
        <div className="-mx-1 overflow-x-auto">
          <NavLinks orientation="horizontal" />
        </div>
      </header>
      <main className="min-w-0 flex-1">
        <div className="mx-auto flex w-full max-w-5xl flex-col gap-8 px-4 py-8 sm:px-8 md:py-12">
          {children}
        </div>
      </main>
    </div>
  )
}
