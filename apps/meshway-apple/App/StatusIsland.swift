import SwiftUI
import NetworkExtension

// A presentation projection only. The switch still reads the Node's desiredUp.
enum IslandState: Equatable {
    case connected, connecting, approval, disconnected, stopping, leaving, unknown, error

    var title: String {
        switch self {
        case .connected: String(localized: "Connected")
        case .connecting: String(localized: "Connecting…")
        case .approval: String(localized: "Needs approval")
        case .disconnected: String(localized: "Off")
        case .stopping: String(localized: "Disconnecting…")
        case .leaving: String(localized: "Signing out…")
        case .unknown: String(localized: "Status unknown")
        case .error: String(localized: "Error")
        }
    }
    var color: Color {
        switch self {
        case .connected: .green
        case .error: .orange
        case .connecting, .approval: .accentColor
        default: .secondary
        }
    }
    static func resolve(phase: ClientPhase?, vpn: NEVPNStatus?, hasError: Bool, approvalRequired: Bool, plane: ClientPlane? = nil) -> Self {
        if hasError || phase == .blocked || phase == .retrying { return .error }
        if phase == .awaitingApproval || approvalRequired { return .approval }
        if let phase {
            switch phase {
            case .running:
                if plane == .ready { return .connected }
                if plane == .preparing { return .connecting }
                if plane == .failed || plane == .suspended { return .error }
                return .unknown
            case .starting, .synchronizing, .controlReady: return .connecting
            case .blocked, .retrying: return .error
            case .down: return .disconnected
            case .stopping: return .stopping
            case .leaving: return .leaving
            case .awaitingApproval: return .approval
            }
        }
        switch vpn {
        case .connecting, .reasserting, .connected: return .connecting
        case .disconnecting: return .stopping
        case .disconnected, .invalid: return .disconnected
        default: return .unknown
        }
    }
}

@MainActor
struct ConnectionPresentation {
    let model: AppModel
    var errors: [AppIssue] {
        var values = [model.issue, model.ipcIssue].compactMap { $0 }
        if let error = model.status?.lastError { values.append(AppIssue(.problem(error), detail: model.status?.lastErrorDetail)) }
        if let error = model.status?.networkError { values.append(AppIssue(.problem(error.problem), detail: error.detail)) }
        if !model.sampleMode {
            if let error = model.tunnel.preferencesError { values.append(AppIssue(.vpnPreferences, detail: error)) }
            if model.tunnel.extensionState == "Activation failed" { values.append(AppIssue(.extensionActivationFailed, detail: model.tunnel.message)) }
        }
        // A blocked or failed state always names a code, even without a reported cause.
        if values.isEmpty, let status = model.status {
            if status.phase == .blocked { values.append(AppIssue(.deviceBlocked)) }
            else if status.phase == .retrying { values.append(AppIssue(.connectionRetrying)) }
            else if status.phase == .running && status.dataPlane == .failed { values.append(AppIssue(.networkSetupFailed)) }
            else if status.phase == .running && status.dataPlane == .suspended { values.append(AppIssue(.networkSuspended)) }
        }
        return values
    }
    /// Facts that leave the connection state unchanged but deserve attention.
    var warnings: [AppIssue] {
        var values: [AppIssue] = []
        if let status = model.status, status.desiredUp {
            if status.sync != .connected, let error = status.syncError { values.append(AppIssue(.sync(error))) }
            if let netmap = status.netmap, !netmap.missingArtifacts.isEmpty || netmap.artifactProblem != nil {
                values.append(AppIssue(.artifact(netmap.artifactProblem), detail: netmap.missingArtifacts.joined(separator: ", ")))
            }
            if let unavailable = status.netmap?.routing?.requestedUnavailable { values.append(AppIssue(.selection(unavailable))) }
            if status.phase == .awaitingApproval && status.enrollmentUrl.flatMap(model.approvalURL) == nil { values.append(AppIssue(.approvalLinkMissing)) }
        }
        if !model.sampleMode {
            let state = model.tunnel.extensionState
            if state == "Approval required" { values.append(AppIssue(.extensionApprovalRequired)) }
            else if state == "Extension request requires reboot" { values.append(AppIssue(.extensionRebootRequired)) }
            else if state.hasPrefix("Unable to inspect extension") { values.append(AppIssue(.extensionStatusUnavailable, detail: state)) }
        }
        return values
    }
    /// Offline use stays possible until the installed configuration's expiry.
    var offlineUntil: Date? {
        guard warnings.contains(where: { $0.code == .syncServerUnreachable || $0.code == .syncServerBusy }),
              model.netmapApplied, let expiry = model.status?.netmap?.expiresAt, expiry > .now else { return nil }
        return expiry
    }
    var state: IslandState {
        IslandState.resolve(phase: model.status?.phase,
                            vpn: model.sampleMode || !model.tunnel.preferencesLoaded ? nil : model.tunnel.vpnStatus,
                            hasError: !errors.isEmpty,
                            approvalRequired: !model.sampleMode && model.tunnel.extensionState == "Approval required", plane: model.status?.dataPlane)
    }
    var step: ConnectionStep? { ConnectionStep.resolve(phase: model.status?.phase, plane: model.status?.dataPlane) }
    var reason: AppIssue? { state == .error ? errors.first : nil }
    var showsWarning: Bool { state != .error && !warnings.isEmpty }
}

func offlineText(_ date: Date) -> String {
    String(localized: "Works offline until \(date.formatted(.dateTime.month().day().hour().minute()))")
}

struct StatusIsland: View {
    let model: AppModel
    var toolbar = false
    @State private var expanded = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    private var presentation: ConnectionPresentation { ConnectionPresentation(model: model) }

    var body: some View {
        HStack(spacing: 9) {
            ConnectionDot(color: presentation.state.color, connecting: presentation.state == .connecting)
            if toolbar {
                VStack(alignment: .leading, spacing: 4) {
                    statusText
                    if presentation.state == .connecting { ConnectionSteps(step: presentation.step) }
                }.fixedSize(horizontal: presentation.state != .error, vertical: false)
                if presentation.state != .connecting { address }
            } else {
                VStack(alignment: .leading, spacing: 4) {
                    statusText
                    if presentation.state == .connecting { ConnectionSteps(step: presentation.step) }
                    else { address }
                }.frame(maxWidth: .infinity, alignment: .leading)
            }
            Toggle(isOn: Binding(get: { model.status?.desiredUp ?? false }, set: { value in Task { await model.connect(value) } })) { Text("Connect Meshway") }
                .labelsHidden().toggleStyle(ConnectionSwitchStyle()).tint(.teal).controlSize(.small)
                .disabled(model.busy || model.sampleMode || (model.status == nil && model.server.isEmpty))
                .help(unavailableHelp(model, fallback: model.status == nil && model.server.isEmpty ? String(localized: "Open Meshway to enter a server address") : String(localized: "Connect or disconnect")))
        }
        .font(.system(size: 12)).padding(.horizontal, 12).frame(height: toolbar ? 36 : 60)
        .background(presentation.state == .error ? Color.orange.opacity(0.12) : .clear, in: .rect(cornerRadius: toolbar ? 14 : 18))
        .glassEffect(.regular.tint(presentation.state == .error ? .orange.opacity(0.25) : .clear), in: .rect(cornerRadius: toolbar ? 14 : 18))
        .animation(reduceMotion ? nil : .easeInOut(duration: 0.22), value: presentation.state)
        .popover(isPresented: $expanded) { IssuePopover(model: model) }
    }
    private var statusText: some View {
        Button { expanded = true } label: {
            HStack(spacing: 5) {
                Text(presentation.reason?.message ?? (presentation.state == .connecting ? presentation.step?.title ?? presentation.state.title : presentation.state.title))
                    .fontWeight(.semibold).lineLimit(1).truncationMode(.tail)
                    .contentTransition(reduceMotion ? .identity : .numericText())
                if presentation.showsWarning {
                    Image(systemName: "exclamationmark.triangle.fill").font(.system(size: 10)).foregroundStyle(Color.issueAmber)
                        .transition(.opacity).accessibilityLabel(Text("Warning"))
                }
            }.frame(maxWidth: toolbar ? nil : .infinity, alignment: .leading)
        }.buttonStyle(.plain)
            .help(presentation.reason?.code.rawValue ?? presentation.warnings.first.map { $0.message + " " + $0.code.rawValue } ?? String(localized: "Connection details"))
    }
    @ViewBuilder private var address: some View {
        if let address = model.status?.node?.ipv4 { CopyText(value: address) }
        else { Text("No IPv4 address").font(.system(size: 11)).foregroundStyle(.secondary) }
    }
}

struct IssuePopover: View {
    let model: AppModel
    private var presentation: ConnectionPresentation { ConnectionPresentation(model: model) }
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                Text(presentation.state.title).font(.headline)
                ForEach(presentation.errors) { IssueRow(issue: $0) }
                ForEach(presentation.warnings) { IssueRow(issue: $0) }
                if let until = presentation.offlineUntil { Text(offlineText(until)).font(.system(size: 12)).foregroundStyle(.secondary) }
                if let notice = model.notice { Text(notice).font(.system(size: 12)) }
                Divider()
                LabeledContent { Text(model.phase) } label: { Text(verbatim: "Meshway") }.font(.system(size: 12))
                LabeledContent { Text(model.vpnLabel) } label: { Text("macOS VPN") }.font(.system(size: 12))
            }.padding(20)
        }.frame(width: 340).frame(minHeight: 140, maxHeight: 300)
    }
}

struct CopyText: View {
    let value: String
    @State private var copied = false
    @State private var copyID = 0
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    var body: some View {
        Button { copy(value); copied = true; copyID += 1 } label: {
            Text(value).font(.system(size: 11, design: .monospaced))
                .foregroundStyle(copied ? Color.teal : Color.secondary)
                .overlay(alignment: .trailing) {
                    Image(systemName: "checkmark").font(.system(size: 9, weight: .semibold))
                        .foregroundStyle(.teal).offset(x: 12).opacity(copied ? 1 : 0)
                }
        }.buttonStyle(.plain).help(copied ? String(localized: "Copied") : String(localized: "Click to copy"))
            .accessibilityLabel(copied ? String(localized: "Copied") : String(localized: "Copy \(value)"))
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.15), value: copied)
            .task(id: copyID) {
                guard copied else { return }
                do { try await Task.sleep(for: .seconds(1.5)) } catch { return }
                copied = false
            }
    }
}

struct MenuRowStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        MenuRowBody(configuration: configuration)
    }
    private struct MenuRowBody: View {
        let configuration: ButtonStyle.Configuration
        @State private var hovered = false
        @Environment(\.isEnabled) private var enabled
        var body: some View {
            configuration.label
                .foregroundStyle(hovered && enabled ? Color.white : Color.primary)
                .padding(.horizontal, 10).frame(height: 29).frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(.rect(cornerRadius: 6))
                .background(hovered && enabled ? Color.accentColor.opacity(configuration.isPressed ? 0.8 : 1) : .clear, in: .rect(cornerRadius: 6))
                .opacity(enabled ? 1 : 0.55).onHover { hovered = $0 }
        }
    }
}
