const SAFE_WORD = /^[A-Za-z0-9_@%+=:,./-]+$/

/** POSIX shell quoting for a single argument. */
export function shellQuote(value: string): string {
  if (SAFE_WORD.test(value)) return value
  return `'${value.replaceAll("'", `'\\''`)}'`
}

export function shellCommand(args: string[]): string {
  return args.map(shellQuote).join(" ")
}
