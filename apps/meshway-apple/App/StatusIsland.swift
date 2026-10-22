import SwiftUI
import NetworkExtension

// A presentation projection only. The switch still reads the Node's desiredUp.
enum IslandState: Equatable {
    case connected, connecting, approval, disconnected, stopping, leaving, unknown, error

    var title: String {
        switch self {
        case .connected: "已连接"
        case .connecting: "连接中"
        case .approval: "需要批准"
        case .disconnected: "已断开"
        case .stopping: "正在断开"
        case .leaving: "正在注销"
        case .unknown: "状态未知"
        case .error: "出错"
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
    static func resolve(phase: ClientPhase?, vpn: NEVPNStatus?, hasError: Bool, approvalRequired: Bool) -> Self {
        if hasError { return .error }
        if phase == .awaitingApproval || approvalRequired { return .approval }
        if let phase {
            switch phase {
            case .running: return .connected
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
    var errors: [String] {
        var values = [model.issue, model.ipcIssue].compactMap { $0 }
        if let error = model.status?.lastError { values.append("Node · \(human(error)): \(model.status?.lastErrorDetail ?? "未报告详细原因")") }
        if let error = model.status?.networkError { values.append("Network · \(human(error.problem)): \(error.detail ?? "未报告详细原因")") }
        if !model.sampleMode {
            if let error = model.tunnel.preferencesError { values.append(error) }
            if model.tunnel.extensionState == "Activation failed" { values.append(model.tunnel.message) }
        }
        return values
    }
    var state: IslandState {
        IslandState.resolve(phase: model.status?.phase,
                            vpn: model.sampleMode || !model.tunnel.preferencesLoaded ? nil : model.tunnel.vpnStatus,
                            hasError: !errors.isEmpty,
                            approvalRequired: !model.sampleMode && model.tunnel.extensionState == "Approval required")
    }
    var reason: String? {
        errors.first ?? (state == .error ? model.phase : nil)
    }
}

struct StatusIsland: View {
    let model: AppModel
    var toolbar = false
    @State private var expanded = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    private var presentation: ConnectionPresentation { ConnectionPresentation(model: model) }

    var body: some View {
        HStack(spacing: 9) {
            Circle().fill(presentation.state.color).frame(width: 8, height: 8)
            if toolbar {
                statusText.frame(maxWidth: 170, alignment: .leading)
                address
            } else {
                VStack(alignment: .leading, spacing: 4) {
                    statusText
                    address
                }.frame(maxWidth: .infinity, alignment: .leading)
            }
            Toggle("连接 Meshway", isOn: Binding(get: { model.status?.desiredUp ?? false }, set: { value in Task { await model.connect(value) } }))
                .labelsHidden().toggleStyle(.switch).tint(.teal).controlSize(.small)
                .disabled(model.busy || model.sampleMode || (model.status == nil && model.server.isEmpty))
                .help(unavailableHelp(model, fallback: model.status == nil && model.server.isEmpty ? "打开 Meshway 输入 Server URL" : "连接或断开 Meshway"))
        }
        .font(.system(size: 12)).padding(.horizontal, 12).frame(height: toolbar ? 36 : 60)
        .glassEffect(.regular.tint(presentation.state == .error ? .orange.opacity(0.15) : .clear), in: .rect(cornerRadius: toolbar ? 14 : 18))
        .animation(reduceMotion ? nil : .easeInOut(duration: 0.22), value: presentation.state)
        .popover(isPresented: $expanded) {
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    Text(presentation.state.title).font(.headline)
                    ForEach(Array(presentation.errors.enumerated()), id: \.offset) { _, error in Text(error).textSelection(.enabled) }
                    if let notice = model.notice { Text(notice).textSelection(.enabled) }
                    LabeledContent("Node", value: model.phase)
                    LabeledContent("macOS VPN", value: model.vpnLabel)
                    if !model.sampleMode { Text(model.tunnel.message).foregroundStyle(.secondary) }
                }.padding(20)
            }.frame(width: 360, height: 250)
        }
    }
    private var statusText: some View {
        Button { expanded = true } label: {
            Text(presentation.reason.map { "出错 · " + $0 } ?? presentation.state.title)
                .fontWeight(.semibold).lineLimit(1).truncationMode(.tail)
                .contentTransition(reduceMotion ? .identity : .numericText())
                .frame(maxWidth: .infinity, alignment: .leading)
        }.buttonStyle(.plain).help(presentation.reason ?? "查看连接详情")
    }
    @ViewBuilder private var address: some View {
        if let address = model.status?.node?.ipv4 { CopyText(value: address) }
        else { Text(model.status?.node == nil ? "尚未接入" : "未分配 IPv4").font(.system(size: 11)).foregroundStyle(.secondary) }
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
        }.buttonStyle(.plain).help(copied ? "已复制" : "点按复制 \(value)")
            .accessibilityLabel(copied ? "已复制 \(value)" : "复制 \(value)")
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
