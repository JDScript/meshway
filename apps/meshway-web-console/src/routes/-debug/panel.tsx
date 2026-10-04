import { useQueryClient } from "@tanstack/react-query"
import { useParams } from "@tanstack/react-router"
import {
  ClipboardCopyIcon,
  EraserIcon,
  PauseIcon,
  PlayIcon,
  RefreshCcwDotIcon,
  RotateCwIcon,
  UnplugIcon,
} from "lucide-react"
import { useRef, type ReactNode } from "react"
import { toast } from "sonner"

import { streamControls } from "@/api/live/stream"
import { networkKey } from "@/api/queries/network"
import { CloseButton } from "@/components/chrome/close-button"
import { IconAction } from "@/components/chrome/icon-action"
import { SegmentedTabs } from "@/components/chrome/segmented-control"
import { Tabs, TabsContent } from "@/components/ui/tabs"
import { m } from "@/paraglide/messages.js"
import {
  closeDebugPanel,
  setDebugHeight,
  setDebugTab,
  useDebugHeight,
  useDebugTab,
  type DebugTab,
} from "@/state/debug"
import { clearTrace, useTraceSince } from "@/state/debug-trace"
import { useConnections } from "@/state/live"

import { CacheTab } from "./cache-tab"
import { ConsistencyTab } from "./consistency-tab"
import { diagnostics } from "./diagnostics"
import { traceTime } from "./format"
import { LocalTab } from "./local-tab"
import { RequestsTab } from "./requests-tab"
import { StreamTab } from "./stream-tab"
import { TimeTab } from "./time-tab"

const TABS = [
  "stream",
  "consistency",
  "cache",
  "requests",
  "local",
  "time",
] as const satisfies readonly DebugTab[]

const tabLabel: Record<DebugTab, () => string> = {
  stream: m.debug_tab_stream,
  consistency: m.debug_tab_consistency,
  cache: m.debug_tab_cache,
  requests: m.debug_requests,
  local: m.debug_tab_local,
  time: m.debug_tab_time,
}

const isTab = (value: unknown): value is DebugTab =>
  TABS.some((tab) => tab === value)

/** Arrow keys move the edge this far. */
const KEY_STEP = 24

/**
 * The debug panel: a glass sheet over the bottom of the window, above the
 * surfaces and below dialogs and toasts, its height dragged at its top
 * edge. It shows the open network where a tab is about one.
 */
export function DebugPanel() {
  const tab = useDebugTab()
  const height = useDebugHeight()
  const since = useTraceSince()
  const { network } = useParams({ strict: false })
  const networkId = network ?? null

  return (
    <section
      aria-label={m.debug_title()}
      style={{ height }}
      className="glass fixed inset-x-(--mobile-edge) bottom-(--mobile-edge) z-40 flex flex-col rounded-surface md:inset-x-(--edge) md:bottom-(--edge)"
    >
      <ResizeEdge height={height} />
      <Tabs
        value={tab}
        onValueChange={(value) => {
          if (isTab(value)) setDebugTab(value)
        }}
        className="min-h-0 flex-1 gap-0"
      >
        <div className="flex items-center gap-1 px-4 pt-3 pb-3">
          <div className="flex min-w-0 flex-1 items-center gap-3 overflow-x-auto">
            <SegmentedTabs
              value={tab}
              options={TABS.map((value) => ({
                value,
                label: tabLabel[value](),
              }))}
            />
            <span className="shrink-0 text-meta text-muted-foreground tabular-nums">
              {m.debug_recording_since({ time: traceTime(since) })}
            </span>
          </div>
          <StreamActions networkId={networkId} />
          <CopyDiagnostics />
          <IconAction
            label={m.debug_clear()}
            icon={EraserIcon}
            onClick={clearTrace}
          />
          <CloseButton onClick={closeDebugPanel} />
        </div>
        <Pane value="stream">
          <StreamTab networkId={networkId} />
        </Pane>
        <Pane value="consistency">
          <ConsistencyTab networkId={networkId} />
        </Pane>
        <Pane value="cache">
          <CacheTab networkId={networkId} />
        </Pane>
        <Pane value="requests">
          <RequestsTab />
        </Pane>
        <Pane value="local">
          <LocalTab networkId={networkId} />
        </Pane>
        <Pane value="time">
          <TimeTab networkId={networkId} />
        </Pane>
      </Tabs>
    </section>
  )
}

function Pane({ value, children }: { value: DebugTab; children: ReactNode }) {
  return (
    <TabsContent value={value} className="flex min-h-0 flex-col">
      <div className="scroll-edge min-h-0 flex-1 overflow-y-auto px-4 pt-1 pb-4">
        {children}
      </div>
    </TabsContent>
  )
}

/** The top edge: dragged (or moved with the arrow keys) it sets the panel's height. */
function ResizeEdge({ height }: { height: number }) {
  const start = useRef<{ y: number; height: number } | null>(null)
  return (
    <div
      role="separator"
      aria-orientation="horizontal"
      aria-label={m.debug_resize()}
      aria-valuenow={height}
      tabIndex={0}
      className="absolute inset-x-0 -top-1.5 z-10 flex h-4 cursor-row-resize touch-none justify-center rounded-full outline-none focus-visible:ring-2 focus-visible:ring-ring"
      onPointerDown={(event) => {
        event.currentTarget.setPointerCapture(event.pointerId)
        start.current = { y: event.clientY, height }
      }}
      onPointerMove={(event) => {
        if (start.current)
          setDebugHeight(start.current.height + start.current.y - event.clientY)
      }}
      onPointerUp={() => (start.current = null)}
      onPointerCancel={() => (start.current = null)}
      onKeyDown={(event) => {
        if (event.key === "ArrowUp") setDebugHeight(height + KEY_STEP)
        else if (event.key === "ArrowDown") setDebugHeight(height - KEY_STEP)
        else return
        event.preventDefault()
      }}
    >
      <span className="mt-2 h-1 w-10 rounded-full bg-foreground/15" />
    </div>
  )
}

/** What the panel may do to the open network's stream, and reading it all again. */
function StreamActions({ networkId }: { networkId: string | null }) {
  const queryClient = useQueryClient()
  const connections = useConnections()
  const phase = networkId ? (connections[networkId]?.phase ?? null) : null
  const controls = () => (networkId ? streamControls(networkId) : null)
  const paused = phase === "paused"
  return (
    <>
      <IconAction
        label={m.debug_drop()}
        icon={UnplugIcon}
        disabled={phase !== "live"}
        onClick={() => controls()?.drop()}
      />
      <IconAction
        label={m.debug_resync()}
        icon={RefreshCcwDotIcon}
        disabled={phase !== "live"}
        onClick={() => controls()?.resync()}
      />
      <IconAction
        label={paused ? m.debug_resume() : m.debug_pause()}
        icon={paused ? PlayIcon : PauseIcon}
        disabled={!paused && phase !== "live" && phase !== "connecting"}
        onClick={() => (paused ? controls()?.resume() : controls()?.pause())}
      />
      <IconAction
        label={m.debug_refetch()}
        icon={RotateCwIcon}
        disabled={networkId === null}
        onClick={() => {
          if (networkId)
            void queryClient.invalidateQueries({
              queryKey: networkKey(networkId),
            })
        }}
      />
    </>
  )
}

function CopyDiagnostics() {
  const queryClient = useQueryClient()
  const connections = useConnections()
  return (
    <IconAction
      label={m.debug_copy()}
      icon={ClipboardCopyIcon}
      onClick={() => {
        const text = JSON.stringify(
          diagnostics(queryClient, connections),
          null,
          2
        )
        navigator.clipboard.writeText(text).then(
          () => toast.success(m.debug_copied()),
          () => toast.error(m.debug_copy_failed())
        )
      }}
    />
  )
}
