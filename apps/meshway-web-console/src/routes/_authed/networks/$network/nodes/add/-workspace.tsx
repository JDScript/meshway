import {
  createEnrollmentKey,
  type CreateEnrollmentKeyRequest,
  type EnrollmentKeyResponse,
  type NetworkSummary,
  type NodeInventoryResponse,
} from "@meshway/control-client"
import { Button } from "@meshway/ui/button"
import { useQuery, useQueryClient } from "@tanstack/react-query"
import { Link } from "@tanstack/react-router"
import { CheckIcon } from "lucide-react"
import { useEffect, useMemo, useRef, useState } from "react"

import { authed, client } from "@/api/client"
import { failureText } from "@/api/failure"
import { configurationQuery, isDenied } from "@/api/queries/configuration"
import { enrollmentKeysQuery, transportCatalogQuery } from "@/api/queries/nodes"
import { BusyLabel } from "@/components/chrome/busy-label"
import { CloseButton } from "@/components/chrome/close-button"
import { TextField } from "@/components/chrome/form"
import { GlassDialog, SecretField } from "@/components/chrome/glass-dialog"
import { EmptyLine, Section } from "@/components/chrome/section"
import { SegmentedControl } from "@/components/chrome/segmented-control"
import { SurfaceHeader } from "@/components/chrome/surface-header"
import { Tile } from "@/components/chrome/tile"
import { surfaceTitleId } from "@/components/surface/page-frame"
import { Skeleton } from "@/components/ui/skeleton"
import { useSecretMutation } from "@/hooks/use-secret-mutation"
import { dateTime } from "@/lib/format"
import {
  LISTENER_KEY,
  listenerOf,
  listenerProblems,
  newListener,
  presetsIn,
  problemField,
  withPreset,
  type ListenerField,
  type ListenerFields,
  type Preset,
} from "@/lib/nodes/listener-model"
import { hostnameOf, KEY_EXPIRIES, type KeyExpiry } from "@/lib/nodes/model"
import { m } from "@/paraglide/messages.js"
import { attempts } from "@/state/attempts"

import { expiryLabel, presetLabel } from "../-labels"
import {
  fieldLabelClass,
  ListenerInputs,
  listenerProblemText,
} from "../-listener-inputs"
import { EnrollmentKeys } from "./-enrollment-keys"
import { InstallSteps } from "./-install"

/** Long enough for the dialog to finish closing before the key is dropped. */
const UNMOUNT_AFTER_MS = 400
const NONE = "none"
type Choice = Preset | typeof NONE

type Body = Omit<CreateEnrollmentKeyRequest, "requestId">

/** By network. */
const enrollmentKeys = attempts<Body>("enrollment-key")

/**
 * Adding a node: an enrolment key for one Tag-identity device, with an
 * optional hostname and one optional listener, then the network's keys.
 * The key is shown once, in a dialog that drops it when it closes; this
 * controller provides no Agent install, and the page says so.
 */
export function AddNode({
  networkId,
  network,
  nodes,
  onClose,
  onOpenNode,
}: {
  networkId: string
  network: NetworkSummary | null
  nodes: NodeInventoryResponse | undefined
  onClose: () => void
  onOpenNode: (nodeId: string) => void
}) {
  const queryClient = useQueryClient()
  const configuration = useQuery(configurationQuery(networkId))
  const catalog = useQuery(transportCatalogQuery())
  const tagNames = useMemo(
    () =>
      configuration.data
        ? Object.keys(configuration.data.configuration.tagsOwners).sort()
        : null,
    [configuration.data]
  )
  const presets = useMemo(
    () =>
      catalog.data
        ? presetsIn(catalog.data.items.map((item) => item.protocol))
        : [],
    [catalog.data]
  )

  const [tags, setTags] = useState<readonly string[]>([])
  const [hostname, setHostname] = useState("")
  const [expiry, setExpiry] = useState<KeyExpiry>(86_400)
  const [choice, setChoice] = useState<Choice>(NONE)
  // Kept while "no listener" is chosen, so nothing typed is lost.
  const [fields, setFields] = useState<ListenerFields>(() =>
    newListener("native_udp")
  )
  const [attempted, setAttempted] = useState(false)

  // Tags that left the configuration are no longer chosen.
  const chosen = tagNames ? tags.filter((tag) => tagNames.includes(tag)) : tags
  const name = hostnameOf(hostname)
  const listenerIssues = choice === NONE ? [] : listenerProblems(fields)
  const problem =
    chosen.length === 0
      ? m.nodes_add_tags_missing()
      : name === null
        ? m.nodes_add_hostname_invalid()
        : listenerIssues[0]
          ? listenerProblemText[listenerIssues[0]]()
          : null
  const invalid = new Set<ListenerField>(
    attempted ? listenerIssues.map((issue) => problemField[issue]) : []
  )

  const secret = useSecretMutation(
    ({ key, body }: { key: string; body: () => Body }) =>
      enrollmentKeys.send(
        networkId,
        key,
        body,
        (attempt): Promise<EnrollmentKeyResponse> =>
          authed((token) =>
            createEnrollmentKey(
              client,
              { networkId },
              { requestId: attempt.requestId, ...attempt.body },
              { token }
            )
          )
      ),
    {
      permission_denied: () => m.nodes_manage_denied(),
      invalid_request: () => m.nodes_add_refused(),
    }
  )

  // Shown from the moment the key arrives until the dialog is closed; the
  // key is dropped once the dialog has finished closing.
  const created = secret.result
  const [dismissed, setDismissed] = useState(false)
  const { clear } = secret
  const closing = useRef<number | undefined>(undefined)
  useEffect(() => () => window.clearTimeout(closing.current), [])
  const dismiss = () => {
    setDismissed(true)
    window.clearTimeout(closing.current)
    closing.current = window.setTimeout(() => {
      clear()
      setDismissed(false)
    }, UNMOUNT_AFTER_MS)
  }
  useEffect(() => {
    if (created)
      void queryClient.invalidateQueries({
        queryKey: enrollmentKeysQuery(networkId).queryKey,
      })
  }, [created, queryClient, networkId])

  const submit = () => {
    setAttempted(true)
    if (problem !== null || name === null || secret.pending) return
    const listener = choice === NONE ? null : listenerOf(fields)
    if (choice !== NONE && listener === null) return
    const input = {
      tags: [...chosen].sort(),
      hostname: name,
      expiry,
      listener:
        choice === NONE ? null : { key: LISTENER_KEY[choice], listener },
    }
    secret.create({
      key: JSON.stringify(input),
      body: () => ({
        tags: input.tags,
        initialSettings: {
          ...(name === "" ? {} : { hostname: name }),
          listeners:
            input.listener && input.listener.listener
              ? { [input.listener.key]: input.listener.listener }
              : {},
        },
        expiresAt: new Date(Date.now() + expiry * 1000).toISOString(),
      }),
    })
  }

  const toggle = (tag: string) =>
    setTags((current) =>
      current.includes(tag)
        ? current.filter((entry) => entry !== tag)
        : [...current, tag]
    )

  return (
    <div className="swap-in flex min-h-0 flex-1 flex-col">
      <SurfaceHeader
        size="lg"
        className="px-6 pt-6 pb-4"
        title={(className) => (
          <h2 id={surfaceTitleId("nodes", "workspace")} className={className}>
            {m.nodes_add()}
          </h2>
        )}
        action={<CloseButton onClick={onClose} />}
      />
      <form
        noValidate
        className="flex min-h-0 flex-1 flex-col"
        onSubmit={(event) => {
          event.preventDefault()
          submit()
        }}
      >
        <div className="scroll-edge min-h-0 flex-1 overflow-y-auto px-6 pt-1 pb-6">
          <div className="flex max-w-workspace flex-col gap-8">
            <Section title={m.nodes_add_tags()} count={chosen.length}>
              {tagNames === null ? (
                configuration.isError ? (
                  <EmptyLine>
                    {isDenied(configuration.error)
                      ? m.nodes_listeners_denied()
                      : (failureText(configuration.error) ??
                        m.nodes_listeners_failed())}
                  </EmptyLine>
                ) : (
                  <Skeleton className="h-12 w-full rounded-tile" />
                )
              ) : tagNames.length === 0 ? (
                <EmptyLine>
                  <span className="min-w-0 flex-1 truncate">
                    {m.nodes_add_tags_none()}
                  </span>
                  <Link
                    to="/networks/$network/configuration/$view"
                    params={{ network: networkId, view: "source" }}
                    search={{ at: "/tagsOwners" }}
                    className="shrink-0 font-medium text-foreground underline-offset-4 hover:underline"
                  >
                    {m.nodes_add_tags_define()}
                  </Link>
                </EmptyLine>
              ) : (
                <fieldset className="flex flex-wrap gap-1.5">
                  <legend className="sr-only">{m.nodes_add_tags()}</legend>
                  {tagNames.map((tag) => {
                    const on = chosen.includes(tag)
                    return (
                      <button
                        key={tag}
                        type="button"
                        aria-pressed={on}
                        onClick={() => toggle(tag)}
                        data-invalid={
                          (attempted && chosen.length === 0) || undefined
                        }
                        className="inline-flex h-8 items-center gap-1.5 rounded-full border border-border px-3 font-mono text-meta transition-colors outline-none hover:bg-fill-hover focus-visible:ring-2 focus-visible:ring-ring aria-pressed:border-transparent aria-pressed:bg-fill-selected aria-pressed:text-foreground aria-pressed:hover:bg-fill-selected data-invalid:border-warning [&:not([aria-pressed=true])]:text-muted-foreground"
                      >
                        {on ? <CheckIcon className="-ml-0.5 size-3.5" /> : null}
                        {tag}
                      </button>
                    )
                  })}
                </fieldset>
              )}
            </Section>

            <div className="grid gap-4 sm:grid-cols-[1fr_auto]">
              <TextField
                id="add-node-hostname"
                name="hostname"
                label={m.nodes_add_hostname()}
                autoComplete="off"
                autoCapitalize="none"
                spellCheck={false}
                maxLength={63}
                className="font-mono placeholder:font-sans"
                placeholder={m.nodes_add_hostname_placeholder()}
                value={hostname}
                invalid={attempted && name === null}
                onChange={(event) => setHostname(event.target.value)}
              />
              <div className="flex flex-col gap-2">
                <span className={fieldLabelClass}>{m.nodes_add_expiry()}</span>
                <SegmentedControl<`${KeyExpiry}`>
                  label={m.nodes_add_expiry()}
                  value={`${expiry}`}
                  options={KEY_EXPIRIES.map((seconds) => ({
                    value: `${seconds}`,
                    label: expiryLabel[seconds](),
                  }))}
                  onChange={(value) => {
                    const next = KEY_EXPIRIES.find(
                      (seconds) => `${seconds}` === value
                    )
                    if (next !== undefined) setExpiry(next)
                  }}
                  className="h-10 self-start"
                />
              </div>
            </div>

            <Section title={m.nodes_add_listener()}>
              <div className="flex flex-col gap-4">
                {catalog.data === undefined ? (
                  catalog.isError ? (
                    <EmptyLine>
                      {failureText(catalog.error) ?? m.nodes_catalog_failed()}
                    </EmptyLine>
                  ) : (
                    <Skeleton className="h-9 w-80 rounded-full" />
                  )
                ) : (
                  <SegmentedControl<Choice>
                    label={m.nodes_add_listener()}
                    value={choice}
                    options={[
                      { value: NONE, label: m.nodes_add_listener_none() },
                      ...presets.map((preset) => ({
                        value: preset,
                        label: presetLabel[preset](),
                      })),
                    ]}
                    onChange={(next) => {
                      setChoice(next)
                      if (next !== NONE)
                        setFields((current) => withPreset(current, next))
                    }}
                    className="self-start"
                  />
                )}
                {choice !== NONE ? (
                  <ListenerInputs
                    fields={fields}
                    invalid={invalid}
                    onChange={setFields}
                  />
                ) : null}
              </div>
            </Section>

            <Section title={m.nodes_install()}>
              <InstallSteps />
            </Section>

            <EnrollmentKeys
              networkId={networkId}
              network={network}
              nodes={nodes}
              onOpenNode={onOpenNode}
            />
          </div>
        </div>
        <footer className="flex h-16 shrink-0 items-center gap-3 border-t border-divider px-6">
          <span className="min-w-0 flex-1 truncate text-row text-warning">
            {attempted ? problem : null}
          </span>
          <Button
            type="submit"
            className="h-9 rounded-full px-4"
            disabled={secret.pending || tagNames === null}
            aria-busy={secret.pending}
          >
            <BusyLabel busy={secret.pending}>{m.nodes_add_submit()}</BusyLabel>
          </Button>
        </footer>
      </form>

      {created ? (
        <KeyDialog open={!dismissed} created={created} onClose={dismiss} />
      ) : null}
    </div>
  )
}

/** The key just created: shown once, here only. */
function KeyDialog({
  open,
  created,
  onClose,
}: {
  open: boolean
  created: EnrollmentKeyResponse
  onClose: () => void
}) {
  const secret =
    created.secretDelivery === "available" ? (created.secret ?? null) : null
  return (
    <GlassDialog
      open={open}
      onOpenChange={(next) => {
        if (!next) onClose()
      }}
      title={m.nodes_key_created()}
      footer={
        <Button className="h-9 rounded-full px-4" onClick={onClose}>
          {m.nodes_done()}
        </Button>
      }
    >
      <div className="grid grid-cols-2 gap-2">
        <Tile label={m.nodes_add_tags()}>
          <span className="font-mono font-normal">
            {created.key.tags.join(", ")}
          </span>
        </Tile>
        <Tile label={m.nodes_key_expires()}>
          {dateTime(created.key.expiresAt) ?? "—"}
        </Tile>
      </div>
      {secret ? (
        <SecretField label={m.nodes_key_secret()} value={secret} />
      ) : (
        <p className="text-row text-muted-foreground">
          {m.nodes_key_unavailable()}
        </p>
      )}
      {secret ? <InstallSteps joining /> : null}
    </GlassDialog>
  )
}
