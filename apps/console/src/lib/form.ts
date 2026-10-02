/** A text field's value from a submitted form ("" when absent). */
export function formText(form: FormData, name: string): string {
  const value = form.get(name)
  return typeof value === "string" ? value : ""
}

/** An address of the form the controller accepts: one `@`, no spaces, at most 254 characters. */
export function validEmail(email: string): boolean {
  return email.length <= 254 && /^[^\s@]+@[^\s@]+$/.test(email)
}
