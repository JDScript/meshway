import { useEffect, useRef } from "react"
import { toast } from "sonner"

/** Announces a failing read once per outage, as a toast: nothing on the page changes size. */
export function useReadFailureToast(
  id: string,
  failing: boolean,
  message: string
) {
  const announced = useRef(false)
  useEffect(() => {
    if (failing && !announced.current) {
      announced.current = true
      toast.error(message, { id: `read-${id}` })
    }
    if (!failing) announced.current = false
  }, [failing, id, message])
}
