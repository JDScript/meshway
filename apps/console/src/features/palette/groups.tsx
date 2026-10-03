import { useQuery } from "@tanstack/react-query"
import { useNavigate } from "@tanstack/react-router"
import {
  MoonIcon,
  NetworkIcon,
  PlusIcon,
  SunIcon,
  TagIcon,
  UserPlusIcon,
  UserRoundIcon,
  type LucideIcon,
} from "lucide-react"
import type { ReactNode } from "react"

import { InitialAvatar } from "@/components/chrome/avatar"
import { useThemeToggle } from "@/components/chrome/preferences"
import { roleLabel } from "@/features/members/roles"
import {
  membersQuery,
  networksQuery,
  tagsQuery,
} from "@/features/network/queries"
import { useNetworkRole } from "@/features/network/role"
import { useNetwork } from "@/features/network/use-network-view"
import { RowTags } from "@/features/nodes/node-list"
import { vitalityLabel } from "@/features/overview/labels"
import { vitalityOf } from "@/features/overview/model"
import { VitalityDot } from "@/features/overview/status-dots"
import { requestRuleFocus } from "@/features/policy/navigation"
import { layerIcon, layerLabel } from "@/features/policy/policy-panel"
import { policyQuery } from "@/features/policy/queries"
import { POLICY_LAYERS, type PolicyLayer } from "@/features/policy/url"
import { sectionIcon, sectionLabel } from "@/features/settings/settings-panel"
import { SETTINGS_SECTIONS } from "@/features/settings/url"
import { useSurfaceHistory } from "@/features/shell/history"
import { DOCK, type PageId } from "@/features/shell/pages"
import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"

// The palette's groups: every page, node, tag, member and rule of the
// network, the other networks and a few actions, built from the reads the
// pages already make. Only what this sign-in can use is offered.

export type Entry = {
  /** Unique within the palette (cmdk's value). */
  value: string
  /** The name first, then what else finds it. */
  keywords: readonly string[]
  lead: ReactNode
  label: ReactNode
  detail?: ReactNode
  /** Offered only once something is typed (a layer, a section: their page leads to them). */
  searchOnly?: boolean
  /** Leaves for another surface: focus is not handed back where it was. */
  navigates: boolean
  run: () => void
}

export type Group = {
  key: string
  heading: string
  /** Unread or unreadable: shown as such, in place of entries. */
  status: "ready" | "loading" | "failed"
  entries: readonly Entry[]
  /** Bounded with the rest counted; the pages are few and always shown. */
  bounded: boolean
}

const statusOf = (query: { data: unknown; isError: boolean }) =>
  query.data !== undefined ? "ready" : query.isError ? "failed" : "loading"

const glyph = (Icon: LucideIcon) => (
  <Icon aria-hidden="true" className="size-4 text-muted-foreground" />
)

const name = (text: string, mono = false) => (
  <span className={cn("min-w-0 truncate", mono && "font-mono")}>{text}</span>
)

/** The groups, from the cached reads; the ones only the palette needs are read while it is open. */
export function usePaletteGroups({
  networkId,
  page,
  mobile,
  open,
  onSwitchNetwork,
  onInvite,
}: {
  networkId: string
  page: PageId
  mobile: boolean
  open: boolean
  onSwitchNetwork: (networkId: string) => void
  onInvite: (() => void) | null
}): Group[] {
  const navigate = useNavigate()
  const history = useSurfaceHistory(networkId)
  const view = useNetwork()
  const role = useNetworkRole(networkId)
  const canManage = role === "owner" || role === "admin"
  const theme = useThemeToggle()
  const networks = useQuery(networksQuery())
  const tags = useQuery({ ...tagsQuery(networkId), enabled: open })
  const members = useQuery({
    ...membersQuery(networkId),
    enabled: open && role !== null,
  })
  const policy = useQuery({ ...policyQuery(networkId), enabled: open })

  const network = { network: networkId }
  const jump = (workspace: boolean) =>
    history.jump({ mobile, fromOverview: page === "overview", workspace })
  // Pages, with the policy's layers and the settings' sections.
  const dock = (id: PageId) => DOCK.find((item) => item.page === id)
  const pageEntry = (
    id: string,
    icon: LucideIcon,
    label: string,
    run: () => void,
    parent?: string
  ): Entry => ({
    value: `page:${id}`,
    keywords: parent ? [label, parent] : [label],
    searchOnly: parent !== undefined,
    lead: glyph(icon),
    label: name(label),
    detail: parent,
    navigates: true,
    run,
  })
  const pages: Entry[] = []
  const overview = dock("overview")
  if (overview)
    pages.push(
      pageEntry("overview", overview.icon, overview.label(), () => {
        // On phones the overview is where the pages are left for, as the launcher leaves them.
        if (page === "overview") return
        if (mobile) history.leavePage()
        else
          void navigate({
            to: "/networks/$network/overview",
            params: network,
            ...jump(false),
          })
      })
    )
  const nodesPage = dock("nodes")
  if (nodesPage) {
    pages.push(
      pageEntry(
        "nodes",
        nodesPage.icon,
        nodesPage.label(),
        () =>
          void navigate({
            to: "/networks/$network/nodes",
            params: network,
            ...jump(false),
          })
      ),
      pageEntry(
        "tags",
        TagIcon,
        m.nodes_view_tags(),
        () =>
          void navigate({
            to: "/networks/$network/nodes",
            params: network,
            search: { view: "tags" },
            ...jump(false),
          })
      )
    )
  }
  const membersPage = dock("members")
  if (membersPage)
    pages.push(
      pageEntry(
        "members",
        membersPage.icon,
        membersPage.label(),
        () =>
          void navigate({
            to: "/networks/$network/members",
            params: network,
            ...jump(false),
          })
      )
    )
  const policyPage = dock("policy")
  if (policyPage) {
    pages.push(
      pageEntry(
        "policy",
        policyPage.icon,
        policyPage.label(),
        () =>
          void navigate({
            to: "/networks/$network/policy",
            params: network,
            ...jump(false),
          })
      )
    )
    for (const layer of POLICY_LAYERS)
      pages.push(
        pageEntry(
          `policy/${layer}`,
          layerIcon[layer],
          layerLabel[layer](),
          () =>
            void navigate({
              to: "/networks/$network/policy/$layer",
              params: { ...network, layer },
              ...jump(true),
            }),
          policyPage.label()
        )
      )
  }
  const settingsPage = dock("settings")
  if (settingsPage) {
    pages.push(
      pageEntry(
        "settings",
        settingsPage.icon,
        settingsPage.label(),
        () =>
          void navigate({
            to: "/networks/$network/settings",
            params: network,
            ...jump(false),
          })
      )
    )
    for (const section of SETTINGS_SECTIONS)
      pages.push(
        pageEntry(
          `settings/${section}`,
          sectionIcon[section],
          sectionLabel[section](),
          () =>
            void navigate({
              to: "/networks/$network/settings/$section",
              params: { ...network, section },
              ...jump(true),
            }),
          settingsPage.label()
        )
      )
  }
  pages.push(
    pageEntry(
      "account",
      UserRoundIcon,
      m.account_title(),
      () =>
        void navigate({
          to: "/networks/$network/account",
          params: network,
          ...jump(true),
        })
    )
  )

  // Nodes, as the nodes list shows them.
  const tagsByNode = new Map(
    (tags.data?.assignments ?? []).map((assignment) => [
      assignment.node_id,
      assignment.tags.map((tag) => tag.replace(/^tag:/, "")).sort(),
    ])
  )
  const nodes: Entry[] = view.inventory
    .filter((node) => node.revoked_at === null)
    .sort((a, b) => a.name.localeCompare(b.name) || a.id.localeCompare(b.id))
    .map((node) => {
      const nodeTags = tagsByNode.get(node.id) ?? []
      const vitality = vitalityOf(node, view.context.runtimes, view.now)
      return {
        value: `node:${node.id}`,
        keywords: [
          node.name,
          node.mesh_ipv4,
          node.mesh_ipv6,
          ...nodeTags,
          node.id,
        ],
        lead: (
          <span
            role="img"
            aria-label={vitalityLabel[vitality]()}
            className="flex"
          >
            <VitalityDot vitality={vitality} />
          </span>
        ),
        label: (
          <>
            {name(node.name)}
            <span className="flex min-w-0 text-meta text-muted-foreground">
              <RowTags tags={nodeTags} />
            </span>
          </>
        ),
        detail: <span className="font-mono">{node.mesh_ipv4}</span>,
        navigates: true,
        run: () =>
          void navigate({
            to: "/networks/$network/nodes/$node",
            params: { ...network, node: node.id },
            ...jump(true),
          }),
      }
    })

  const tagMembers = new Map<string, number>()
  for (const assignment of tags.data?.assignments ?? [])
    for (const tag of assignment.tags)
      tagMembers.set(tag, (tagMembers.get(tag) ?? 0) + 1)
  const tagEntries: Entry[] = [...(tags.data?.tags ?? [])]
    .sort((a, b) => a.tag.localeCompare(b.tag))
    .map((tag) => ({
      value: `tag:${tag.tag}`,
      keywords: [tag.tag.replace(/^tag:/, ""), tag.tag, tag.description],
      lead: glyph(TagIcon),
      label: name(tag.tag, true),
      detail: m.nodes_tag_members_count({
        count: String(tagMembers.get(tag.tag) ?? 0),
      }),
      navigates: true,
      run: () =>
        void navigate({
          to: "/networks/$network/nodes/tags/$tag",
          params: { ...network, tag: tag.tag },
          search: { view: "tags" },
          ...jump(true),
        }),
    }))

  const memberEntries: Entry[] = [...(members.data?.members ?? [])]
    .sort((a, b) => a.user.email.localeCompare(b.user.email))
    .map(({ user, membership }) => ({
      value: `member:${user.user_id}`,
      keywords: [user.email, roleLabel[membership.role]()],
      lead: <InitialAvatar email={user.email} />,
      label: name(user.email),
      detail: roleLabel[membership.role](),
      navigates: true,
      run: () =>
        void navigate({
          to: "/networks/$network/members/$member",
          params: { ...network, member: user.user_id },
          ...jump(true),
        }),
    }))

  const saved = policy.data?.policy
  const ruleEntries: Entry[] = saved
    ? (
        [
          ["connections", saved.connections],
          ["grants", saved.grants],
          ["mesh", saved.mesh],
        ] as const satisfies readonly (readonly [
          PolicyLayer,
          readonly { id: string }[],
        ])[]
      ).flatMap(([layer, rules]) =>
        rules.map((rule) => ({
          value: `rule:${layer}:${rule.id}`,
          keywords: [rule.id, layerLabel[layer]()],
          lead: glyph(layerIcon[layer]),
          label: name(rule.id, true),
          detail: layerLabel[layer](),
          navigates: true,
          run: () => {
            requestRuleFocus(rule.id)
            void navigate({
              to: "/networks/$network/policy/$layer",
              params: { ...network, layer },
              search: { rule: rule.id },
              ...jump(true),
            })
          },
        }))
      )
    : []

  const networkEntries: Entry[] = (networks.data?.networks ?? [])
    .filter((other) => other.network_id !== networkId)
    .map((other) => ({
      value: `network:${other.network_id}`,
      keywords: [other.name, other.network.ipv4_prefix],
      lead: glyph(NetworkIcon),
      label: name(other.name),
      detail: <span className="font-mono">{other.network.ipv4_prefix}</span>,
      navigates: true,
      run: () => onSwitchNetwork(other.network_id),
    }))

  const actions: Entry[] = []
  if (canManage)
    actions.push({
      value: "action:add-node",
      keywords: [m.nodes_add_title()],
      lead: glyph(PlusIcon),
      label: name(m.nodes_add_title()),
      navigates: true,
      run: () =>
        void navigate({
          to: "/networks/$network/nodes/add",
          params: network,
          ...jump(true),
        }),
    })
  if (onInvite)
    actions.push({
      value: "action:invite",
      keywords: [m.palette_invite()],
      lead: glyph(UserPlusIcon),
      label: name(m.palette_invite()),
      navigates: true,
      run: onInvite,
    })
  actions.push({
    value: "action:theme",
    keywords: [m.theme_switch(), ...m.palette_theme_keywords().split(" ")],
    lead: glyph(theme.dark ? SunIcon : MoonIcon),
    label: name(m.theme_switch()),
    navigates: false,
    run: theme.toggle,
  })

  const groups: Group[] = [
    {
      key: "pages",
      heading: m.palette_group_pages(),
      status: "ready",
      entries: pages,
      bounded: false,
    },
    {
      key: "nodes",
      heading: m.palette_group_nodes(),
      status: statusOf(view.nodes),
      entries: nodes,
      bounded: true,
    },
    {
      key: "tags",
      heading: m.palette_group_tags(),
      status: statusOf(tags),
      entries: tagEntries,
      bounded: true,
    },
  ]
  // Every role reads the members, as the members page does; unknown until the session is read.
  if (role !== null)
    groups.push({
      key: "members",
      heading: m.palette_group_members(),
      status: statusOf(members),
      entries: memberEntries,
      bounded: true,
    })
  groups.push(
    {
      key: "rules",
      heading: m.palette_group_rules(),
      status: statusOf(policy),
      entries: ruleEntries,
      bounded: true,
    },
    {
      key: "networks",
      heading: m.palette_group_networks(),
      status: statusOf(networks),
      entries: networkEntries,
      bounded: true,
    },
    {
      key: "actions",
      heading: m.palette_group_actions(),
      status: "ready",
      entries: actions,
      bounded: false,
    }
  )
  return groups
}
