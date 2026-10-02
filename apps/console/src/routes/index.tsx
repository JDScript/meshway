import { createFileRoute } from "@tanstack/react-router"

import { Button } from "@/components/ui/button"
import { m } from "@/paraglide/messages.js"
import { getLocale, locales, setLocale } from "@/paraglide/runtime.js"

export const Route = createFileRoute("/")({
  component: Scaffold,
})

function Scaffold() {
  const current = getLocale()
  return (
    <main className="flex min-h-svh flex-col items-start justify-center gap-4 p-10">
      <h1 className="text-2xl font-semibold">{m.scaffold_title()}</h1>
      <p className="text-muted-foreground">{m.scaffold_body()}</p>
      <div className="flex gap-2">
        {locales.map((locale) => (
          <Button
            key={locale}
            variant={locale === current ? "default" : "outline"}
            onClick={() => setLocale(locale)}
          >
            {m.language_name({}, { locale })}
          </Button>
        ))}
      </div>
    </main>
  )
}
