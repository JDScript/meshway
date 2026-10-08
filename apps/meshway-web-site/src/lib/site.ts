/** The console the site sends people to (sign-in), set per deployment at build time. */
export const consoleUrl = (() => {
  const url = process.env.NEXT_PUBLIC_MESHWAY_CONSOLE_URL
  if (!url) throw new Error("NEXT_PUBLIC_MESHWAY_CONSOLE_URL is not set")
  return url.replace(/\/$/, "")
})()

export const appName = "Meshway"
