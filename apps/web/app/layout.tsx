import type { Metadata } from "next"
import { Geist_Mono, Inter } from "next/font/google"
import type { ReactNode } from "react"

import "./globals.css"
import { NavLinks } from "@/components/nav-links"
import { ThemeProvider } from "@/components/theme-provider"
import { controllerUrl } from "@/lib/controller"
import { cn } from "@/lib/utils"

const fontSans = Inter({ subsets: ["latin"], variable: "--font-sans" })
const fontMono = Geist_Mono({ subsets: ["latin"], variable: "--font-mono" })

export const metadata: Metadata = {
  title: { default: "Meshway 控制台", template: "%s · Meshway" },
  description: "Meshway controller 的本地开发控制台",
}

function Brand() {
  return (
    <div className="flex items-baseline gap-2">
      <span className="font-heading text-base font-semibold tracking-tight">
        Meshway
      </span>
      <span className="text-xs text-muted-foreground">控制台</span>
    </div>
  )
}

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html
      lang="zh-CN"
      suppressHydrationWarning
      className={cn("antialiased", fontSans.variable, fontMono.variable)}
    >
      <body className="font-sans">
        <ThemeProvider>
          <div className="flex min-h-svh flex-col md:flex-row">
            <aside className="hidden w-60 shrink-0 flex-col border-r bg-sidebar text-sidebar-foreground md:sticky md:top-0 md:flex md:h-svh">
              <div className="px-5 pt-6 pb-5">
                <Brand />
              </div>
              <div className="flex-1 px-3">
                <NavLinks orientation="vertical" />
              </div>
              <div className="flex flex-col gap-1 border-t px-5 py-4">
                <span className="text-xs text-muted-foreground">Controller</span>
                <code className="font-mono text-xs break-all">
                  {controllerUrl()}
                </code>
              </div>
            </aside>
            <header className="flex flex-col gap-3 border-b px-4 pt-4 pb-2 md:hidden">
              <Brand />
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
        </ThemeProvider>
      </body>
    </html>
  )
}
