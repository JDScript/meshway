import { notFound, redirect } from "next/navigation"

import { consoleRoutes, isNetworkId } from "@/lib/routes"

/** Invitations are created on the network's members page now. */
export default async function NewInvitationPage({ searchParams }: { searchParams: Promise<Record<string, string | string[] | undefined>> }): Promise<never> {
  const { network } = await searchParams
  if (typeof network !== "string" || !isNetworkId(network)) notFound()
  redirect(`${consoleRoutes(network).members}?invite=1`)
}
