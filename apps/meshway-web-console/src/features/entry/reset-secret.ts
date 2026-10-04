import { heldSecret } from "./fragment"

/** The reset link's secret, taken by `routes/reset-password.tsx` and read by its page. */
export const heldReset = heldSecret()
