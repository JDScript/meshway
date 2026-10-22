import SwiftUI

@MainActor @Observable
final class AppNavigation {
    static let shared = AppNavigation()
    var page: AppPage? = .peers
}

// Motion and feedback live in the presentation layer; no optimistic Node state.
struct PhaseLabel: View {
    let model: AppModel
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    private var color: Color {
        switch model.status?.phase {
        case .running: .green
        case .blocked, .retrying: .orange
        case .starting, .synchronizing, .awaitingApproval, .stopping, .leaving: .accentColor
        default: .secondary
        }
    }
    var body: some View {
        HStack(spacing: 6) {
            Circle().fill(color).frame(width: 7, height: 7)
            Text(model.phase).lineLimit(1).contentTransition(.numericText())
        }.font(.callout).animation(reduceMotion ? nil : .easeInOut(duration: 0.2), value: model.phase)
    }
}

struct ConnectionHeader: View {
    let model: AppModel
    var compact: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: model.status?.phase == .running ? "network" : "power")
                .font(compact ? .title3 : .title).foregroundStyle(.tint)
                .frame(width: compact ? 36 : 48, height: compact ? 36 : 48)
                .glassEffect(.regular, in: .circle)
                .contentTransition(reduceMotion ? .identity : .symbolEffect(.replace))
                .symbolEffect(.pulse, isActive: model.busy && !reduceMotion)
            VStack(alignment: .leading, spacing: 5) {
                Text(model.status?.node?.hostname ?? "This Mac").font(compact ? .headline : .title2.weight(.semibold)).lineLimit(1)
                PhaseLabel(model: model)
            }.frame(maxWidth: .infinity, alignment: .leading)
            Toggle("Connect", isOn: Binding(get: { model.status?.desiredUp ?? false }, set: { enabled in Task { await model.connect(enabled) } }))
                .labelsHidden().toggleStyle(.switch).controlSize(.large)
                .disabled(model.busy || model.sampleMode || (model.status == nil && model.server.isEmpty))
                .help(unavailableHelp(model, fallback: model.status == nil && model.server.isEmpty ? "Open Meshway to enter a Server URL" : "Change the Node's desired connection state"))
        }.frame(height: compact ? 56 : 66)
    }
}

struct SampleBanner: View {
    var body: some View {
        Label("SAMPLE", systemImage: "testtube.2").font(.system(size: 9, weight: .semibold)).tracking(0.6)
            .foregroundStyle(.secondary).padding(.horizontal, 7).padding(.vertical, 4)
            .background(.quaternary, in: Capsule())
            .help("SAMPLE DATA · Debug preview. System-changing actions are disabled.")
    }
}

// Fixed-size feedback: incoming errors never move rows. The popover preserves
// complete, independent Control and network errors without clipping details.
struct FeedbackShelf: View {
    let model: AppModel
    var compact = false
    @State private var expanded = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    private var messages: [String] {
        var result = [model.issue, model.ipcIssue, model.notice].compactMap { $0 }
        if let error = model.status?.lastError { result.append("Node · \(human(error)): \(model.status?.lastErrorDetail ?? "No detail reported")") }
        if let error = model.status?.networkError { result.append("Network · \(human(error.problem)): \(error.detail ?? "No detail reported")") }
        return result
    }
    var body: some View {
        Button { expanded = true } label: {
            HStack(spacing: 8) {
                Image(systemName: messages.isEmpty ? "info.circle" : "exclamationmark.circle")
                Text(messages.first ?? (model.busy ? "Working…" : (model.sampleMode ? "Preview · actions disabled" : "macOS VPN · \(model.vpnLabel)")))
                    .lineLimit(1).truncationMode(.tail).frame(maxWidth: .infinity, alignment: .leading)
                Text(messages.count > 1 ? "+\(messages.count - 1)" : "").font(.caption2).frame(width: 16)
                Image(systemName: "chevron.up").font(.caption2).opacity(messages.isEmpty ? 0 : 1)
            }.font(.caption).foregroundStyle(messages.isEmpty ? Color.secondary : Color.orange)
                .contentTransition(.opacity).padding(.horizontal, compact ? 4 : 20)
                .frame(height: compact ? 28 : 38).contentShape(Rectangle())
        }.buttonStyle(.plain).disabled(messages.isEmpty)
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.18), value: messages)
            .popover(isPresented: $expanded) {
                ScrollView {
                    VStack(alignment: .leading, spacing: 16) {
                        Text("Connection details").font(.headline)
                        ForEach(Array(messages.enumerated()), id: \.offset) { _, message in Text(message).textSelection(.enabled) }
                    }.padding(20)
                }.frame(width: 370, height: 240)
            }.help(messages.joined(separator: "\n"))
    }
}

struct RelativeTime: View {
    let date: Date
    var body: some View {
        TimelineView(.periodic(from: .now, by: 1)) { _ in
            Text(date, format: .relative(presentation: .named)).foregroundStyle(.secondary)
                .contentTransition(.numericText()).lineLimit(1)
                .help(date.formatted(date: .complete, time: .complete))
        }
    }
}
struct OptionalTime: View {
    let date: Date?
    var body: some View {
        if let date { RelativeTime(date: date) } else { Text("Not reported").foregroundStyle(.secondary) }
    }
}

struct CopyButton: View {
    let value: String
    var title = "Copy address"
    @State private var copied = false
    @State private var resetID = 0
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    var body: some View {
        Button { copy(value); copied = true; resetID += 1 } label: {
            Image(systemName: copied ? "checkmark" : "doc.on.doc")
                .contentTransition(reduceMotion ? .identity : .symbolEffect(.replace))
                .foregroundStyle(copied ? Color.green : Color.secondary).frame(width: 18, height: 18)
        }.buttonStyle(.borderless).help(copied ? "Copied" : title).accessibilityLabel(copied ? "Copied" : title)
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
    var missing = "Not assigned"
    var body: some View {
        LabeledContent(title) {
            HStack(spacing: 8) {
                Text(value ?? missing).font(value == nil ? .callout : .callout.monospaced()).foregroundStyle(.secondary).textSelection(.enabled)
                if let value { CopyButton(value: value, title: "Copy \(title)") }
                else { Color.clear.frame(width: 18, height: 18) }
            }
        }
    }
}
@MainActor func unavailableHelp(_ model: AppModel, fallback: String) -> String {
    if model.sampleMode { return "Sample preview — system-changing actions are disabled" }
    if model.busy { return "Waiting for the current operation to finish" }
    return fallback
}
func copy(_ value: String) { NSPasteboard.general.clearContents(); NSPasteboard.general.setString(value, forType: .string) }
func human<T>(_ value: T) -> String {
    String(describing: value).replacingOccurrences(of: "([a-z])([A-Z])", with: "$1 $2", options: .regularExpression).capitalized
}
func milliseconds(_ value: UInt64) -> String { String(format: "%.2f ms", Double(value) / 1000) }
