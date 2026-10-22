import SwiftUI

@MainActor
func peerSummary(_ peer: ClientPeer, model: AppModel) -> String {
    switch peer.state {
    case .direct: return peer.rtt.map { milliseconds($0.durationUs) } ?? "直连"
    case .relay: return peer.via.isEmpty ? "中继" : "经 " + peer.via.map(model.name).joined(separator: " → ")
    case .idle, .relayIdle: return "空闲"
    case .offline: return "离线"
    case .unreachable: return "不可达"
    }
}

struct PeerDot: View {
    let peer: ClientPeer
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    private var color: Color {
        switch peer.state {
        case .direct: .green
        case .relay: .indigo
        case .idle, .relayIdle: .gray
        case .offline: .gray.opacity(0.35)
        case .unreachable: .orange
        }
    }
    var body: some View {
        Circle().fill(color).frame(width: 7, height: 7).accessibilityLabel(human(peer.state))
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.2), value: peer.state)
    }
}

struct DevicesView: View {
    @Bindable var model: AppModel
    @State private var selected: String?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    private var peer: ClientPeer? { model.peers.first { $0.nodeId == selected } }

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            VStack(spacing: 6) {
                HStack(spacing: 6) {
                    Image(systemName: "magnifyingglass").foregroundStyle(.tertiary)
                    TextField("搜索设备", text: $model.search).textFieldStyle(.plain)
                    Text("\(model.peers.count)").foregroundStyle(.tertiary)
                }.font(.system(size: 12)).padding(.horizontal, 10).frame(height: 30)
                ScrollView {
                    LazyVStack(spacing: 1) {
                        ForEach(model.peers, id: \.nodeId) { peer in
                            Button { selected = peer.nodeId } label: {
                                HStack(spacing: 10) {
                                    PeerDot(peer: peer)
                                    Text(peer.node.hostname).fontWeight(.medium).frame(maxWidth: .infinity, alignment: .leading).lineLimit(1)
                                    Text(peer.node.ipv4 ?? peer.node.ipv6 ?? "未分配地址")
                                        .font(.system(size: 11, design: .monospaced)).opacity(0.55).frame(width: 110, alignment: .leading).lineLimit(1)
                                    Text(peerSummary(peer, model: model)).font(.system(size: 11)).opacity(0.55)
                                        .frame(width: 125, alignment: .trailing).lineLimit(1)
                                        .help(model.path(peer))
                                }
                            }.buttonStyle(MenuRowStyle())
                                .background(selected == peer.nodeId ? Color.accentColor.opacity(0.12) : .clear, in: .rect(cornerRadius: 6))
                                .accessibilityAddTraits(selected == peer.nodeId ? .isSelected : [])
                                .contextMenu { PeerCopyActions(peer: peer) }.transition(.opacity)
                        }
                    }.padding(4)
                }.scrollIndicators(.hidden)
                    .overlay {
                        if let kind = EmptyStateKind.devices(query: model.search, count: model.peers.count, server: serverEmptyState(model)) {
                            EmptyStateView(kind: kind)
                        }
                    }
            }.frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(.background.opacity(0.45), in: .rect(cornerRadius: 16))
            // A stable column keeps addresses and rows still as selection changes.
            ZStack(alignment: .top) {
                if let peer {
                    PeerDetail(model: model, peer: peer) { selected = nil }
                        .id(peer.nodeId).transition(reduceMotion ? .opacity : .move(edge: .trailing).combined(with: .opacity))
                } else if let kind = serverEmptyState(model) {
                    EmptyStateView(kind: kind, compact: true)
                }
            }.frame(width: 210).frame(maxHeight: .infinity)
        }.font(.system(size: 12)).clipped()
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.22), value: selected)
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.18), value: model.peers.map(\.nodeId))
            .task {
                #if DEBUG
                if model.sampleMode && CommandLine.arguments.contains("--preview-selected") { selected = model.peers.first?.nodeId }
                #endif
            }
    }
}

struct CompactPeerRow: View {
    let model: AppModel
    let peer: ClientPeer
    @State private var copied = false
    @State private var copyID = 0
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    private var address: String? { peer.node.ipv4 ?? peer.node.ipv6 }
    var body: some View {
        Button {
            if let address { copy(address); copied = true; copyID += 1 }
        } label: {
            HStack(spacing: 9) {
                PeerDot(peer: peer)
                Text(peer.node.hostname).lineLimit(1)
                Spacer(minLength: 3)
                Text(copied ? "已复制" : peerSummary(peer, model: model)).font(.system(size: 11)).opacity(0.55).lineLimit(1)
            }
        }.buttonStyle(MenuRowStyle()).disabled(address == nil)
            .help(address.map { "点按复制 \($0) · \(model.path(peer))" } ?? "未分配地址")
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.15), value: copied)
            .task(id: copyID) {
                guard copied else { return }
                do { try await Task.sleep(for: .seconds(1.5)) } catch { return }
                copied = false
            }
    }
}

struct PeerCopyActions: View {
    let peer: ClientPeer
    var body: some View {
        Button("复制名称") { copy(peer.node.hostname) }
        if let value = peer.node.ipv4 { Button("复制 IPv4") { copy(value) } }
        if let value = peer.node.ipv6 { Button("复制 IPv6") { copy(value) } }
        Button("复制 Node ID") { copy(peer.nodeId) }
    }
}

struct PeerDetail: View {
    let model: AppModel
    let peer: ClientPeer
    var close: () -> Void
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Text(peer.node.hostname).font(.system(size: 15, weight: .semibold)).lineLimit(2)
                Spacer(minLength: 0)
                Button(action: close) { Image(systemName: "xmark").font(.system(size: 10)).foregroundStyle(.tertiary) }.buttonStyle(.plain).help("关闭详情")
            }
            VStack(alignment: .leading, spacing: 7) {
                Text("地址").foregroundStyle(.secondary)
                if let value = peer.node.ipv4 { CopyText(value: value) }
                if let value = peer.node.ipv6 { CopyText(value: value) }
                if peer.node.ipv4 == nil && peer.node.ipv6 == nil { Text("未分配") }
            }
            VStack(alignment: .leading, spacing: 7) {
                Text("路径 · 承载").foregroundStyle(.secondary)
                Text(path).fixedSize(horizontal: false, vertical: true)
                Text(peer.transport ?? "未报告承载").foregroundStyle(.secondary)
            }
            VStack(alignment: .leading, spacing: 7) {
                Text("延迟").foregroundStyle(.secondary)
                Text(peer.rtt.map { milliseconds($0.durationUs) } ?? "未测量")
                    .help(peer.rtt.map { "测量于 " + $0.observedAt.formatted() } ?? "尚无成功的 RTT 测量")
            }
            Spacer()
            Button("Ping", systemImage: "waveform.path") {
                model.pingTarget = peer.node.hostname; model.ping(); AppNavigation.shared.page = .diagnostics
            }.buttonStyle(.glass).frame(maxWidth: .infinity)
                .disabled(model.sampleMode || model.pingRunning)
                .help(unavailableHelp(model, fallback: model.pingRunning ? "请等待当前 Ping 完成" : "测量此设备"))
        }.font(.system(size: 12)).padding(16).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .background(.background.opacity(0.55), in: .rect(cornerRadius: 16))
    }
    private var path: String {
        if peer.state == .direct { return "直连" }
        if peer.state == .relay { return peerSummary(peer, model: model) }
        return model.path(peer)
    }
}

struct ProfilesView: View {
    let model: AppModel
    let selection: ClientSelection
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("出口方案").font(.system(size: 12, weight: .semibold)).foregroundStyle(.secondary).padding(.horizontal, 10)
            ProfileRows(model: model, selection: selection)
            if selection.profiles.count == 1 && selection.profiles.first?.kind == .mesh { EmptyStateView(kind: .mesh) }
            Spacer()
        }.frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

struct ProfileRows: View {
    let model: AppModel
    let selection: ClientSelection
    var compact = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    private var selectHelp: String {
        selection.locked ? "管理员已锁定方案选择" : (!model.canRequestSelection ? "需要有效身份、netmap 和运行 instance" : "请求 Server 切换方案")
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 1) {
            ForEach(selection.profiles, id: \.name) { profile in
                Button { Task { await model.perform(.selectProfile(profile: profile.name)) } } label: {
                    HStack(spacing: 8) {
                        Image(systemName: "checkmark").font(.system(size: 11, weight: .semibold)).frame(width: 12)
                            .opacity(profile.name == selection.appliedProfile && model.netmapApplied ? 1 : 0)
                        Text(profile.name).lineLimit(1)
                        Spacer(minLength: 4)
                        Text(exitNames(profile))
                            .font(.system(size: 11)).opacity(0.55).lineLimit(1)
                    }
                }.buttonStyle(MenuRowStyle())
                    .disabled(!model.canRequestSelection || selection.locked || model.busy || model.sampleMode)
                    .help(unavailableHelp(model, fallback: selectHelp))
            }
            if !compact || selection.requestedProfile != nil {
                Button { Task { await model.perform(.clearProfile) } } label: {
                    HStack { Text("使用默认方案"); Spacer(); Text(selection.defaultProfile).font(.system(size: 11)).opacity(0.55) }
                }.buttonStyle(MenuRowStyle())
                    .disabled(!model.canRequestSelection || model.busy || model.sampleMode || selection.requestedProfile == nil)
                    .help(unavailableHelp(model, fallback: !model.canRequestSelection ? "需要有效身份、netmap 和运行 instance" : (selection.requestedProfile == nil ? "已跟随默认方案" : "清除持久方案请求")))
            }
            if !model.netmapApplied || model.notice != nil || selection.requestedUnavailable != nil {
                Text(model.notice ?? selection.requestedUnavailable.map(human) ?? "等待本机应用方案")
                    .font(.system(size: 11)).foregroundStyle(.secondary).lineLimit(2).padding(.horizontal, 10)
            }
        }.font(.system(size: 13))
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.18), value: selection.appliedProfile)
    }
    private func exitNames(_ profile: ClientProfile) -> String {
        guard !profile.exits.isEmpty else { return "" }
        let names = profile.exits.compactMap { id in model.status?.peers.first { $0.nodeId == id }?.node.hostname }
        guard names.count == profile.exits.count else { return "出口名称未报告" }
        return "经 " + names.joined(separator: "、")
    }
}
