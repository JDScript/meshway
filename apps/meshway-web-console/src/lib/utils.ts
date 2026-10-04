import { createCn } from "cn/config"

/**
 * The theme's own scales (`@theme` in index.css). Class merging must know
 * them: unknown, `text-meta` would count as a colour and lose to
 * `text-muted-foreground`. `styles.test.ts` keeps this list in step.
 */
export const THEME_SCALES = {
  text: ["meta", "row", "lead", "display"],
  radius: ["surface", "sheet", "row", "tile"],
} as const

/** Joins and merges class names, knowing the theme's scales. Import it from here, never from `cn` itself. */
export const cn = createCn({
  extend: {
    classGroups: {
      "font-size": [{ text: [...THEME_SCALES.text] }],
      rounded: [{ rounded: [...THEME_SCALES.radius] }],
    },
  },
})
