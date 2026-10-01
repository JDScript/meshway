import "server-only"

export function controllerUrl(): string {
  return process.env.MESHWAY_CONTROLLER_URL ?? "http://127.0.0.1:8420"
}
