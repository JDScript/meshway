import { LanguagesIcon, MoonIcon, SunIcon } from "lucide-react"

import { useTheme } from "@/components/theme-provider"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { m } from "@/paraglide/messages.js"
import { getLocale, isLocale, locales, setLocale } from "@/paraglide/runtime.js"

export function ThemeSwitch() {
  const { theme, setTheme } = useTheme()
  const dark =
    theme === "dark" ||
    (theme === "system" &&
      window.matchMedia("(prefers-color-scheme: dark)").matches)
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            variant="ghost"
            size="icon"
            className="bar-button"
            aria-label={m.theme_switch()}
            onClick={() => setTheme(dark ? "light" : "dark")}
          >
            {dark ? <SunIcon /> : <MoonIcon />}
          </Button>
        }
      />
      <TooltipContent>{m.theme_switch()}</TooltipContent>
    </Tooltip>
  )
}

export function LanguageSwitch() {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button
            variant="ghost"
            size="icon"
            className="bar-button"
            aria-label={m.language_switch()}
          >
            <LanguagesIcon />
          </Button>
        }
      />
      <DropdownMenuContent align="end">
        <DropdownMenuRadioGroup
          value={getLocale()}
          onValueChange={(value) => {
            if (
              typeof value === "string" &&
              isLocale(value) &&
              value !== getLocale()
            )
              setLocale(value)
          }}
        >
          {locales.map((locale) => (
            <DropdownMenuRadioItem key={locale} value={locale}>
              {m.language_name({}, { locale })}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
