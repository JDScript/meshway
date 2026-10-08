import type { Lang } from "@/lib/i18n"

/** The landing page (product, docs and sign-in), in `lang`. */
export function Landing({ lang }: { lang: Lang }) {
  return (
    <main className="flex flex-1 items-center justify-center p-8">
      <h1 className="text-display font-semibold">
        {lang === "zh" ? "Meshway" : "Meshway"}
      </h1>
    </main>
  )
}
