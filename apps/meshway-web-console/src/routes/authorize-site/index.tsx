import { createFileRoute } from "@tanstack/react-router"

import { EntryPage } from "@/components/entry/frame"
import { siteAuthorization, zSiteAuthorization } from "@/lib/delegation"

import { SiteCard } from "./-site-card"

/**
 * `/authorize-site?client=&redirect_uri=&code_challenge=&code_challenge_method=S256&state=`,
 * where a site sends people to let it act for their account. Nothing in the
 * address is a credential: the code the server issues on consent can only
 * be exchanged with the verifier the site kept.
 */
export const Route = createFileRoute("/authorize-site/")({
  validateSearch: zSiteAuthorization,
  component: AuthorizeSitePage,
})

function AuthorizeSitePage() {
  const search = Route.useSearch()
  return (
    <EntryPage>
      <SiteCard request={siteAuthorization(search)} />
    </EntryPage>
  )
}
