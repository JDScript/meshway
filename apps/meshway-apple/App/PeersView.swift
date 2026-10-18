import SwiftUI

struct PeerRow: View {
    let model: AppModel
    let peer: ClientPeer
    var body: some View {
        HStack(spacing: 14) {
            Image(systemName: "desktopcomputer").font(.title3).foregroundStyle(.secondary).frame(width: 30)
            VStack(alignment: .leading, spacing: 5) {
                HStack { Text(peer.node.hostname).font(.body.weight(.medium)); Spacer(); Text(peer.node.ipv4 ?? peer.node.ipv6 ?? "No address").font(.callout.monospaced()).foregroundStyle(.secondary) }
                HStack(spacing: 6) {
                    Text(human(peer.state)).foregroundStyle(peer.state == .direct ? Color.green : Color.secondary)
                    if let transport = peer.transport { Text("· \(transport)") }
                    Spacer()
                    if let rtt = peer.rtt { Text(milliseconds(rtt.durationUs)).help("Measured " + rtt.observedAt.formatted()) }
                }.font(.caption).foregroundStyle(.secondary)
                Text(model.path(peer)).font(.caption).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
            }
        }.padding(.vertical, 12).contentShape(Rectangle())
    }
}
struct PeerCopyActions: View {
    let peer: ClientPeer
    var body: some View {
        Button("Copy name") { copy(peer.node.hostname) }
        if let value = peer.node.ipv4 { Button("Copy IPv4") { copy(value) } }
        if let value = peer.node.ipv6 { Button("Copy IPv6") { copy(value) } }
        Button("Copy Node ID") { copy(peer.nodeId) }
    }
}
struct PeerDetail: View {
    let model: AppModel
    let peer: ClientPeer
    @Environment(\.dismiss) private var dismiss
    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            HStack { Text(peer.node.hostname).font(.title2.weight(.semibold)); Spacer(); Button("Done") { dismiss() } }
            AddressRow(title: "Mesh IPv4", value: peer.node.ipv4)
            AddressRow(title: "Mesh IPv6", value: peer.node.ipv6)
            AddressRow(title: "Node ID", value: peer.nodeId)
            Divider()
            LabeledContent("State", value: human(peer.state))
            LabeledContent("Path", value: model.path(peer))
            LabeledContent("Transport", value: peer.transport ?? "Not reported")
            if let mtu = peer.mtu { LabeledContent("MTU", value: "\(mtu.path) outer / \(mtu.inner) inner") }
            else { LabeledContent("MTU", value: missingMtu) }
            if let rtt = peer.rtt {
                HStack { Text("Last successful RTT"); Spacer(); Text(milliseconds(rtt.durationUs)); RelativeTime(date: rtt.observedAt) }
            } else { LabeledContent("RTT", value: "No successful measurement") }
            Button("Ping this peer", systemImage: "waveform.path") { model.pingTarget = peer.node.hostname; model.ping(); dismiss() }.disabled(model.sampleMode || model.pingRunning)
        }.padding(28).frame(width: 560).textSelection(.enabled)
    }
    private var missingMtu: String {
        guard peer.state == .direct else { return "Not applicable to this path" }
        guard let transport = peer.transport else { return "Not reported" }
        return transport == "native_udp" ? "Not measured" : "Not applicable to this transport"
    }
}
struct ProfilesView: View {
    let model: AppModel
    let selection: ClientSelection
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack { Label("Exit profile", systemImage: "arrow.up.forward.circle").font(.headline); Spacer(); Text(selection.appliedProfile).fontWeight(.semibold) }
            Text("Default: \(selection.defaultProfile)" + (selection.locked ? " · locked by administrator" : "")).font(.callout).foregroundStyle(.secondary)
            Text(selection.requestedProfile.map { "Requested: \($0)" } ?? "Following the default profile").font(.caption).foregroundStyle(.secondary)
            if let reason = selection.requestedUnavailable { Text("Requested profile unavailable: \(human(reason))").foregroundStyle(.orange) }
            if let auto = selection.auto { Text("Auto → \(auto.profile) · \(human(auto.basis)) · region \(auto.region ?? "not declared")").font(.caption) }
            ForEach(selection.profiles, id: \.name) { profile in
                HStack(alignment: .top) {
                    Image(systemName: profile.name == selection.appliedProfile ? "checkmark.circle.fill" : "circle").foregroundStyle(profile.name == selection.appliedProfile ? Color.accentColor : Color.secondary)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(profile.name).fontWeight(.medium)
                        Text(summary(profile)).font(.caption).foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button("Select") { Task { await model.perform(.selectProfile(profile: profile.name)) } }.disabled(selection.locked || model.busy || model.sampleMode)
                }
            }
            Button("Use default profile") { Task { await model.perform(.clearProfile) } }.disabled(model.busy || model.sampleMode || selection.requestedProfile == nil)
        }.card()
    }
    private func summary(_ p: ClientProfile) -> String {
        switch p.kind {
        case .mesh: return "Mesh only; other external traffic unmanaged"
        case .auto: return "Selects an allowed profile using the declared region"
        case .global: return "All external traffic · " + action(p.fallback)
        case .custom:
            var parts = ["\(p.rules) rules"]
            if !p.exits.isEmpty { parts.append("exits " + p.exits.map(model.name).joined(separator: ", ")) }
            if p.fallback != nil { parts.append("otherwise " + action(p.fallback)) }
            return parts.joined(separator: " · ")
        }
    }
    private func action(_ value: ClientRouteAction?) -> String {
        guard let value else { return "not specified" }
        switch value {
        case .direct: return "direct"
        case .unmanaged: return "unmanaged"
        case .exit(let nodes, let via): return "exit " + nodes.map(selector).joined(separator: ", ") + (via.isEmpty ? "" : " via " + via.map(selector).joined(separator: " → "))
        }
    }
    private func selector(_ value: String) -> String { value.hasPrefix("node:") ? model.name(String(value.dropFirst(5))) : value }
}
