import SwiftUI

@MainActor @Observable
final class AppNavigation {
    static let shared = AppNavigation()
    var page: AppPage? = .peers
}

struct SampleBanner: View {
    var body: some View {
        #if DEBUG
        Text(verbatim: "SAMPLE").font(.system(size: 8, weight: .medium)).tracking(0.5)
            .foregroundStyle(.tertiary).padding(.horizontal, 5).padding(.vertical, 2)
            .background(.quaternary.opacity(0.5), in: Capsule())
            .help("Preview data. Actions are disabled.")
        #else
        EmptyView()
        #endif
    }
}

struct RelativeTime: View {
    let date: Date
    @Environment(\.locale) private var locale
    var body: some View {
        TimelineView(.periodic(from: .now, by: 1)) { _ in
            Text(date, format: .relative(presentation: .named).locale(locale)).foregroundStyle(.secondary)
                .contentTransition(.numericText()).lineLimit(1)
                .help(date.formatted(.dateTime.locale(locale)))
        }
    }
}
struct OptionalTime: View {
    let date: Date?
    var body: some View {
        if let date { RelativeTime(date: date) } else { Text("—").foregroundStyle(.secondary) }
    }
}

struct CopyButton: View {
    let value: String
    var title: String = String(localized: "Copy")
    @State private var copied = false
    @State private var resetID = 0
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    var body: some View {
        Button { copy(value); copied = true; resetID += 1 } label: {
            Image(systemName: copied ? "checkmark" : "doc.on.doc")
                .contentTransition(reduceMotion ? .identity : .symbolEffect(.replace))
                .foregroundStyle(copied ? Color.teal : Color.secondary).frame(width: 18, height: 18)
        }.buttonStyle(.borderless).help(copied ? String(localized: "Copied") : title).accessibilityLabel(copied ? String(localized: "Copied") : title)
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.18), value: copied)
            .task(id: resetID) {
                guard copied else { return }
                do { try await Task.sleep(for: .seconds(1.6)) } catch { return }
                copied = false
            }
    }
}
struct AddressRow: View {
    let title: String
    let value: String?
    var missing: String = String(localized: "None")
    var body: some View {
        LabeledContent {
            HStack(spacing: 8) {
                Text(value ?? missing).font(value == nil ? .callout : .callout.monospaced()).foregroundStyle(.secondary).textSelection(.enabled)
                if let value { CopyButton(value: value) }
                else { Color.clear.frame(width: 18, height: 18) }
            }
        } label: { Text(verbatim: title) }
    }
}

/// The message, its code and any untranslated detail; the code is copyable.
struct IssueRow: View {
    let issue: AppIssue
    var compact = false
    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Image(systemName: issue.code.isWarning ? "exclamationmark.triangle.fill" : "xmark.octagon.fill")
                .foregroundStyle(issue.code.isWarning ? Color.issueAmber : Color.orange).font(.system(size: compact ? 10 : 12))
            VStack(alignment: .leading, spacing: 3) {
                Text(issue.message).font(.system(size: compact ? 11 : 12)).fixedSize(horizontal: false, vertical: true)
                IssueCodeLabel(code: issue.code)
                if let detail = issue.detail, !compact {
                    Text(verbatim: detail).font(.system(size: 10)).foregroundStyle(.tertiary).textSelection(.enabled)
                        .lineLimit(3).fixedSize(horizontal: false, vertical: true)
                }
            }
        }.frame(maxWidth: .infinity, alignment: .leading).help(issue.detail ?? issue.code.rawValue)
    }
}
struct IssueCodeLabel: View {
    let code: IssueCode
    var body: some View {
        HStack(spacing: 4) {
            Text(verbatim: code.rawValue).font(.system(size: 10, design: .monospaced)).foregroundStyle(.secondary).textSelection(.enabled)
            CopyButton(value: code.rawValue, title: String(localized: "Copy code")).controlSize(.mini).scaleEffect(0.8)
        }
    }
}

extension Color {
    static let issueAmber = Color(red: 0.96, green: 0.65, blue: 0.14)
}

@MainActor func unavailableHelp(_ model: AppModel, fallback: String) -> String {
    if model.sampleMode { return String(localized: "Preview data. Actions are disabled.") }
    if model.busy { return String(localized: "Please wait…") }
    return fallback
}
func copy(_ value: String) { NSPasteboard.general.clearContents(); NSPasteboard.general.setString(value, forType: .string) }
func milliseconds(_ value: UInt64) -> String {
    (Double(value) / 1000).formatted(.number.precision(.fractionLength(2))) + " ms"
}
func peerStateName(_ state: ClientPeerState) -> String {
    switch state {
    case .direct: String(localized: "Direct")
    case .relay: String(localized: "Relay")
    case .relayIdle: String(localized: "Relay · idle")
    case .idle: String(localized: "Idle")
    case .unreachable: String(localized: "Unreachable")
    case .offline: String(localized: "Offline")
    }
}
func planeName(_ plane: ClientPlane) -> String {
    switch plane {
    case .controlOnly: String(localized: "Control only")
    case .stopped: String(localized: "Off")
    case .preparing: String(localized: "Starting")
    case .ready: String(localized: "Ready")
    case .suspended: String(localized: "Paused")
    case .failed: String(localized: "Failed")
    }
}
func syncName(_ sync: ClientSync) -> String {
    switch sync {
    case .connected: String(localized: "Connected")
    case .reconnecting: String(localized: "Reconnecting")
    case .disconnected: String(localized: "Disconnected")
    }
}
func basisName(_ basis: ClientAutoBasis) -> String {
    switch basis {
    case .label: String(localized: "Region")
    case .noRuleMatched: String(localized: "No matching rule")
    case .noTrustedRegion: String(localized: "Region not set")
    }
}
