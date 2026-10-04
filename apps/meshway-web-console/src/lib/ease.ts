// From AgentUI (https://www.agentui.pro, registry source `lib/ease.ts`),
// keeping only the spring this project uses. CSS easings live in index.css
// (`--ease-settle`, `--ease-entrance`).

/** Shared-layout glides — pills, indicators and panels morphing between positions. */
export const SPRING_LAYOUT = {
  type: "spring",
  stiffness: 360,
  damping: 32,
  mass: 0.6,
} as const
