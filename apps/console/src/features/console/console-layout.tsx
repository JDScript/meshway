import { Link, Outlet, useNavigate } from "@tanstack/react-router"
import { GlobeIcon, ServerIcon, type LucideIcon } from "lucide-react"
import { useLayoutEffect, useRef } from "react"

import { LanguageSwitch, ThemeSwitch } from "@/components/chrome/preferences"
import { AccountMenu } from "@/features/auth/account-menu"
import { DEFAULT_LOOK } from "@/features/globe/look"
import { setGlobe } from "@/features/globe/store"
import { useNetworkView } from "@/features/network/use-network-view"
import { useIsMobile } from "@/hooks/use-mobile"
import { m } from "@/paraglide/messages.js"

import { OrbRing } from "./orb-ring"

/** Radius of the docked globe; the ring sits just outside it. */
const ORB_RADIUS = 16

type NavItem = { to: "/networks/$network/overview" | "/networks/$network/nodes"; label: () => string; icon: LucideIcon }

const NAV: NavItem[] = [
  { to: "/networks/$network/overview", label: () => m.nav_overview(), icon: GlobeIcon },
  { to: "/networks/$network/nodes", label: () => m.nav_nodes(), icon: ServerIcon },
]

/**
 * Console mode. The globe shrinks into the orb at the top of the sidebar and
 * keeps turning there, still drawing its links; the ring around it is the
 * link-state bar of globe mode. The orb leads back to the globe.
 */
export function ConsoleLayout({ networkId }: { networkId: string }) {
  const view = useNetworkView(networkId)
  const orb = useRef<HTMLButtonElement>(null)
  const navigate = useNavigate()
  const mobile = useIsMobile()

  useLayoutEffect(() => {
    const element = orb.current
    if (!element) return
    const place = () => {
      const box = element.getBoundingClientRect()
      setGlobe({
        framing: "dock",
        dock: { x: box.left + box.width / 2, y: box.top + box.height / 2, radius: ORB_RADIUS },
        selectedNodeId: null,
        rightInset: 0,
        look: DEFAULT_LOOK,
      })
    }
    place()
    const observer = new ResizeObserver(place)
    observer.observe(document.documentElement)
    return () => observer.disconnect()
  }, [mobile])

  const known = view.links?.kind === "known" ? view.links : null

  return (
    <div className="ui-layer fixed inset-0 flex flex-col md:flex-row">
      <aside className="pointer-events-auto flex shrink-0 flex-col px-4 pt-4 md:w-[248px] md:pt-5 md:pb-4">
        <div className="flex items-center gap-3">
          <button
            ref={orb}
            type="button"
            aria-label={m.console_back_to_globe()}
            onClick={() => void navigate({ to: "/networks/$network/overview", params: { network: networkId } })}
            className="enter relative size-[60px] shrink-0 rounded-full outline-none focus-visible:ring-2 focus-visible:ring-ring"
            style={{ "--enter-delay": "650ms", "--enter-from": "0px", "--enter-scale": "0.8" }}
          >
            <OrbRing links={view.links} />
          </button>
          <div className="enter min-w-0" style={{ "--enter-delay": "200ms", "--enter-from": "6px" }}>
            <div className="truncate text-sm font-semibold">{view.network?.name ?? "\u00a0"}</div>
            <div className="mt-0.5 text-xs text-muted-foreground tabular-nums">
              {m.console_summary({
                nodes: view.nodeCount === null ? "—" : String(view.nodeCount),
                confirmed: known ? String(known.confirmed) : "—",
                total: known ? String(known.total) : "—",
              })}
            </div>
          </div>
        </div>

        <nav className="mt-4 flex gap-1 md:mt-8 md:flex-col md:gap-0.5" aria-label={m.console_navigation()}>
          {NAV.map((item, index) => (
            <Link
              key={item.to}
              to={item.to}
              params={{ network: networkId }}
              className="enter flex h-9 items-center gap-3 rounded-xl px-3 text-sm text-muted-foreground transition-colors hover:bg-foreground/5 hover:text-foreground data-[status=active]:bg-foreground/7 data-[status=active]:font-medium data-[status=active]:text-foreground"
              style={{ "--enter-delay": `${260 + index * 50}ms`, "--enter-from": "6px" }}
            >
              <item.icon className="size-4" />
              {item.label()}
            </Link>
          ))}
        </nav>

        <div className="enter mt-auto hidden items-center gap-0.5 md:flex" style={{ "--enter-delay": "400ms", "--enter-from": "6px" }}>
          <LanguageSwitch />
          <ThemeSwitch />
          <AccountMenu />
        </div>
      </aside>

      <main
        className="enter console-panel pointer-events-auto m-3 min-h-0 min-w-0 flex-1 overflow-y-auto rounded-[28px] md:my-3 md:mr-3 md:ml-0"
        // Starts once the globe has visibly set off for the dock.
        style={{ "--enter-delay": "380ms", "--enter-from": "22px" }}
      >
        <Outlet />
      </main>
    </div>
  )
}
