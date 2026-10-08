import type { Lang } from "@/lib/i18n"
import { getReference, operationDocument } from "@/lib/openapi"
import { codeSamples } from "@/lib/openapi-samples"

import { APIReference } from "./page"

export async function OperationReference({
  operationId,
  lang,
}: {
  operationId: string
  lang: Lang
}) {
  const { document, operations } = await getReference()
  const entry = operations.find(
    (entry) => entry.operation.operationId === operationId
  )
  if (!entry) throw new Error(`Unknown reference operation: ${operationId}`)
  const samples = codeSamples(document, entry)
  const bundled = operationDocument(document, {
    ...entry,
    operation: {
      ...entry.operation,
      "x-codeSamples": [
        { id: "curl", lang: "bash", label: "curl", source: samples.curl },
        {
          id: "typescript",
          lang: "typescript",
          label: "TypeScript",
          source: samples.typescript,
        },
      ],
    },
  })
  return (
    <APIReference
      lang={lang}
      payload={{ bundled }}
      operations={[{ path: entry.path, method: entry.method }]}
      showTitle={false}
      showDescription
    />
  )
}
