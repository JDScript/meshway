"use client"

import { useComponents, useOpenAPI } from "fumadocs-openapi"
import { useOperation } from "fumadocs-openapi/operation"
import { createCodeUsageGeneratorRegistry } from "fumadocs-openapi/requests/generators"
import { createOpenAPIPage, type OpenAPIPageProps } from "fumadocs-openapi/ui"
import { I18nProvider, useI18n } from "fumadocs-ui/contexts/i18n"

import type { Lang } from "@/lib/i18n"
import { apiSettings } from "@/lib/openapi-options"
import { apiChinese } from "@/lib/openapi-translations"

function EventSchema() {
  const { operation } = useOperation()
  const { doc } = useOpenAPI()
  const { SchemaUI, Markdown, CodeBlock } = useComponents()
  const { locale } = useI18n()
  const extension: unknown =
    "x-meshway-sse" in operation ? operation["x-meshway-sse"] : undefined
  if (
    !extension ||
    typeof extension !== "object" ||
    !("eventSchema" in extension)
  )
    return null
  const reference = extension.eventSchema
  if (
    !reference ||
    typeof reference !== "object" ||
    !("$ref" in reference) ||
    typeof reference.$ref !== "string"
  )
    return null
  const schema =
    doc.dereferenced.components?.schemas?.[
      reference.$ref.replace("#/components/schemas/", "")
    ]
  if (schema === undefined) throw new Error("Missing SSE event schema")
  return (
    <section className="mt-10" aria-labelledby="event-schema">
      <h2 id="event-schema">{locale === "zh" ? "事件数据" : "Event data"}</h2>
      <p className="text-fd-muted-foreground">
        {locale === "zh"
          ? "每个 SSE message 的 data 按 UserEvent 解码。只有完整批次或事件已应用后才保存 cursor；重连时通过 Last-Event-ID 续传。"
          : "Decode each SSE message’s data as UserEvent. Save a cursor only after applying a complete batch or event; resume with Last-Event-ID."}
      </p>
      <SchemaUI
        root={doc.resolve(schema)}
        readOnly
        renderMarkdown={(md) => <Markdown md={md} />}
        renderCodeblock={(props) => <CodeBlock {...props} />}
        client={{
          name: "UserEvent",
          as: "body",
          rootId: "event-data",
          required: true,
        }}
      />
    </section>
  )
}

const APIPage = createOpenAPIPage({
  ...apiSettings,
  codeUsages: createCodeUsageGeneratorRegistry(),
  generateTypeScriptDefinitions: false,
  showResponseSchema: true,
  content: {
    // Synthetic response examples imply values and guarantees absent from the contract.
    renderAPIExampleLayout: ({ usageTabs }) => usageTabs,
    renderOperationLayout: (slots) => (
      <div className="meshway-api-layout">
        <div className="meshway-api-intro min-w-0">
          {slots.apiPlayground}
          {slots.description}
          <FieldGuide />
        </div>
        <div className="meshway-api-fields min-w-0">
          {slots.authSchemes}
          {slots.parameters}
          {slots.body}
          {slots.responses}
          <EventSchema />
        </div>
        <aside className="meshway-api-samples">
          <SampleHeading />
          {slots.apiExample}
        </aside>
      </div>
    ),
  },
})

function FieldGuide() {
  const { locale } = useI18n()
  return (
    <p className="mt-4 text-xs text-fd-muted-foreground">
      {locale === "zh"
        ? "* 必填 · ? 可省略 · null 表示可空。点击带下划线的类型可展开嵌套字段或联合分支。"
        : "* Required · ? Optional · null means nullable. Select an underlined type to explore nested fields or union variants."}
    </p>
  )
}

function SampleHeading() {
  const { locale } = useI18n()
  const { requestBody } = useOperation()
  return (
    <>
      <h2 className="mt-0! text-base!">
        {locale === "zh" ? "调用示例" : "Code samples"}
      </h2>
      <p className="mb-4 text-xs leading-relaxed text-fd-muted-foreground">
        {locale === "zh"
          ? "将环境变量设为你自己服务端的 origin 和会话 token；替换尖括号中的路径值。TypeScript 示例在 Node.js 中运行。"
          : "Set the environment variables to your server origin and session token; replace path values in angle brackets. TypeScript samples run in Node.js."}
        {requestBody &&
          (locale === "zh"
            ? " 按请求字段准备 request.json；SDK 在发送前校验内容。"
            : " Prepare request.json using the request fields; the SDK validates it before sending.")}
      </p>
    </>
  )
}

export function APIReference({
  lang,
  ...props
}: OpenAPIPageProps & { lang: Lang }) {
  return (
    <I18nProvider locale={lang} translations={lang === "zh" ? apiChinese : {}}>
      <div className="meshway-api not-prose">
        <APIPage {...props} />
      </div>
    </I18nProvider>
  )
}
