import { Link, type HistoryState } from "@tanstack/react-router"
import { GlobeIcon } from "lucide-react"
import { motion, useReducedMotion } from "motion/react"
import { useId, useState, type ComponentProps, type ReactNode } from "react"

import { DraftMark } from "@/components/chrome/draft-mark"
import { SharedLayoutBg } from "@/components/chrome/shared-layout-bg"
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
import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"

import { useDraftPages } from "./drafts"
import { CHROME } from "./glass-drawer"
import { useSurfaceHistory } from "./history"
import { DOCK_WIDTH } from "./layout"
import { DOCK, PAGES, type PageId } from "./pages"

const ITEM = 36
const PADDING = (DOCK_WIDTH - ITEM) / 2

function PageLink({
  networkId,
  page,
  state,
  replace,
  className,
  children,
  ...props
}: {
  networkId: string
  page: PageId
  state?: HistoryState
  replace?: boolean
  className: string
  children: ReactNode
} & Omit<ComponentProps<"a">, "href" | "children">) {
  return (
    <Link
      to={PAGES[page].route}
      params={{ network: networkId }}
      state={state}
      replace={replace}
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
export function Dock({ networkId, page }: { networkId: string; page: PageId }) {
  const activeLayout = useId()
  const reduce = useReducedMotion()
  const drafts = useDraftPages(networkId)
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
        // Pointing at the current page adds nothing: its own pill already marks it.
        pillClassName="rounded-full bg-fill-hover group-data-[current=true]/dock-item:invisible"
        pillContainerClassName="inset-y-auto top-0 h-9"
        className="gap-1"
      >
        {DOCK.map((item) => {
          const active = item.page === page
          return (
            <li
              key={item.page}
              data-current={active}
              className="group/dock-item"
            >
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
                      {drafts[item.page] ? (
                        <span className="absolute top-1.5 right-1.5 flex">
                          <DraftMark />
                        </span>
                      ) : null}
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
 * Small screens: one round button at the start of the bottom bar, under
 * the thumb, showing the current page; it unfolds the pages above it as
 * labelled buttons, the nearest first.
 */
export function DockLauncher({
  networkId,
  page,
}: {
  networkId: string
  page: PageId
}) {
  const [open, setOpen] = useState(false)
  const drafts = useDraftPages(networkId)
  const history = useSurfaceHistory(networkId)
  const current = DOCK.find((item) => item.page === page)
  const Icon = current?.icon ?? GlobeIcon
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger
        {...CHROME}
        aria-label={m.nav_label()}
        className="enter glass pointer-events-auto flex size-(--bar-height) shrink-0 items-center justify-center rounded-full outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
      >
        <Icon className="size-4" />
      </PopoverTrigger>
      <PopoverContent
        side="top"
        align="start"
        sideOffset={8}
        className="launcher w-auto gap-2 bg-transparent p-0 shadow-none ring-0"
        render={<nav aria-label={m.nav_label()} />}
      >
        {DOCK.map((item, index) => (
          <PageLink
            key={item.page}
            networkId={networkId}
            page={item.page}
            aria-current={item.page === page ? "page" : undefined}
            // Pages open as sheets over the overview: the first one remembers it, the next ones replace.
            {...(item.page === "overview" || item.page === page
              ? {}
              : history.toPage(page === "overview"))}
            onClick={(event) => {
              setOpen(false)
              // The overview from a page is leaving the pages, as a swipe would.
              if (item.page === "overview" && page !== "overview") {
                event.preventDefault()
                history.leavePage()
              }
            }}
            className={cn(
              "launcher-item glass flex h-(--bar-height) items-center gap-3 self-start rounded-full pr-5 pl-3.5 text-row font-medium outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring",
              item.page === page ? "text-foreground" : "text-muted-foreground"
            )}
            style={{ "--launcher-index": DOCK.length - 1 - index }}
          >
            <item.icon className="size-4" />
            {item.label()}
            {drafts[item.page] ? <DraftMark /> : null}
          </PageLink>
        ))}
      </PopoverContent>
    </Popover>
  )
}
