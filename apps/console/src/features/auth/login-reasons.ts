// Why the sign-in page was opened, carried in its search (`/login?reason=`)
// and announced there once as a toast.

export const LOGIN_REASONS = [
  "expired",
  "signed_out",
  "signed_out_unconfirmed",
  "signed_out_all",
  "password_changed",
  "password_reset",
] as const

export type LoginReason = (typeof LOGIN_REASONS)[number]
