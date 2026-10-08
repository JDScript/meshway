import { z } from "zod"

import { m } from "@/paraglide/messages.js"

// A site asking to act for the signed-in account (delegated sign-in:
// authorization code with PKCE). The site sends people here with what the
// server needs to issue a code; the console asks for consent, has the
// server issue the code, and hands it back on the site's own address. The
// server checks the client and its address; nothing here decides them.

/** The request in the address, as the site sends it (OAuth's names). */
export const zSiteAuthorization = z.object({
  client: z
    .string()
    .regex(/^[a-z0-9][a-z0-9-]{0,31}$/)
    .optional()
    .catch(undefined),
  redirect_uri: z.string().max(2048).optional().catch(undefined),
  code_challenge: z
    .string()
    .regex(/^[A-Za-z0-9_-]{43}$/)
    .optional()
    .catch(undefined),
  code_challenge_method: z.literal("S256").optional().catch(undefined),
  state: z
    .string()
    .regex(/^[\x21-\x7e]{1,512}$/)
    .optional()
    .catch(undefined),
})

export type SiteAuthorization = {
  client: string
  redirectUri: URL
  codeChallenge: string
  state: string
}

/** The request when complete and well formed (a web address the site returns to), else null. */
export function siteAuthorization(
  search: z.output<typeof zSiteAuthorization>
): SiteAuthorization | null {
  const { client, redirect_uri, code_challenge, code_challenge_method, state } =
    search
  if (!client || !redirect_uri || !code_challenge || !state) return null
  if (code_challenge_method !== "S256") return null
  let redirectUri: URL
  try {
    redirectUri = new URL(redirect_uri)
  } catch {
    return null
  }
  if (redirectUri.protocol !== "https:" && redirectUri.protocol !== "http:")
    return null
  if (redirectUri.username || redirectUri.password || redirectUri.hash)
    return null
  return { client, redirectUri, codeChallenge: code_challenge, state }
}

/** Where the code goes: the site's address with `code` and its own `state`. */
export function returnAddress(
  request: SiteAuthorization,
  code: string
): string {
  const target = new URL(request.redirectUri)
  target.searchParams.set("code", code)
  target.searchParams.set("state", request.state)
  return target.href
}

/** The clients this console knows by name; another is shown as the server names it. */
const CLIENT_NAMES: Record<string, () => string> = {
  docs: () => m.site_client_docs(),
}

export const delegationClientName = (client: string): string =>
  (Object.hasOwn(CLIENT_NAMES, client)
    ? CLIENT_NAMES[client]
    : undefined)?.() ?? client
