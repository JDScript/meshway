import SwiftUI

struct MenuPanel: View {
    let model: AppModel
    @Environment(\.openWindow) private var openWindow
    @Environment(\.openSettings) private var openSettings
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.colorScheme) private var colorScheme
    @State private var expandedPeers = false
    private var peers: [ClientPeer] { model.status?.peers ?? [] }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            StatusIsland(model: model).padding(.bottom, 6)
            sectionLabel(String(localized: "Exit"))
            ScrollView {
            if ConnectionPresentation(model: model).state == .connecting { SkeletonRows(count: min(3, max(2, model.selection?.profiles.count ?? 2))) }
            else if let selection = model.selection { ProfileRows(model: model, selection: selection, compact: true) }
            else { EmptyStateView(kind: .profiles, compact: true, height: 96) }
            }.scrollIndicators(.hidden).frame(height: 96, alignment: .top)
            Divider().padding(.horizontal, 10).padding(.vertical, 5)
            sectionLabel(String(localized: "Devices"))
            VStack(alignment: .leading, spacing: 0) {
            if ConnectionPresentation(model: model).state == .connecting { SkeletonRows(count: max(3, min(peers.count, 5))) }
            else if peers.isEmpty {
                EmptyStateView(kind: serverEmptyState(model) ?? .devices, compact: true)
            } else {
                ScrollView {
                    VStack(spacing: 1) {
                        ForEach(expandedPeers ? peers : Array(peers.prefix(5)), id: \.nodeId) { peer in
                            CompactPeerRow(model: model, peer: peer).contextMenu { PeerCopyActions(peer: peer) }.transition(.opacity)
                        }
                    }
                }.scrollIndicators(.hidden)
                if peers.count > 5 {
                    Button(expandedPeers ? String(localized: "Show less") : String(localized: "\(peers.count - 5) more…")) { expandedPeers.toggle() }
                        .buttonStyle(MenuRowStyle()).font(.system(size: 12))
                }
            }
            }.frame(height: expandedPeers ? 330 : 180, alignment: .top)
            Divider().padding(.horizontal, 10).padding(.vertical, 5)
            Button("Open Meshway") { openWindow(id: "main"); NSApp.activate() }.buttonStyle(MenuRowStyle())
            Button { openSettings(); NSApp.activate() } label: {
                HStack { Text("Settings…"); Spacer(); Text(verbatim: "⌘,").opacity(0.5) }
            }.buttonStyle(MenuRowStyle())
            Button("Quit") { NSApp.terminate(nil) }.buttonStyle(MenuRowStyle())
        }.font(.system(size: 13)).padding(10).frame(width: 316)
            .background(colorScheme == .light ? Color.white.opacity(0.94) : Color(nsColor: .windowBackgroundColor).opacity(0.9), in: .rect(cornerRadius: 26))
            .glassEffect(.regular, in: .rect(cornerRadius: 26))
            .overlay(alignment: .bottomTrailing) {
                if model.sampleMode { SampleBanner().padding(5).allowsHitTesting(false) }
            }
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.18), value: peers.map(\.nodeId))
    }
    private func sectionLabel(_ title: String) -> some View {
        Text(title).font(.system(size: 11, weight: .semibold)).foregroundStyle(.tertiary)
            .padding(.horizontal, 10).padding(.top, 4).padding(.bottom, 3)
    }
}
