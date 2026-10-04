// The settings page in the address:
//
//   /networks/<network>/settings             the sections
//   /networks/<network>/settings/<section>   one section's workspace

export const SETTINGS_SECTIONS = ["network", "activity"] as const
export type SettingsSection = (typeof SETTINGS_SECTIONS)[number]

export const isSettingsSection = (value: string): value is SettingsSection =>
  SETTINGS_SECTIONS.some((section) => section === value)

/** Whether this account may open `section` here (activity: those who manage members). */
export const sectionOffered = (
  section: SettingsSection,
  actions: { manageMembers: boolean }
) => section !== "activity" || actions.manageMembers
