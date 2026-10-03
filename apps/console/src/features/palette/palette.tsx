import { Dialog as DialogPrimitive } from "@base-ui/react/dialog"
import { Command, useCommandState } from "cmdk"
import { SearchIcon } from "lucide-react"
import { motion, useReducedMotion } from "motion/react"
import {
  createContext,
  useContext,
  useEffect,
  useId,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react"
import { create } from "zustand"

import { Button } from "@/components/ui/button"
import { Kbd } from "@/components/ui/kbd"
import { Skeleton } from "@/components/ui/skeleton"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { InviteDialog } from "@/features/members/invite-dialog"
import { grantableRoles } from "@/features/members/roles"
import { useNetworkRole } from "@/features/network/role"
import {
  EDGE,
  MOBILE_EDGE,
  MOBILE_PANEL_TOP,
  MOBILE_SHEET_BOTTOM,
  PANEL_TOP,
} from "@/features/shell/layout"
import type { PageId } from "@/features/shell/pages"
import { useIsMobile } from "@/hooks/use-mobile"
import { SPRING_LAYOUT } from "@/lib/ease"
import { useViewportHeight } from "@/lib/use-viewport-height"
import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"

import { usePaletteGroups, type Entry } from "./groups"
import { bestMatches } from "./match"

// The command palette (⌘K): search and jump across the network one is in
// (`groups.tsx` says what it offers).

const usePalette = create<{
  open: boolean
  search: string
  /** Closed by a jump: focus goes where it lands, not back. */
  handOff: boolean
}>(() => ({ open: false, search: "", handOff: false }))
const openPalette = () =>
  usePalette.setState({ open: true, search: "", handOff: false })
const closePalette = (handOff = false) =>
  usePalette.setState({ open: false, handOff })
const setSearch = (search: string) => usePalette.setState({ search })

/** How many results a group shows; the rest are counted. */
const GROUP_LIMIT = 6
const PANEL_WIDTH = 560

const MAC = /Mac|iPhone|iPad/.test(navigator.userAgent)
const SHORTCUT = MAC ? "⌘K" : "Ctrl K"

/** ⌘K (Ctrl+K elsewhere) opens and closes the palette while `enabled`. */
function useShortcut(enabled: boolean) {
  useEffect(() => {
    if (!enabled) return
    const onKey = (event: KeyboardEvent) => {
      if (event.key.toLowerCase() !== "k" || event.isComposing) return
      if (!(MAC ? event.metaKey : event.ctrlKey) || event.altKey) return
      event.preventDefault()
      if (usePalette.getState().open) closePalette()
      else openPalette()
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [enabled])
}

/** The search button in the top bar, before the language switch. */
export function PaletteButton() {
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            variant="ghost"
            size="icon"
            className="bar-button"
            aria-label={m.palette_open()}
            aria-keyshortcuts={MAC ? "Meta+K" : "Control+K"}
            onClick={openPalette}
          >
            <SearchIcon />
          </Button>
        }
      />
      <TooltipContent>
        {m.palette_open()}
        <Kbd>{SHORTCUT}</Kbd>
      </TooltipContent>
    </Tooltip>
  )
}

/** The visible viewport's height: on phones the keyboard takes the rest. */
function useVisibleHeight(): number {
  return useSyncExternalStore(
    (onChange) => {
      const viewport = window.visualViewport
      viewport?.addEventListener("resize", onChange)
      window.addEventListener("resize", onChange)
      return () => {
        viewport?.removeEventListener("resize", onChange)
        window.removeEventListener("resize", onChange)
      }
    },
    () => window.visualViewport?.height ?? window.innerHeight
  )
}

/**
 * The palette itself, mounted once in a network's shell. ⌘K works only
 * there (not on the entry pages) and once the first load is done.
 */
export function CommandPalette({
  networkId,
  page,
  ready,
  onSwitchNetwork,
}: {
  networkId: string
  page: PageId
  ready: boolean
  onSwitchNetwork: (networkId: string) => void
}) {
  useShortcut(ready)
  const open = usePalette((state) => state.open) && ready
  const mobile = useIsMobile()
  const visible = useVisibleHeight()
  const layout = useViewportHeight()
  const search = usePalette((state) => state.search)
  const [inviting, setInviting] = useState(false)
  const role = useNetworkRole(networkId)
  const grantable = grantableRoles(role)

  const groups = usePaletteGroups({
    networkId,
    page,
    mobile,
    open,
    onSwitchNetwork,
    onInvite: grantable.length > 0 ? () => setInviting(true) : null,
  })

  const shown = groups.map((group) => {
    const { shown: entries, more } =
      group.status === "ready"
        ? bestMatches(
            search.trim() === ""
              ? group.entries.filter((entry) => !entry.searchOnly)
              : group.entries,
            search,
            group.bounded ? GROUP_LIMIT : Infinity
          )
        : { shown: [], more: 0 }
    return { ...group, entries, more }
  })
  const visibleGroups = shown.filter(
    (group) => group.status !== "ready" || group.entries.length > 0
  )
  const matched = shown.some((group) => group.entries.length > 0)
  const reading = shown.some((group) => group.status === "loading")

  const choose = (entry: Entry) => {
    closePalette(entry.navigates)
    entry.run()
  }

  const top = mobile ? MOBILE_PANEL_TOP : `max(${PANEL_TOP}px, 14vh)`
  const layoutId = useId()

  return (
    <>
      <DialogPrimitive.Root
        open={open}
        onOpenChange={(next) => (next ? openPalette() : closePalette())}
      >
        <DialogPrimitive.Portal>
          <DialogPrimitive.Backdrop className="fixed inset-0 isolate z-50 bg-black/10 duration-100 supports-backdrop-filter:backdrop-blur-xs data-open:animate-in data-open:fade-in-0 data-closed:animate-out data-closed:fade-out-0" />
          <DialogPrimitive.Popup
            // A jump hands focus to where it lands (a rule focuses itself); Escape and a click outside return it.
            finalFocus={() => !usePalette.getState().handOff}
            className={cn(
              "glass fixed z-50 flex flex-col overflow-hidden rounded-surface p-2 text-foreground duration-100 outline-none data-open:animate-in data-open:fade-in-0 data-open:zoom-in-95 data-open:slide-in-from-top-2 data-closed:animate-out data-closed:fade-out-0 data-closed:zoom-out-95",
              mobile ? "inset-x-(--mobile-edge)" : "left-1/2 -translate-x-1/2"
            )}
            style={{
              top,
              width: mobile ? undefined : PANEL_WIDTH,
              maxWidth: mobile ? undefined : `calc(100vw - ${2 * EDGE}px)`,
              // Phones: above the bottom bar, or above the keyboard while it is up.
              maxHeight: mobile
                ? Math.min(
                    visible - MOBILE_EDGE,
                    layout - MOBILE_SHEET_BOTTOM
                  ) - MOBILE_PANEL_TOP
                : `min(480px, calc(${visible}px - ${top} - ${EDGE}px))`,
            }}
          >
            <DialogPrimitive.Title className="sr-only">
              {m.palette_open()}
            </DialogPrimitive.Title>
            <Command
              label={m.palette_open()}
              shouldFilter={false}
              loop
              className="flex min-h-0 flex-col"
            >
              <div className="relative mb-1 flex h-10 shrink-0 items-center rounded-row bg-field">
                <SearchIcon
                  aria-hidden="true"
                  className="pointer-events-none absolute left-3 size-4 text-muted-foreground"
                />
                <Command.Input
                  value={search}
                  onValueChange={setSearch}
                  placeholder={m.palette_search()}
                  className="h-full w-full bg-transparent pr-3 pl-9 text-base outline-none placeholder:text-muted-foreground md:text-row"
                />
              </div>
              <motion.div
                layoutScroll
                className="scroll-edge min-h-0 flex-1 overflow-y-auto overscroll-contain"
              >
                <Command.List>
                  <HighlightContext.Provider value={layoutId}>
                    {visibleGroups.map((group) => (
                      <Command.Group
                        key={group.key}
                        heading={group.heading}
                        className="[&_[cmdk-group-heading]]:px-3 [&_[cmdk-group-heading]]:pt-2 [&_[cmdk-group-heading]]:pb-1 [&_[cmdk-group-heading]]:text-meta [&_[cmdk-group-heading]]:text-muted-foreground"
                      >
                        {group.status === "loading" ? <LoadingRow /> : null}
                        {group.status === "failed" ? (
                          <NoteRow>{m.palette_failed()}</NoteRow>
                        ) : null}
                        {group.entries.map((entry) => (
                          <PaletteItem
                            key={entry.value}
                            entry={entry}
                            onChoose={choose}
                          />
                        ))}
                        {group.more > 0 ? (
                          <NoteRow>
                            {m.palette_more({ count: group.more })}
                          </NoteRow>
                        ) : null}
                      </Command.Group>
                    ))}
                    {!matched && !reading ? (
                      <NoteRow>{m.palette_empty()}</NoteRow>
                    ) : null}
                  </HighlightContext.Provider>
                </Command.List>
              </motion.div>
            </Command>
          </DialogPrimitive.Popup>
        </DialogPrimitive.Portal>
      </DialogPrimitive.Root>
      {grantable.length > 0 ? (
        <InviteDialog
          networkId={networkId}
          roles={grantable}
          open={inviting}
          onOpenChange={setInviting}
        />
      ) : null}
    </>
  )
}

const HighlightContext = createContext<string | undefined>(undefined)

const ROW = "relative isolate flex h-10 items-center gap-2.5 rounded-row px-3"
const LEAD = "flex size-4 shrink-0 items-center justify-center"

/** One result; the highlighted one has the glass menus' gliding pill. */
function PaletteItem({
  entry,
  onChoose,
}: {
  entry: Entry
  onChoose: (entry: Entry) => void
}) {
  const layoutId = useContext(HighlightContext)
  const reduce = useReducedMotion()
  const selected = useCommandState((state) => state.value === entry.value)
  return (
    <Command.Item
      value={entry.value}
      onSelect={() => onChoose(entry)}
      className={cn(ROW, "cursor-pointer text-row outline-none select-none")}
    >
      {selected ? (
        <motion.span
          aria-hidden="true"
          layoutId={layoutId}
          transition={reduce ? { duration: 0 } : SPRING_LAYOUT}
          className="absolute inset-0 -z-10 rounded-row bg-fill-hover"
        />
      ) : null}
      <span className={LEAD}>{entry.lead}</span>
      <span className="flex min-w-0 flex-1 items-center gap-2">
        {entry.label}
      </span>
      {entry.detail !== undefined ? (
        <span className="flex shrink-0 items-center text-meta text-muted-foreground tabular-nums">
          {entry.detail}
        </span>
      ) : null}
    </Command.Item>
  )
}

/** A group being read. */
function LoadingRow() {
  return (
    <div aria-hidden="true" className={ROW}>
      <span className={LEAD}>
        <Skeleton className="size-2 rounded-full" />
      </span>
      <Skeleton className="h-3 w-40 rounded-full" />
    </div>
  )
}

/** A fact in place of results: unreadable, how many more, nothing matching. */
function NoteRow({ children }: { children: ReactNode }) {
  return (
    <div className={cn(ROW, "text-row text-muted-foreground")}>
      <span className={LEAD} />
      {children}
    </div>
  )
}
