import { Link, type HistoryState } from "@tanstack/react-router"
import { cn } from "cn"
import { GlobeIcon, ServerIcon, type LucideIcon } from "lucide-react"
import { motion, useReducedMotion } from "motion/react"
import { useId, useState, type ComponentProps, type ReactNode } from "react"

import { SharedLayoutBg } from "@/components/motion/shared-layout-bg"
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { SPRING_LAYOUT } from "@/lib/ease"
import { m } from "@/paraglide/messages.js"

import { CHROME } from "./glass-drawer"
import { DOCK_WIDTH } from "./layout"
import type { Page } from "./route"

/** The pages of a network, in dock order. Only pages that exist are listed. */
const PAGES: { page: Page; icon: LucideIcon; label: () => string }[] = [
  { page: "overview", icon: GlobeIcon, label: () => m.nav_overview() },
  { page: "nodes", icon: ServerIcon, label: () => m.nav_nodes() },
]

const ITEM = 36
const PADDING = (DOCK_WIDTH - ITEM) / 2

function PageLink({
  networkId,
  page,
  state,
  className,
  children,
  ...props
}: {
  networkId: string
  page: Page
  state?: HistoryState
  className: string
  children: ReactNode
} & Omit<ComponentProps<"a">, "href" | "children">) {
  const params = { network: networkId }
  return page === "nodes" ? (
    <Link
      to="/networks/$network/nodes"
      params={params}
      state={state}
      className={className}
      {...props}
    >
      {children}
    </Link>
  ) : (
    <Link
      to="/networks/$network/overview"
      params={params}
      state={state}
      className={className}
      {...props}
    >
      {children}
    </Link>
  )
}

/**
 * Desktop: an icon rail that is always there, on AgentUI's animated-sidebar
 * pattern. A soft pill follows the pointer from item to item
 * (`SharedLayoutBg`); the current page has its own pill, the same shape,
 * which glides when the page changes. Flat fills only, so the two can
 * overlap without a seam.
 */
export function Dock({ networkId, page }: { networkId: string; page: Page }) {
  const activeLayout = useId()
  const reduce = useReducedMotion()
  return (
    <nav
      {...CHROME}
      aria-label={m.nav_label()}
      className="enter glass pointer-events-auto absolute rounded-full"
      style={{
        left: "var(--edge)",
        top: "var(--panel-top)",
        width: DOCK_WIDTH,
        padding: PADDING,
      }}
    >
      <SharedLayoutBg
        as="ul"
        inset={0}
        pillClassName="rounded-full bg-fill-hover"
        pillContainerClassName="inset-y-auto top-0 h-9"
        className="gap-1"
      >
        {PAGES.map((item) => {
          const active = item.page === page
          return (
            <li key={item.page}>
              <Tooltip>
                <TooltipTrigger
                  render={
                    <PageLink
                      networkId={networkId}
                      page={item.page}
                      aria-label={item.label()}
                      aria-current={active ? "page" : undefined}
                      className={cn(
                        "relative flex size-9 items-center justify-center rounded-full transition-colors duration-200 outline-none focus-visible:ring-2 focus-visible:ring-ring",
                        active
                          ? "text-foreground"
                          : "text-muted-foreground hover:text-foreground"
                      )}
                    >
                      {active ? (
                        <motion.span
                          layoutId={activeLayout}
                          transition={reduce ? { duration: 0 } : SPRING_LAYOUT}
                          className="absolute inset-0 rounded-full bg-fill-selected"
                        />
                      ) : null}
                      <item.icon className="relative size-4.5" />
                    </PageLink>
                  }
                />
                <TooltipContent side="right" sideOffset={10}>
                  {item.label()}
                </TooltipContent>
              </Tooltip>
            </li>
          )
        })}
      </SharedLayoutBg>
    </nav>
  )
}

/**
 * Small screens: one round button showing the current page; it unfolds the
 * pages below it as labelled buttons, one after another.
 */
export function DockLauncher({
  networkId,
  page,
}: {
  networkId: string
  page: Page
}) {
  const [open, setOpen] = useState(false)
  const current = PAGES.find((item) => item.page === page) ?? PAGES[0]
  const Icon = current?.icon ?? GlobeIcon
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger
        {...CHROME}
        aria-label={m.nav_label()}
        className="enter glass pointer-events-auto absolute top-(--mobile-edge) left-(--mobile-edge) flex size-(--bar-height) items-center justify-center rounded-full outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
      >
        <Icon className="size-4" />
      </PopoverTrigger>
      <PopoverContent
        side="bottom"
        align="start"
        sideOffset={8}
        className="launcher w-auto gap-2 bg-transparent p-0 shadow-none ring-0"
        render={<nav aria-label={m.nav_label()} />}
      >
        {PAGES.map((item, index) => (
          <PageLink
            key={item.page}
            networkId={networkId}
            page={item.page}
            aria-current={item.page === page ? "page" : undefined}
            // A page other than the overview opens as a sheet here: dismissing it goes back.
            state={item.page === "overview" ? undefined : { opened: true }}
            onClick={() => setOpen(false)}
            className={cn(
              "launcher-item glass flex h-(--bar-height) items-center gap-3 self-start rounded-full pr-5 pl-3.5 text-sm font-medium outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring",
              item.page === page ? "text-foreground" : "text-muted-foreground"
            )}
            style={{ "--launcher-index": index }}
          >
            <item.icon className="size-4" />
            {item.label()}
          </PageLink>
        ))}
      </PopoverContent>
    </Popover>
  )
}
