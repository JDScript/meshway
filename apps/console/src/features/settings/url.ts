// The settings page in the address:
//
//   /networks/<network>/settings             the sections
//   /networks/<network>/settings/<section>   one section's workspace

export const SETTINGS_SECTIONS = ["network", "dns"] as const
export type SettingsSection = (typeof SETTINGS_SECTIONS)[number]

export const isSettingsSection = (value: string): value is SettingsSection =>
  SETTINGS_SECTIONS.some((section) => section === value)
