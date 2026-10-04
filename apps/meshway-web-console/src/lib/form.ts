/** A text field's value from a submitted form ("" when absent). */
export function formText(form: FormData, name: string): string {
  const value = form.get(name)
  return typeof value === "string" ? value : ""
}
