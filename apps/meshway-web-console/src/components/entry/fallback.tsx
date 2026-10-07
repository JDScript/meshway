import { useQuery } from "@tanstack/react-query"
import {
  useLocation,
  useMatch,
  useNavigate,
  type ErrorComponentProps,
} from "@tanstack/react-router"
import { useLayoutEffect, useRef, type ReactNode } from "react"
import { useStore } from "zustand"

import { ApiFailure } from "@/api/client"
import { failureText } from "@/api/failure"
import { sessionQuery } from "@/api/queries/auth"
import { useSignOut } from "@/hooks/use-sign-out"
import { m } from "@/paraglide/messages.js"
import { setGlobeAlert, type GlobeAlert } from "@/state/globe"
import { sessionStore } from "@/state/session"

import { EntryCard, EntryFoot, EntryLink, EntryPage } from "./frame"

// What is shown where a page cannot be: an address with no page, a page
// that failed. One card below the globe, which drops to the middle above it
// and loses its signal (state/globe.ts, GlobeAlert): outside a network with
// the preferences in the corner (EntryPage), inside one with the network's
// rail and bar staying.

/** The space kept between the globe's place and the card's top. */
const CARD_GAP = 24

/** Tells the globe a page cannot be shown, and how much of the bottom the card covers. */
function useGlobeAlert(tone: GlobeAlert["tone"]) {
  const card = useRef<HTMLDivElement>(null)
  useLayoutEffect(() => {
    const element = card.current
    if (!element) return
    const measure = () =>
      setGlobeAlert({
        tone,
        bottom: Math.max(
          0,
          window.innerHeight - element.getBoundingClientRect().top + CARD_GAP
        ),
      })
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(element)
    window.addEventListener("resize", measure)
    return () => {
      observer.disconnect()
      window.removeEventListener("resize", measure)
      setGlobeAlert(null)
    }
  }, [tone])
  return card
}

/** The network whose shell this renders in; undefined outside one. */
const useShellNetwork = () =>
  useMatch({ from: "/_authed/networks/$network", shouldThrow: false })?.params
    .network

/** A card with what happened and the ways on, in or outside a network's shell. */
export function FallbackCard({
  inShell,
  tone,
  title,
  body,
  detail,
  fact,
  actions,
}: {
  /** Inside a network's shell, which stays drawn around it. */
  inShell: boolean
  /** Amber rings for what is not there, red for what failed. */
  tone: GlobeAlert["tone"]
  title: string
  body?: string | null
  /** A technical detail, e.g. an error's message. */
  detail?: string | null
  /** At the card's foot, before the actions (the account). */
  fact?: ReactNode
  actions: ReactNode
}) {
  const place = useGlobeAlert(tone)
  const card = (
    <EntryCard title={title}>
      {body ? (
        <p className="mt-7 text-row text-muted-foreground">{body}</p>
      ) : null}
      {detail ? (
        <code className="mt-3 block font-mono text-meta break-words text-muted-foreground">
          {detail}
        </code>
      ) : null}
      <EntryFoot>
        <span className="min-w-0 truncate">{fact}</span>
        <span className="flex shrink-0 items-center gap-4">{actions}</span>
      </EntryFoot>
    </EntryCard>
  )
  const below = (
    <div className="flex flex-1 items-end justify-center px-6 pb-[7vh]">
      <div ref={place} className="pointer-events-auto w-full max-w-95">
        {card}
      </div>
    </div>
  )
  return inShell ? (
    <main className="pointer-events-none relative flex min-h-svh flex-col">
      {below}
    </main>
  ) : (
    <EntryPage below>{below}</EntryPage>
  )
}

/** The signed-in account, for a card's foot. */
export function AccountFact() {
  const session = useQuery(sessionQuery())
  return session.data?.user.email ?? null
}

/** Signs out, from a card's foot. */
export function SignOutLink() {
  const signOut = useSignOut()
  return <EntryLink onClick={() => void signOut()}>{m.sign_out()}</EntryLink>
}

/** Opens the console (`/`: the network last open, else the first, else creating one). */
export function OpenConsoleLink() {
  const navigate = useNavigate()
  return (
    <EntryLink onClick={() => void navigate({ to: "/" })}>
      {m.open_console()}
    </EntryLink>
  )
}

/** No page at this address: back to the network's overview, the console, or the sign-in. */
export function PageNotFound() {
  const navigate = useNavigate()
  const { pathname } = useLocation()
  const network = useShellNetwork()
  const signedIn = useStore(sessionStore, (state) => state.session !== null)
  return (
    <FallbackCard
      inShell={network !== undefined}
      tone="missing"
      title={m.page_not_found()}
      body={m.page_not_found_body({ path: pathname })}
      actions={
        network !== undefined ? (
          <EntryLink
            onClick={() =>
              void navigate({
                to: "/networks/$network/overview",
                params: { network },
              })
            }
          >
            {m.page_back_to_overview()}
          </EntryLink>
        ) : signedIn ? (
          <OpenConsoleLink />
        ) : (
          <EntryLink onClick={() => void navigate({ to: "/login" })}>
            {m.login_title()}
          </EntryLink>
        )
      }
    />
  )
}

/** The console was updated since this page loaded: the files it asks for are gone. */
const STALE_BUNDLE =
  /dynamically imported module|Importing a module script failed|Unable to preload CSS/i

/** A page failed to show: why, when it is known, and reloading. */
export function PageFailed({ error }: ErrorComponentProps) {
  const inShell = useShellNetwork() !== undefined
  const stale = error instanceof Error && STALE_BUNDLE.test(error.message)
  const known = error instanceof ApiFailure
  return (
    <FallbackCard
      inShell={inShell}
      tone="failed"
      title={m.page_failed()}
      body={
        stale
          ? m.page_failed_stale()
          : known
            ? failureText(error)
            : m.page_failed_unexpected()
      }
      detail={!stale && !known && error instanceof Error ? error.message : null}
      actions={
        <EntryLink onClick={() => window.location.reload()}>
          {m.page_reload()}
        </EntryLink>
      }
    />
  )
}
