import { LanguagesIcon } from "lucide-react"
import { useId } from "react"

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
  isTheme,
  setTheme,
  switchLocale,
  THEMES,
  toggleTheme,
  useDark,
  useLocale,
  useTheme,
  type Theme,
} from "@/state/preferences"
import { useSkyPlaced } from "@/state/sky"

export function ThemeSwitch() {
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
            <ThemeIcon />
          </Button>
        }
      />
      <TooltipContent>{m.theme_switch()}</TooltipContent>
    </Tooltip>
  )
}

/**
 * What shows now, on a horizon: the sun by light, the moon by dark. A
 * change sets the one below the horizon as the other rises.
 */
function ThemeIcon() {
  const dark = useDark()
  const sky = useId()
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      data-dark={dark}
      className="theme-icon size-4"
    >
      <defs>
        <clipPath id={sky}>
          <rect x="-2" y="-2" width="28" height="24" />
        </clipPath>
      </defs>
      <g clipPath={`url(#${CSS.escape(sky)})`}>
        <g className="theme-icon__sun">
          <circle cx="12" cy="12" r="4" />
          <path d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M6.34 17.66l-1.41 1.41M19.07 4.93l-1.41 1.41" />
        </g>
        <path
          className="theme-icon__moon"
          d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"
        />
      </g>
    </svg>
  )
}

const THEME_LABEL: Record<Theme, () => string> = {
  sun: () => m.theme_sun(),
  system: () => m.theme_system(),
  light: () => m.theme_light(),
  dark: () => m.theme_dark(),
}

/** The theme, chosen among following the sun, the system, light and dark (for a menu). */
export function ThemeChoice() {
  const theme = useTheme()
  const placed = useSkyPlaced()
  return (
    <DropdownMenuRadioGroup
      value={theme}
      onValueChange={(value) => {
        if (isTheme(value)) setTheme(value)
      }}
    >
      {THEMES.map((choice) => (
        <DropdownMenuRadioItem key={choice} value={choice}>
          {THEME_LABEL[choice]()}
          {choice === "sun" && !placed ? (
            <span className="ml-auto pl-3 text-meta text-muted-foreground">
              {m.theme_sun_unplaced()}
            </span>
          ) : null}
        </DropdownMenuRadioItem>
      ))}
    </DropdownMenuRadioGroup>
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
