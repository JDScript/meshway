import { LanguagesIcon, MoonIcon, SunIcon } from "lucide-react"

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
import { isLocale, locales } from "@/paraglide/runtime.js"
import {
  switchLocale,
  toggleTheme,
  useDark,
  useLocale,
} from "@/state/preferences"

export function ThemeSwitch() {
  const dark = useDark()
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            variant="ghost"
            size="icon"
            className="bar-button"
            aria-label={m.theme_switch()}
            onClick={toggleTheme}
          >
            {dark ? <SunIcon /> : <MoonIcon />}
          </Button>
        }
      />
      <TooltipContent>{m.theme_switch()}</TooltipContent>
    </Tooltip>
  )
}

/** `disabled` keeps the control in its place while it cannot be used (a secret is held that a reload would lose). */
export function LanguageSwitch({ disabled = false }: { disabled?: boolean }) {
  const locale = useLocale()
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        disabled={disabled}
        render={
          <Button
            variant="ghost"
            size="icon"
            className="bar-button"
            aria-label={m.language_switch()}
            disabled={disabled}
          >
            <LanguagesIcon />
          </Button>
        }
      />
      <DropdownMenuContent align="end">
        <DropdownMenuRadioGroup
          value={locale}
          onValueChange={(value) => {
            if (typeof value === "string" && isLocale(value))
              switchLocale(value)
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
