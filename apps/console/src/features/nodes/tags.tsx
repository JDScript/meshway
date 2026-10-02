import { saveTags, type TagDefinition } from "@meshway/control-client"
import { useQuery, useQueryClient } from "@tanstack/react-query"
import { ArrowUpRightIcon, RotateCcwIcon, Trash2Icon } from "lucide-react"
import { useMemo, useState } from "react"

import { CloseButton } from "@/components/chrome/close-button"
import { TextField } from "@/components/chrome/form"
import { GlassDialog } from "@/components/chrome/glass-dialog"
import { SurfaceHeader } from "@/components/chrome/surface-header"
import { Button } from "@/components/ui/button"
import { Field, FieldLabel } from "@/components/ui/field"
import {
  InputGroup,
  InputGroupAddon,
  InputGroupInput,
} from "@/components/ui/input-group"
import { Skeleton } from "@/components/ui/skeleton"
import { nodesQuery, tagsQuery } from "@/features/network/queries"
import { useCanManage } from "@/features/network/role"
import { useDraftCommit } from "@/features/shell/draft-commit"
import type { Saved } from "@/features/shell/drafts"
import { surfaceTitleId } from "@/features/shell/page-frame"
import { authed } from "@/lib/api"
import { m } from "@/paraglide/messages.js"

import { useTagCatalogDraft } from "./drafts"
import type { NodeContext } from "./node-facts"
import { EmptyLine, TabSection } from "./sections"
import {
  isTagName,
  MAX_TAGS,
  TAG_PREFIX,
  tagMembers,
  tagName,
  tagProblems,
  type TagProblem,
} from "./tag-model"

const PLACEHOLDER_ROWS = 4

/** The catalog as saved and as drafted, shared by the panel and the workspace. */
function useCatalog(networkId: string) {
  const tags = useQuery(tagsQuery(networkId))
  const saved: Saved<TagDefinition[]> | undefined = useMemo(
    () =>
      tags.data
        ? { revision: tags.data.revision, value: tags.data.tags }
        : undefined,
    [tags.data]
  )
  const draft = useTagCatalogDraft(networkId, saved)
  const members = useMemo(() => tagMembers(tags.data), [tags.data])
  return { tags, saved, draft, members }
}

/** The tag catalog as the nodes page's panel: saved tags, with what the draft adds or removes. */
export function TagList({
  networkId,
  openTag,
  onOpen,
}: {
  networkId: string
  openTag: string | null
  onOpen: (tag: string) => void
}) {
  const { tags, saved, draft, members } = useCatalog(networkId)
  const drafted = draft.value ?? []
  const savedNames = new Set(saved?.value.map((tag) => tag.tag))
  const draftNames = new Set(drafted.map((tag) => tag.tag))
  const removed = (saved?.value ?? []).filter((tag) => !draftNames.has(tag.tag))
  const rows = [
    ...drafted.map((tag) => ({
      tag,
      state: savedNames.has(tag.tag) ? ("saved" as const) : ("added" as const),
    })),
    ...removed.map((tag) => ({ tag, state: "removed" as const })),
  ].sort((a, b) => a.tag.tag.localeCompare(b.tag.tag))

  return (
    <ul
      aria-labelledby={surfaceTitleId("nodes", "panel")}
      className="min-h-0 flex-1 overflow-y-auto px-2 pb-2"
    >
      {!tags.data && !tags.isError
        ? Array.from({ length: PLACEHOLDER_ROWS }, (_, index) => (
            <li key={index} className="flex h-14 items-center gap-3 px-3">
              <Skeleton className="h-3 w-28 rounded-full" />
            </li>
          ))
        : null}
      {!tags.data && tags.isError ? (
        <li className="flex h-14 items-center px-3 text-row text-muted-foreground">
          {m.nodes_tags_failed()}
        </li>
      ) : null}
      {tags.data && rows.length === 0 ? (
        <li className="flex h-14 items-center px-3 text-row text-muted-foreground">
          {m.nodes_tags_none()}
        </li>
      ) : null}
      {rows.map(({ tag, state }) => (
        <li key={tag.tag}>
          <button
            type="button"
            aria-current={tag.tag === openTag ? "true" : undefined}
            onClick={() => onOpen(tag.tag)}
            className="list-row h-14 gap-3 px-3"
          >
            <span className="min-w-0 flex-1">
              <span
                className={
                  state === "removed"
                    ? "block truncate font-mono text-row font-medium text-muted-foreground line-through"
                    : "block truncate font-mono text-row font-medium"
                }
              >
                {tag.tag}
              </span>
              <span className="block truncate text-meta text-muted-foreground">
                {state === "added"
                  ? m.node_draft_mark()
                  : state === "removed"
                    ? m.nodes_tag_removing()
                    : tag.description || "—"}
              </span>
            </span>
            <span className="shrink-0 text-meta text-muted-foreground tabular-nums">
              {state === "added"
                ? ""
                : String(members.get(tag.tag)?.length ?? 0)}
            </span>
          </button>
        </li>
      ))}
    </ul>
  )
}

const tagProblemText = (problem: TagProblem) =>
  problem.kind === "too_many"
    ? m.nodes_tag_too_many({ max: String(MAX_TAGS) })
    : problem.kind === "name"
      ? m.nodes_tag_name_invalid()
      : problem.kind === "duplicate"
        ? m.nodes_tag_duplicate()
        : m.nodes_tag_description_invalid()

/** One tag: its description (edited in the catalog's draft), its nodes, its removal. */
export function TagWorkspace({
  networkId,
  tag,
  context,
  onClose,
  onOpenNode,
}: {
  networkId: string
  tag: string
  context: NodeContext
  onClose: () => void
  onOpenNode: (nodeId: string) => void
}) {
  const queryClient = useQueryClient()
  const canManage = useCanManage(networkId)
  const { tags, saved, draft, members } = useCatalog(networkId)
  const drafted = draft.value?.find((entry) => entry.tag === tag)
  const kept = saved?.value.find((entry) => entry.tag === tag)
  const nodeIds = members.get(tag) ?? []
  const problems = draft.value ? tagProblems(draft.value) : []
  const first = problems[0]

  const commit = useDraftCommit({
    page: "nodes",
    draft,
    saved,
    problem: first ? tagProblemText(first) : null,
    save: (value, revision) =>
      authed((base, token) =>
        saveTags(base, token, networkId, {
          expected_revision: revision,
          tags: value,
        })
      ),
    onSaved: (snapshot) => {
      queryClient.setQueryData(tagsQuery(networkId).queryKey, snapshot)
      void queryClient.invalidateQueries({
        queryKey: nodesQuery(networkId).queryKey,
      })
      // The open tag was deleted: nothing is left to show.
      if (!snapshot.tags.some((entry) => entry.tag === tag)) onClose()
    },
    refresh: () => void tags.refetch(),
  })

  const loading = !tags.data
  const state = drafted
    ? kept
      ? "saved"
      : "added"
    : kept
      ? "removed"
      : "missing"

  const setDescription = (description: string) =>
    draft.edit((current) =>
      current.map((entry) =>
        entry.tag === tag ? { ...entry, description } : entry
      )
    )
  const remove = () =>
    draft.edit((current) => current.filter((entry) => entry.tag !== tag))
  const restore = () =>
    draft.edit((current) =>
      kept && !current.some((entry) => entry.tag === tag)
        ? [...current, kept]
        : current
    )

  return (
    <div className="swap-in flex min-h-0 flex-1 flex-col" key={tag}>
      <SurfaceHeader
        size="lg"
        className="px-6 pt-6 pb-4"
        title={(className) =>
          loading && !tags.isError ? (
            <Skeleton className="h-6 w-40 flex-1 rounded-full" />
          ) : (
            <h2 id={surfaceTitleId("nodes", "workspace")} className={className}>
              {state === "missing" ? m.nodes_tag_not_found() : tag}
            </h2>
          )
        }
        action={<CloseButton onClick={onClose} />}
        meta={
          <span className="h-4 text-xs text-muted-foreground">
            {state === "added"
              ? m.node_draft_mark()
              : state === "removed"
                ? m.nodes_tag_removing()
                : state === "saved"
                  ? m.nodes_tag_members_count({ count: String(nodeIds.length) })
                  : null}
          </span>
        }
      />
      <form
        className="min-h-0 flex-1 overflow-y-auto px-6 pt-1 pb-(--commit-space)"
        onSubmit={(event) => {
          event.preventDefault()
          commit.submit()
        }}
      >
        <button type="submit" hidden />
        <div className="flex max-w-xl flex-col gap-8">
          {loading ? (
            tags.isError ? (
              <EmptyLine>{m.nodes_tags_failed()}</EmptyLine>
            ) : (
              <Skeleton className="h-10 w-full rounded-tile" />
            )
          ) : state === "missing" ? null : (
            <>
              <TextField
                id="tag-description"
                placeholder={m.nodes_tag_description_placeholder()}
                name="tag-description"
                label={m.nodes_tag_description()}
                autoComplete="off"
                maxLength={256}
                value={(drafted ?? kept)?.description ?? ""}
                invalid={problems.some(
                  (problem) =>
                    problem.kind === "description" && problem.tag === tag
                )}
                disabled={!canManage || state === "removed"}
                onChange={(event) => setDescription(event.target.value)}
              />
              <TabSection
                title={m.nodes_tag_members()}
                count={String(nodeIds.length)}
              >
                {nodeIds.length === 0 ? (
                  <EmptyLine>{m.nodes_tag_members_none()}</EmptyLine>
                ) : (
                  <ul className="flex flex-col">
                    {nodeIds.map((nodeId) => {
                      const node = context.nodes.get(nodeId)
                      return (
                        <li key={nodeId}>
                          <button
                            type="button"
                            onClick={() => onOpenNode(nodeId)}
                            className="list-row h-12 gap-3 px-3"
                          >
                            <span className="min-w-0 flex-1 truncate text-row">
                              {node?.name ?? nodeId.slice(0, 12)}
                            </span>
                            <span className="shrink-0 font-mono text-meta text-muted-foreground">
                              {node?.mesh_ipv4 ?? ""}
                            </span>
                            <ArrowUpRightIcon className="size-4 shrink-0 text-muted-foreground" />
                          </button>
                        </li>
                      )
                    })}
                  </ul>
                )}
              </TabSection>
              {canManage ? (
                <TabSection title={m.nodes_tag_removal()}>
                  <div className="flex min-h-12 items-center gap-3 rounded-tile bg-fill-subtle py-2 pr-2 pl-3.5">
                    <span className="min-w-0 flex-1 text-row">
                      {state === "removed"
                        ? m.nodes_tag_removing()
                        : nodeIds.length > 0
                          ? m.nodes_tag_assigned({
                              count: String(nodeIds.length),
                            })
                          : m.nodes_tag_unassigned()}
                    </span>
                    {state === "removed" ? (
                      <Button
                        type="button"
                        variant="ghost"
                        className="h-9 rounded-full px-4"
                        onClick={restore}
                      >
                        <RotateCcwIcon />
                        {m.nodes_tag_restore()}
                      </Button>
                    ) : (
                      <Button
                        type="button"
                        variant="destructive"
                        className="h-9 rounded-full px-4"
                        disabled={nodeIds.length > 0}
                        onClick={remove}
                      >
                        <Trash2Icon />
                        {m.nodes_tag_remove()}
                      </Button>
                    )}
                  </div>
                </TabSection>
              ) : null}
            </>
          )}
        </div>
      </form>
    </div>
  )
}

/** Names a new tag; it joins the catalog's draft and opens, unsaved, in its workspace. */
export function NewTagDialog({
  networkId,
  open,
  onOpenChange,
  onCreated,
}: {
  networkId: string
  open: boolean
  onOpenChange: (open: boolean) => void
  onCreated: (tag: string) => void
}) {
  const { saved, draft } = useCatalog(networkId)
  const [typed, setTyped] = useState("")
  const [description, setDescription] = useState("")
  const [attempted, setAttempted] = useState(false)
  const name = tagName(typed)
  const exists = (draft.value ?? []).some((entry) => entry.tag === name)
  const invalidName = !isTagName(name) || exists
  const invalidDescription = [...description].length > 256

  const close = (next: boolean) => {
    onOpenChange(next)
    if (!next) {
      setTyped("")
      setDescription("")
      setAttempted(false)
    }
  }
  const submit = () => {
    setAttempted(true)
    if (invalidName || invalidDescription || !saved) return
    draft.edit((current) => [...current, { tag: name, description }])
    onCreated(name)
    close(false)
  }

  return (
    <GlassDialog open={open} onOpenChange={close} title={m.nodes_tag_new()}>
      <form
        className="flex flex-col gap-4"
        onSubmit={(event) => {
          event.preventDefault()
          submit()
        }}
      >
        <Field data-invalid={(attempted && invalidName) || undefined}>
          <FieldLabel
            htmlFor="new-tag-name"
            className="text-meta text-muted-foreground"
          >
            {m.nodes_tag_name()}
          </FieldLabel>
          <InputGroup className="h-10 rounded-tile border-transparent bg-field">
            <InputGroupAddon className="pl-3.5 font-mono text-row text-muted-foreground">
              {TAG_PREFIX}
            </InputGroupAddon>
            <InputGroupInput
              id="new-tag-name"
              name="tag-name"
              autoComplete="off"
              autoCapitalize="none"
              spellCheck={false}
              maxLength={48}
              className="font-mono md:text-row"
              value={typed}
              aria-invalid={(attempted && invalidName) || undefined}
              onChange={(event) => setTyped(event.target.value)}
            />
          </InputGroup>
        </Field>
        <TextField
          id="new-tag-description"
          placeholder={m.nodes_tag_description_placeholder()}
          name="tag-description"
          label={m.nodes_tag_description()}
          autoComplete="off"
          maxLength={256}
          value={description}
          invalid={attempted && invalidDescription}
          onChange={(event) => setDescription(event.target.value)}
        />
        <div className="flex h-9 items-center justify-end gap-2">
          <span className="min-w-0 flex-1 truncate text-row text-warning">
            {attempted && invalidName
              ? exists
                ? m.nodes_tag_duplicate()
                : m.nodes_tag_name_invalid()
              : null}
          </span>
          <Button
            type="button"
            variant="ghost"
            className="h-9 rounded-full px-4"
            onClick={() => close(false)}
          >
            {m.dialog_cancel()}
          </Button>
          <Button type="submit" className="h-9 rounded-full px-4">
            {m.nodes_tag_add()}
          </Button>
        </div>
      </form>
    </GlassDialog>
  )
}
