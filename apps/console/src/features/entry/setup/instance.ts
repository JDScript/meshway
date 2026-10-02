import {
  ControlApiError,
  listAccountMembers,
  listAccountPasswordResets,
  listNetworks,
  type AccountMember,
  type AccountPasswordReset,
  type AccountRole,
  type NetworkSnapshot,
} from "@meshway/control-client"

import { apiBase, SessionRejected } from "@/lib/api"

// The instance administrator token: setup and recovery only. It is kept in
// the setup page's memory (component state), never stored, never in the
// query cache, and dropped when the page is left or the controller refuses
// it. Network roles never reach any of this.

type Operation<T> = (base: string, token: string) => Promise<T>

/** Calls a generated operation with the instance token; a 401 drops the token. */
export type InstanceCall = {
  <T>(call: Operation<T>): Promise<T>
  /** Keeps the token on a 401: the refusal may concern another credential in the request. */
  unchecked: <T>(call: Operation<T>) => Promise<T>
}

export function instanceCall(
  token: string,
  onRejected: () => void
): InstanceCall {
  const checked = async <T>(call: Operation<T>): Promise<T> => {
    try {
      return await call(apiBase(), token)
    } catch (error) {
      if (error instanceof ControlApiError && error.status === 401) {
        onRejected()
        throw new SessionRejected()
      }
      throw error
    }
  }
  return Object.assign(checked, {
    unchecked: <T>(call: Operation<T>) => call(apiBase(), token),
  })
}

export type SetupNetwork = {
  network: NetworkSnapshot
  /** null when the members could not be read. */
  members: AccountMember[] | null
}

export type SetupAccount = {
  userId: string
  email: string
  networks: { name: string; role: AccountRole }[]
}

export type Instance = {
  networks: SetupNetwork[]
  /** Accounts are global, but listed per network: an account appears once it belongs to one. */
  accounts: SetupAccount[]
  resets: AccountPasswordReset[]
  /** When this was read, in Unix seconds (the controller's clock unit). */
  readAt: number
}

/** Everything the setup shows. The reset inventory needs the instance token itself, not a user's. */
export async function readInstance(call: InstanceCall): Promise<Instance> {
  const [listed, resets] = await Promise.all([
    call(listNetworks),
    call(listAccountPasswordResets),
  ])
  const networks = await Promise.all(
    listed.networks.map(async (network): Promise<SetupNetwork> => {
      try {
        const { members } = await call((base, token) =>
          listAccountMembers(base, token, network.network_id)
        )
        return { network, members }
      } catch (error) {
        if (error instanceof SessionRejected) throw error
        return { network, members: null }
      }
    })
  )
  const accounts = new Map<string, SetupAccount>()
  for (const { network, members } of networks) {
    for (const { user, membership } of members ?? []) {
      const account = accounts.get(user.user_id) ?? {
        userId: user.user_id,
        email: user.email,
        networks: [],
      }
      account.networks.push({ name: network.name, role: membership.role })
      accounts.set(user.user_id, account)
    }
  }
  return {
    networks,
    accounts: [...accounts.values()].sort((a, b) =>
      a.email.localeCompare(b.email)
    ),
    resets: [...resets.resets].sort((a, b) => b.expires_at - a.expires_at),
    readAt: Math.floor(Date.now() / 1000),
  }
}

export type ResetState = "valid" | "used" | "revoked" | "expired" | "stale"

export function resetState(
  reset: AccountPasswordReset,
  now: number
): ResetState {
  if (reset.consumed_at !== null) return "used"
  if (reset.revoked_at !== null) return "revoked"
  if (reset.expires_at <= now) return "expired"
  if (!reset.credential_current) return "stale"
  return "valid"
}

export const hasOwner = ({ members }: SetupNetwork) =>
  members?.some(({ membership }) => membership.role === "owner") ?? false
