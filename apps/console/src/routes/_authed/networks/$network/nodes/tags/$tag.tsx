import { createFileRoute, redirect } from "@tanstack/react-router"

import { isTagName } from "@/features/nodes/tag-model"

/** One tag's workspace over the tags. A name the schema refuses is not a tag: back to the tags. */
export const Route = createFileRoute(
  "/_authed/networks/$network/nodes/tags/$tag"
)({
  staticData: { workspace: true },
  beforeLoad: ({ params }) => {
    if (!isTagName(params.tag))
      throw redirect({
        to: "/networks/$network/nodes",
        params: { network: params.network },
        search: { view: "tags" },
        replace: true,
      })
  },
})
