import SwiftUI

enum SettingsPage: String, CaseIterable, Identifiable {
    case general, account, details, diagnostics
    var id: String { rawValue }
    var title: String {
        switch self {
        case .general: String(localized: "General")
        case .account: String(localized: "Account")
        case .details: String(localized: "Details")
        case .diagnostics: String(localized: "Diagnostics")
        }
    }
    var symbol: String {
        switch self {
        case .general: "gearshape"
        case .account: "laptopcomputer"
        case .details: "info.circle"
        case .diagnostics: "waveform.path.ecg"
        }
    }
}

struct SettingsView: View {
    let model: AppModel
    @State private var page: SettingsPage = .general
    @State private var showLogout = false
    var body: some View {
        TabView(selection: $page) {
            general.tabItem { Label(SettingsPage.general.title, systemImage: SettingsPage.general.symbol) }.tag(SettingsPage.general)
            account.tabItem { Label(SettingsPage.account.title, systemImage: SettingsPage.account.symbol) }.tag(SettingsPage.account)
            details.tabItem { Label(SettingsPage.details.title, systemImage: SettingsPage.details.symbol) }.tag(SettingsPage.details)
            DiagnosticsView(model: model, netcheckOnly: true)
                .tabItem { Label(SettingsPage.diagnostics.title, systemImage: SettingsPage.diagnostics.symbol) }.tag(SettingsPage.diagnostics)
        }.frame(width: 580, height: 510)
            .overlay(alignment: .bottomTrailing) { if model.sampleMode { SampleBanner().padding(6) } }
            .confirmationDialog("Sign this Mac out of the network?", isPresented: $showLogout, titleVisibility: .visible) {
                Button("Sign out", role: .destructive) { Task { await model.identityOperation(logout: true) } }
                Button("Cancel", role: .cancel) {}
            } message: { Text("This Mac is removed from the network. You’ll need approval to join again.") }
            .task {
                #if DEBUG
                if model.sampleMode, let index = CommandLine.arguments.firstIndex(of: "--preview-settings"), CommandLine.arguments.count > index + 1 {
                    page = SettingsPage(rawValue: CommandLine.arguments[index + 1]) ?? .general
                }
                #endif
            }
    }
    private var general: some View {
        Form { Section("App icon") { IconPicker() } }.formStyle(.grouped)
    }
    private var account: some View {
        Form {
            Section("This Mac") {
                LabeledContent { Text(verbatim: model.status?.node?.hostname ?? "—") } label: { Text("Name") }
                AddressRow(title: "IPv4", value: model.status?.node?.ipv4, missing: "—")
                AddressRow(title: "IPv6", value: model.status?.node?.ipv6, missing: "—")
                LabeledContent { Text(verbatim: model.status?.server ?? (model.server.isEmpty ? "—" : model.server)) } label: { Text("Server") }
            }
            Section("Security") {
                Button("Rotate key", systemImage: "key.horizontal") { Task { await model.identityOperation(logout: false) } }
                    .disabled(!model.canRotateKey || model.busy || model.sampleMode)
                    .help(unavailableHelp(model, fallback: model.canRotateKey ? String(localized: "Replace this Mac’s key") : String(localized: "Available after joining")))
                Button("Sign out…", role: .destructive) { showLogout = true }
                    .disabled(!model.canLogout || model.busy || model.sampleMode)
                    .help(unavailableHelp(model, fallback: model.canLogout ? String(localized: "Remove this Mac from the network") : String(localized: "Available after joining")))
            }
            if let notice = model.notice { Text(notice).font(.caption).foregroundStyle(.secondary) }
            if let issue = model.issue { IssueRow(issue: issue) }
        }.formStyle(.grouped)
    }
    private var extensionLabel: String {
        if model.sampleMode { return String(localized: "Preview") }
        switch model.tunnel.extensionState {
        case "Enabled", "Extension request completed": return String(localized: "Enabled")
        case "Approval required": return String(localized: "Needs approval")
        case "Not installed": return String(localized: "Not installed")
        case "Installed, not enabled": return String(localized: "Not enabled")
        case "Activation failed": return String(localized: "Failed")
        case "Extension request requires reboot": return String(localized: "Restart required")
        case "Not checked": return String(localized: "Not checked")
        default: return String(localized: "Unavailable")
        }
    }
    private func row(_ title: LocalizedStringKey, _ value: String) -> some View {
        LabeledContent { Text(verbatim: value) } label: { Text(title) }
    }
    private func time(_ title: LocalizedStringKey, _ date: Date?) -> some View {
        LabeledContent { OptionalTime(date: date) } label: { Text(title) }
    }
    private var details: some View {
        Form {
            Section("Connection") {
                row("Meshway", model.phase)
                row("macOS VPN", model.vpnLabel)
                row("Network extension", extensionLabel)
                Button("Enable network extension…") { model.tunnel.activate() }.disabled(model.sampleMode)
                    .help(unavailableHelp(model, fallback: String(localized: "Allow Meshway in System Settings")))
            }
            Section("Status") {
                time("Works offline until", model.status?.netmap?.expiresAt)
                row("Server", model.status.map { syncName($0.sync) } ?? "—")
                time("Last update", model.status?.lastSyncAt)
                row("Network", model.status.map { planeName($0.dataPlane) } ?? "—")
                time("Last network change", model.status?.lastNetworkChange)
                time("Next retry", model.status?.retryAt)
                row("Key version", model.status?.identity.map { String($0.keyGeneration) } ?? "—")
            }
            Section("Network settings") {
                row("Version", model.status?.netmap.map { "\($0.reference.revision).\($0.reference.serial)" } ?? "—")
                row("Applied", model.status?.netmap == nil ? "—" : (model.netmapApplied ? String(localized: "Yes") : String(localized: "Applying…")))
                row("Missing rule sets", model.status?.netmap.map { String($0.missingArtifacts.count) } ?? "—")
            }
            if let selection = model.selection {
                Section("Exit profile") {
                    row("In use", selection.appliedProfile)
                    row("Chosen", selection.requestedProfile ?? String(localized: "Default"))
                    row("Default", selection.defaultProfile)
                    row("Locked", selection.locked ? String(localized: "Yes") : String(localized: "No"))
                    if let auto = selection.auto {
                        row("Automatic", auto.profile)
                        row("Based on", basisName(auto.basis))
                        row("Region", auto.region ?? "—")
                    }
                }
            }
            let presentation = ConnectionPresentation(model: model)
            if !presentation.errors.isEmpty || !presentation.warnings.isEmpty {
                Section("Issues") {
                    ForEach(presentation.errors) { IssueRow(issue: $0) }
                    ForEach(presentation.warnings) { IssueRow(issue: $0) }
                }
            }
        }.formStyle(.grouped)
    }
}
