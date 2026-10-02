import { heldSecret } from "./fragment"

/** Read by `/invite`; kept across a sign-in in this tab, never stored. */
export const heldInvitation = heldSecret("invitation")

/** Read by `/reset-password`. */
export const heldReset = heldSecret("reset")
