import { redirect } from "next/navigation"

/** First-Owner setup moved to the instance setup page. */
export default function OnboardingPage(): never {
  redirect("/setup")
}
